//! TUI state and input handling. Nothing here performs I/O: anything that
//! touches the disk or runs a tool is a job whose result arrives as an event.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;
use std::sync::mpsc::Sender;

use crossterm::event::{Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::event::Event;
use super::files::{self, FilesState};
use super::jobs::{
    ActionReport, Done, Jobs, KEY_ACTION, KEY_DIFF, KEY_HIDE, KEY_LISTING, KEY_PENDING,
    KEY_PREVIEW, KEY_STATUS, KEY_TRACKED, Payload,
};
use crate::changes::{self, Change, CommitOpts, State};
use crate::hide::HideRules;
use crate::pending::{self, Item, List};
use crate::source::{self, Tracked};
use crate::status::{self, Status};
use crate::store::Store;
use crate::track::{self, ItemResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Status,
    Files,
    Changes,
    Git,
    Machines,
    /// The pending list (bottom right). Not one of the numbered panels: reached
    /// with `0` or by Tab / Shift-Tab.
    Pending,
}

impl Panel {
    /// The numbered panels of the left column.
    pub const ALL: [Panel; 5] = [
        Panel::Status,
        Panel::Files,
        Panel::Changes,
        Panel::Git,
        Panel::Machines,
    ];

    /// Tab order: the numbered panels, then Pending.
    pub const CYCLE: [Panel; 6] = [
        Panel::Status,
        Panel::Files,
        Panel::Changes,
        Panel::Git,
        Panel::Machines,
        Panel::Pending,
    ];

    /// 1-5 for the numbered panels, 0 for Pending.
    pub fn number(self) -> usize {
        Self::ALL
            .iter()
            .position(|p| *p == self)
            .map_or(0, |i| i + 1)
    }

    pub fn title(self) -> &'static str {
        match self {
            Panel::Status => "Status",
            Panel::Files => "Files",
            Panel::Changes => "Changes",
            Panel::Git => "Git",
            Panel::Machines => "Machines",
            Panel::Pending => "Pending",
        }
    }

    fn shifted(self, by: isize) -> Panel {
        let n = Self::CYCLE.len() as isize;
        let at = Self::CYCLE.iter().position(|p| *p == self).unwrap() as isize;
        Self::CYCLE[(at + by).rem_euclid(n) as usize]
    }
}

/// A mutation waiting for its turn on the action queue.
#[derive(Debug, Clone)]
pub enum ActionReq {
    /// Space: flip the tracked state of these HOME paths (absolute).
    Toggle(Vec<std::path::PathBuf>),
    /// `i`: hide (or un-hide) this absolute path, through a pending hide item.
    Hide(std::path::PathBuf, bool),
    Drop(u64),
    Apply(Vec<u64>),
    /// Stage (or, from the Staged tab, unstage) these HOME paths, relative to
    /// HOME; `None` is every change that can be.
    Stage(Option<Vec<String>>),
    Unstage(Option<Vec<String>>),
    /// Commit with this message; `amend` replaces the last commit.
    Commit {
        message: String,
        amend: bool,
    },
    /// Read the last commit message to open the amend box with.
    LoadAmend,
    Discard(Vec<String>),
}

/// The commit message being typed.
#[derive(Debug, Clone)]
pub struct CommitBox {
    pub text: String,
    pub amend: bool,
}

/// Tabs of the Changes panel.
pub const TABS: [&str; 4] = ["Local", "Staged", "Incoming", "Conflicts"];

/// A modal dialog over the main screen.
#[derive(Debug)]
pub enum Dialog {
    /// The one review listing every selected item; Enter or `y` applies.
    Review(Vec<Item>),
    /// Per-item outcome of an apply, partial results included.
    Result(Vec<ItemResult>),
    /// The commit message box: Enter commits, Ctrl+J / Alt+Enter add a line.
    Commit(CommitBox),
    /// `d` in Changes: y/n before overwriting these HOME paths.
    ConfirmDiscard(Vec<String>),
}

#[derive(Debug)]
pub enum Load<T> {
    Loading,
    Ready(T),
    Failed(String),
}

pub struct App {
    pub store: Arc<Store>,
    rules: Arc<HideRules>,
    jobs: Jobs,
    pub focus: Panel,
    pub show_help: bool,
    pub quit: bool,
    pub files: FilesState,
    pub status: Load<Status>,
    pub preview: Load<String>,
    pub size: (u16, u16),
    pub tracked: Arc<Tracked>,
    pub pending: List,
    /// Item ids the user unchecked in the Pending pane (all others are selected).
    pub unchecked: BTreeSet<u64>,
    pub pending_selected: usize,
    /// Anchor row of a `v` range selection in Files.
    pub range_anchor: Option<usize>,
    /// One-line feedback shown in the footer until the next key press.
    pub message: Option<String>,
    pub dialog: Option<Dialog>,
    /// Active tab of the Changes panel (index into [`TABS`]) and the row in it.
    pub changes_tab: usize,
    pub changes_selected: usize,
    /// The diff shown for the selected change.
    pub diff: Load<ratatui::text::Text<'static>>,
    /// Change marker of the `hide` file last loaded (`None` until the first load).
    hide_stamp: Option<crate::hide::HideStamp>,
    actions: VecDeque<ActionReq>,
    action_running: bool,
}

impl App {
    pub fn new(store: Arc<Store>, rules: HideRules, tx: Sender<Event>) -> Self {
        let root = store.dirs.home.clone();
        Self {
            store,
            rules: Arc::new(rules),
            jobs: Jobs::new(tx),
            focus: Panel::Files,
            show_help: false,
            quit: false,
            files: FilesState::new(root),
            status: Load::Loading,
            preview: Load::Ready(String::new()),
            size: (80, 24),
            tracked: Arc::default(),
            pending: List::default(),
            unchecked: BTreeSet::new(),
            pending_selected: 0,
            range_anchor: None,
            message: None,
            dialog: None,
            changes_tab: 0,
            changes_selected: 0,
            diff: Load::Ready(ratatui::text::Text::default()),
            hide_stamp: None,
            actions: VecDeque::new(),
            action_running: false,
        }
    }

    /// Actions queued or running.
    pub fn actions_busy(&self) -> bool {
        self.action_running || !self.actions.is_empty()
    }

    pub fn in_flight_jobs(&self) -> usize {
        self.jobs.in_flight()
    }

    /// Kick off the initial loads. Returns immediately.
    pub fn start(&mut self) {
        self.refresh_status();
        self.refresh_tracking();
        self.refresh_hide();
        self.reload_listing(None);
    }

    /// Re-read the hide rules; the result replaces them only when the file changed.
    fn refresh_hide(&mut self) {
        let store = Arc::clone(&self.store);
        self.jobs.spawn(KEY_HIDE, move |_| {
            let stamp = crate::hide::stamp(&store.repo());
            Payload::Hide(
                HideRules::for_store(&store)
                    .map(|r| (Box::new(r), stamp))
                    .map_err(|e| format!("{e:#}")),
            )
        });
    }

    /// Reload what the repository and the pending list say about tracking.
    fn refresh_tracking(&mut self) {
        let repo = self.store.repo();
        self.jobs.spawn(KEY_TRACKED, move |_| {
            Payload::Tracked(
                source::load(&repo)
                    .map(Box::new)
                    .map_err(|e| format!("{e:#}")),
            )
        });
        let store = Arc::clone(&self.store);
        self.jobs.spawn(KEY_PENDING, move |_| {
            Payload::Pending(
                pending::load(&store)
                    .map(Box::new)
                    .map_err(|e| format!("{e:#}")),
            )
        });
    }

    fn request_action(&mut self, req: ActionReq) {
        self.actions.push_back(req);
        self.pump_actions();
    }

    /// Start the next queued action unless one is running, so they run in order.
    fn pump_actions(&mut self) {
        if self.action_running {
            return;
        }
        let Some(req) = self.actions.pop_front() else {
            return;
        };
        self.action_running = true;
        self.message = Some("working…".into());
        let store = Arc::clone(&self.store);
        self.jobs.spawn_detached(KEY_ACTION, move |_| {
            Payload::Action(run_action(&store, req).map_err(|e| {
                if crate::lock::is_busy(&e) {
                    "Busy: another dotwarden command holds the lock; nothing was changed, try again"
                        .to_string()
                } else {
                    format!("{e:#}")
                }
            }))
        });
    }

    fn refresh_status(&mut self) {
        let store = Arc::clone(&self.store);
        self.jobs.spawn(KEY_STATUS, move |cancel| {
            Payload::Status(
                status::collect(&store, Some(cancel))
                    .map(Box::new)
                    .map_err(|e| format!("{e:#}")),
            )
        });
    }

    fn reload_listing(&mut self, reselect: Option<std::ffi::OsString>) {
        self.files.begin_load(reselect);
        self.preview = Load::Ready(String::new());
        let dir = self.files.cwd.clone();
        let rules = Arc::clone(&self.rules);
        self.jobs.spawn(KEY_LISTING, move |cancel| {
            let entries = files::list_dir(&dir, &rules, cancel);
            Payload::Listing { dir, entries }
        });
    }

    fn request_preview(&mut self) {
        let Some(path) = self.files.selected_path() else {
            self.preview = Load::Ready(String::new());
            return;
        };
        self.preview = Load::Loading;
        self.jobs.spawn(KEY_PREVIEW, move |cancel| {
            let text = files::preview(&path, cancel);
            Payload::Preview { path, text }
        });
    }

    pub fn handle_event(&mut self, ev: Event) {
        match ev {
            Event::Input(TermEvent::Key(key)) if key.kind != KeyEventKind::Release => {
                self.handle_key(key)
            }
            Event::Input(TermEvent::Resize(w, h)) => self.size = (w, h),
            Event::Input(_) => {}
            Event::Job(done) => self.handle_job(done),
            Event::Tick => {
                self.refresh_status();
                self.refresh_tracking();
                self.refresh_hide();
            }
        }
    }

    fn handle_job(&mut self, done: Done) {
        if !self.jobs.is_current(done.key, done.id) {
            return;
        }
        match done.payload {
            Payload::Status(r) => {
                self.status = match r {
                    Ok(s) => Load::Ready(*s),
                    Err(e) => Load::Failed(e),
                };
                self.changes_selected = self
                    .changes_selected
                    .min(self.change_rows().len().saturating_sub(1));
                if self.focus == Panel::Changes {
                    self.request_diff();
                }
            }
            Payload::Diff(r) => {
                self.diff = match r {
                    Ok(t) => Load::Ready(t),
                    Err(e) => Load::Failed(e),
                }
            }
            Payload::Listing { dir, entries } => {
                if dir == self.files.cwd {
                    self.files.set_listing(entries);
                    self.request_preview();
                }
            }
            Payload::Hide(r) => match r {
                Ok((rules, stamp)) => {
                    let changed = self.hide_stamp.is_some_and(|old| old != stamp);
                    self.hide_stamp = Some(stamp);
                    if changed {
                        self.rules = Arc::new(*rules);
                        let keep = self.files.selected_entry().map(|e| e.name.clone());
                        self.reload_listing(keep);
                    }
                }
                Err(e) => self.message = Some(e),
            },
            Payload::Tracked(r) => {
                if let Ok(t) = r {
                    self.tracked = Arc::new(*t);
                }
            }
            Payload::Pending(r) => match r {
                Ok(list) => self.set_pending(*list),
                Err(e) => self.message = Some(e),
            },
            Payload::Action(r) => {
                self.action_running = false;
                match r {
                    Ok(report) => {
                        self.message = Some(report.message);
                        if let Some(text) = report.amend_message {
                            self.dialog = Some(Dialog::Commit(CommitBox { text, amend: true }));
                        }
                        if let Some(results) = report.results {
                            self.dialog = Some(Dialog::Result(results));
                        }
                    }
                    Err(e) => self.message = Some(e),
                }
                self.refresh_tracking();
                self.refresh_hide();
                self.refresh_status();
                self.pump_actions();
            }
            Payload::Preview { path, text } => {
                if self.files.selected_path().as_deref() == Some(path.as_path()) {
                    self.preview = match text {
                        Ok(t) => Load::Ready(t),
                        Err(e) => Load::Failed(e),
                    };
                }
            }
        }
    }

    fn set_pending(&mut self, list: List) {
        let ids: BTreeSet<u64> = list.items.iter().filter_map(|i| i.id).collect();
        self.unchecked.retain(|i| ids.contains(i));
        self.pending = list;
        self.pending_selected = self
            .pending_selected
            .min(self.pending.items.len().saturating_sub(1));
    }

    pub fn selected_item(&self) -> Option<&Item> {
        self.pending.items.get(self.pending_selected)
    }

    /// Pending items that are checked, in id order.
    pub fn checked_items(&self) -> Vec<Item> {
        self.pending
            .items
            .iter()
            .filter(|i| i.id.is_some_and(|id| !self.unchecked.contains(&id)))
            .cloned()
            .collect()
    }

    /// Rows `start..=end` of the visible Files list covered by the `v` range.
    pub fn range_rows(&self) -> Option<(usize, usize)> {
        let a = self.range_anchor?;
        let b = self.files.selected;
        Some((a.min(b), a.max(b)))
    }

    pub fn pending_marks(&self) -> BTreeMap<String, &'static str> {
        pending::marks(&self.pending)
    }

    fn dialog_key(&mut self, key: KeyEvent) {
        match self.dialog.take() {
            Some(Dialog::Review(items)) => match key.code {
                KeyCode::Enter | KeyCode::Char('y') => {
                    let ids = items.iter().filter_map(|i| i.id).collect();
                    self.request_action(ActionReq::Apply(ids));
                }
                KeyCode::Esc | KeyCode::Char('n') => {}
                _ => self.dialog = Some(Dialog::Review(items)),
            },
            Some(Dialog::Commit(mut b)) => {
                let m = key.modifiers;
                match key.code {
                    KeyCode::Esc => return,
                    KeyCode::Enter if m.intersects(KeyModifiers::ALT | KeyModifiers::SHIFT) => {
                        b.text.push('\n')
                    }
                    KeyCode::Char('j') if m.contains(KeyModifiers::CONTROL) => b.text.push('\n'),
                    KeyCode::Enter if b.text.trim().is_empty() => {
                        self.message = Some("the commit message is empty".into())
                    }
                    KeyCode::Enter => {
                        return self.request_action(ActionReq::Commit {
                            message: b.text.trim().to_string(),
                            amend: b.amend,
                        });
                    }
                    KeyCode::Backspace => {
                        b.text.pop();
                    }
                    KeyCode::Char(c)
                        if !m.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        b.text.push(c)
                    }
                    _ => {}
                }
                self.dialog = Some(Dialog::Commit(b));
            }
            Some(Dialog::ConfirmDiscard(paths)) => match key.code {
                KeyCode::Char('y') => self.request_action(ActionReq::Discard(paths)),
                KeyCode::Esc | KeyCode::Char('n') => {}
                _ => self.dialog = Some(Dialog::ConfirmDiscard(paths)),
            },
            Some(Dialog::Result(_)) | None => {}
        }
    }

    /// Changes shown in the active tab.
    pub fn change_rows(&self) -> Vec<&Change> {
        let Load::Ready(s) = &self.status else {
            return Vec::new();
        };
        s.changes
            .iter()
            .filter(|c| tab_of(c.state) == self.changes_tab)
            .collect()
    }

    /// Number of changes per tab.
    pub fn change_counts(&self) -> [usize; 4] {
        let mut n = [0; 4];
        if let Load::Ready(s) = &self.status {
            for c in &s.changes {
                n[tab_of(c.state)] += 1;
            }
        }
        n
    }

    pub fn selected_change(&self) -> Option<&Change> {
        self.change_rows().get(self.changes_selected).copied()
    }

    fn request_diff(&mut self) {
        let Some(c) = self.selected_change().cloned() else {
            self.diff = Load::Ready(ratatui::text::Text::default());
            return;
        };
        let store = Arc::clone(&self.store);
        self.jobs.spawn(KEY_DIFF, move |cancel| {
            Payload::Diff(render_diff(&store, &c, cancel).map_err(|e| format!("{e:#}")))
        });
    }

    fn changes_key(&mut self, key: KeyEvent) {
        let rows = self.change_rows().len();
        let before = (self.changes_tab, self.changes_selected);
        let tab = self.changes_tab;
        let selected = self.selected_change().cloned();
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.changes_selected = (self.changes_selected + 1).min(rows.saturating_sub(1));
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.changes_selected = self.changes_selected.saturating_sub(1);
            }
            KeyCode::Char('[') => {
                self.changes_tab = (tab + TABS.len() - 1) % TABS.len();
                self.changes_selected = 0;
            }
            KeyCode::Char(']') => {
                self.changes_tab = (tab + 1) % TABS.len();
                self.changes_selected = 0;
            }
            KeyCode::Char('h') | KeyCode::Left => self.focus = self.focus.shifted(-1),
            KeyCode::Char('l') | KeyCode::Right => self.focus = self.focus.shifted(1),
            KeyCode::Enter => self.request_diff(),
            KeyCode::Char(' ') => match (tab, selected) {
                (0, Some(c)) => self.request_action(ActionReq::Stage(Some(vec![c.path]))),
                (1, Some(c)) if c.kind == "repo" => {
                    self.message = Some("repository files are unstaged together: press a".into())
                }
                (1, Some(c)) => self.request_action(ActionReq::Unstage(Some(vec![c.path]))),
                (2 | 3, Some(_)) => self.message = Some(READ_ONLY.into()),
                _ => {}
            },
            KeyCode::Char('a') => match tab {
                0 => self.request_action(ActionReq::Stage(None)),
                1 => self.request_action(ActionReq::Unstage(None)),
                _ => self.message = Some(READ_ONLY.into()),
            },
            KeyCode::Char('c') => {
                if self.change_counts()[1] == 0 {
                    self.message = Some("nothing is staged".into());
                } else {
                    self.dialog = Some(Dialog::Commit(CommitBox {
                        text: String::new(),
                        amend: false,
                    }));
                }
            }
            KeyCode::Char('A') => self.request_action(ActionReq::LoadAmend),
            KeyCode::Char('d') => match selected {
                Some(c) if c.state == State::Local => {
                    self.dialog = Some(Dialog::ConfirmDiscard(vec![c.path]))
                }
                Some(c) if c.state == State::New => {
                    self.message = Some("a new file has no committed version to restore".into())
                }
                Some(_) => self.message = Some("only local changes can be discarded".into()),
                None => {}
            },
            _ => {}
        }
        if (self.changes_tab, self.changes_selected) != before {
            self.request_diff();
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        let before = self.focus;
        self.handle_key_inner(key);
        if self.focus == Panel::Changes && before != Panel::Changes {
            self.request_diff();
        }
    }

    fn handle_key_inner(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        self.message = None;
        if self.dialog.is_some() {
            return match key.code {
                KeyCode::Char('c') if ctrl => self.quit = true,
                _ => self.dialog_key(key),
            };
        }
        match key.code {
            KeyCode::Char('c') if ctrl => return self.quit = true,
            KeyCode::Char('q') => return self.quit = true,
            KeyCode::Char('?') => return self.show_help = !self.show_help,
            KeyCode::Esc if self.show_help => return self.show_help = false,
            _ if self.show_help => return,
            KeyCode::Tab => return self.focus = self.focus.shifted(1),
            KeyCode::BackTab => return self.focus = self.focus.shifted(-1),
            KeyCode::Char(c @ '1'..='5') => {
                return self.focus = Panel::ALL[c as usize - '1' as usize];
            }
            KeyCode::Char('0') => return self.focus = Panel::Pending,
            KeyCode::Char('r') => {
                self.refresh_tracking();
                self.refresh_hide();
                self.refresh_status();
                return self.reload_listing(None);
            }
            _ => {}
        }
        match self.focus {
            Panel::Files => self.files_key(key),
            Panel::Pending => self.pending_key(key),
            Panel::Changes => self.changes_key(key),
            _ => match key.code {
                KeyCode::Char('h') | KeyCode::Left => self.focus = self.focus.shifted(-1),
                KeyCode::Char('l') | KeyCode::Right => self.focus = self.focus.shifted(1),
                _ => {}
            },
        }
    }

    fn pending_key(&mut self, key: KeyEvent) {
        let n = self.pending.items.len();
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.pending_selected = (self.pending_selected + 1).min(n.saturating_sub(1));
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.pending_selected = self.pending_selected.saturating_sub(1);
            }
            KeyCode::Char('h') | KeyCode::Left => self.focus = self.focus.shifted(-1),
            KeyCode::Char('l') | KeyCode::Right => self.focus = self.focus.shifted(1),
            KeyCode::Char(' ') => {
                if let Some(id) = self.selected_item().and_then(|i| i.id)
                    && !self.unchecked.remove(&id)
                {
                    self.unchecked.insert(id);
                }
            }
            KeyCode::Char('d') => {
                if let Some(id) = self.selected_item().and_then(|i| i.id) {
                    self.request_action(ActionReq::Drop(id));
                }
            }
            KeyCode::Enter => {
                let items = self.checked_items();
                if items.is_empty() {
                    self.message = Some("no pending items selected".into());
                } else {
                    self.dialog = Some(Dialog::Review(items));
                }
            }
            _ => {}
        }
    }

    /// Space in Files: flip tracking of the selected entry, or of the `v` range.
    fn toggle_selection(&mut self) {
        let visible = self.files.visible();
        let (lo, hi) = self
            .range_rows()
            .unwrap_or((self.files.selected, self.files.selected));
        let paths: Vec<_> = (lo..=hi)
            .filter_map(|i| visible.get(i))
            .map(|e| self.files.cwd.join(&e.name))
            .collect();
        self.range_anchor = None;
        if !paths.is_empty() {
            self.request_action(ActionReq::Toggle(paths));
        }
    }

    fn files_key(&mut self, key: KeyEvent) {
        let before = self.files.selected_path();
        match key.code {
            KeyCode::Char(' ') => return self.toggle_selection(),
            KeyCode::Char('i') => {
                if let Some(e) = self.files.selected_entry() {
                    let (path, is_dir) = (self.files.cwd.join(&e.name), e.is_dir);
                    self.request_action(ActionReq::Hide(path, is_dir));
                }
                return;
            }
            KeyCode::Char('v') => {
                self.range_anchor = match self.range_anchor {
                    Some(_) => None,
                    None => Some(self.files.selected),
                };
                return;
            }
            KeyCode::Esc if self.range_anchor.is_some() => return self.range_anchor = None,
            KeyCode::Char('j') | KeyCode::Down => self.files.move_by(1),
            KeyCode::Char('k') | KeyCode::Up => self.files.move_by(-1),
            KeyCode::PageDown => self.files.move_by(10),
            KeyCode::PageUp => self.files.move_by(-10),
            // lazygit's top/bottom keys; `G` is reserved for the lazygit handoff.
            KeyCode::Char('<') | KeyCode::Home => self.files.select_first(),
            KeyCode::Char('>') | KeyCode::End => self.files.select_last(),
            KeyCode::Char('.') => self.files.toggle_hidden(),
            KeyCode::Char('l') | KeyCode::Right | KeyCode::Enter => {
                if self.files.enter() {
                    self.range_anchor = None;
                    return self.reload_listing(None);
                }
            }
            KeyCode::Char('h') | KeyCode::Left | KeyCode::Backspace => {
                let left = self.files.cwd.file_name().map(std::ffi::OsString::from);
                if self.files.leave() {
                    self.range_anchor = None;
                    return self.reload_listing(left);
                }
            }
            _ => {}
        }
        if self.files.selected_path() != before {
            self.request_preview();
        }
    }
}

fn report(message: String) -> ActionReport {
    ActionReport {
        message,
        results: None,
        amend_message: None,
    }
}

fn done_message(verb: &str, d: &changes::Done) -> String {
    let mut m = match d.paths.as_slice() {
        [one] => format!("{verb} {one}"),
        many => format!("{verb} {} files", many.len()),
    };
    if !d.skipped.is_empty() {
        m.push_str(&format!(" ({} skipped)", d.skipped.len()));
    }
    m
}

const READ_ONLY: &str = "incoming changes and conflicts are read-only for now";

fn tab_of(s: State) -> usize {
    match s {
        State::Local | State::New => 0,
        State::Staged => 1,
        State::Incoming => 2,
        State::Conflict => 3,
    }
}

/// The selected change as styled text: delta's output when it works, else the
/// plain unified diff.
fn render_diff(
    store: &Store,
    c: &Change,
    cancel: &crate::exec::CancelToken,
) -> anyhow::Result<ratatui::text::Text<'static>> {
    use ansi_to_tui::IntoText;
    if matches!(c.state, State::Incoming | State::Conflict) {
        return Ok(format!(
            "~/{} has an {} change.\n\nIt is shown read-only for now; applying and resolving come later.",
            c.path,
            c.state.label()
        )
        .into());
    }
    let sel = (c.kind != "repo").then(|| vec![c.path.clone()]);
    let label = if c.kind == "repo" {
        format!("repo:{}", c.path)
    } else {
        track::tilde(&c.path)
    };
    let plain: String = changes::diff(store, sel.as_deref())?
        .into_iter()
        .filter(|d| d.state == c.state && d.path == label)
        .map(|d| d.diff)
        .collect();
    if plain.is_empty() {
        return Ok("(no textual difference: only the mode or type changed)"
            .to_string()
            .into());
    }
    match changes::through_delta(store, &plain, Some(cancel)) {
        Some(styled) => styled.into_text().or_else(|_| Ok(plain.clone().into())),
        None => Ok(plain.into()),
    }
}

/// Run one queued mutation on a worker thread.
fn run_action(store: &Store, req: ActionReq) -> anyhow::Result<ActionReport> {
    match req {
        ActionReq::Toggle(paths) => {
            let requester = "human";
            let mut messages = Vec::new();
            let mut refused = Vec::new();
            for p in &paths {
                let result =
                    track::resolve_home_path(store, &p.to_string_lossy(), &store.dirs.home)
                        .and_then(|hp| track::toggle(store, &hp, requester));
                match result {
                    Ok(t) => messages.push(t.message),
                    Err(e) if crate::lock::is_busy(&e) => return Err(e),
                    Err(e) => refused.push(format!("{e:#}")),
                }
            }
            let message = match (messages.len(), refused.len()) {
                (_, 0) if messages.len() == 1 => messages.remove(0),
                (n, 0) => format!("{n} pending changes updated"),
                (0, _) => return Err(anyhow::anyhow!(refused.join("; "))),
                (n, m) => format!("{n} pending changes updated, {m} refused: {}", refused[0]),
            };
            Ok(ActionReport {
                message,
                results: None,
                amend_message: None,
            })
        }
        ActionReq::Hide(path, is_dir) => {
            let hp = track::resolve_home_path(store, &path.to_string_lossy(), &store.dirs.home)?;
            let t = track::toggle_hide(store, &hp, is_dir, "human")?;
            Ok(ActionReport {
                message: t.message,
                results: None,
                amend_message: None,
            })
        }
        ActionReq::Drop(id) => {
            let lock = store.lock()?;
            let gone = pending::drop_items(store, &lock, &[id])?;
            Ok(ActionReport {
                message: format!("Dropped {}: {}", gone[0].label(), gone[0].summary),
                results: None,
                amend_message: None,
            })
        }
        ActionReq::Stage(sel) => {
            let d = changes::stage(store, sel.as_deref(), false)?;
            Ok(report(done_message("Staged", &d)))
        }
        ActionReq::Unstage(sel) => {
            let d = changes::unstage(store, sel.as_deref(), false)?;
            Ok(report(done_message("Unstaged", &d)))
        }
        ActionReq::Commit { message, amend } => {
            let c = changes::commit(
                store,
                &CommitOpts {
                    messages: vec![message],
                    amend,
                    dry_run: false,
                    interactive: false,
                },
            )?;
            Ok(report(format!(
                "{} {} ({} file(s))",
                if amend { "Amended" } else { "Committed" },
                c.commit.unwrap_or_default(),
                c.files
            )))
        }
        ActionReq::LoadAmend => {
            changes::ensure_amendable(store)?;
            let mut r = report(String::new());
            r.amend_message = Some(changes::last_message(store)?);
            Ok(r)
        }
        ActionReq::Discard(paths) => {
            let mut items = Vec::new();
            for p in &paths {
                items.push(changes::plan_discard(store, p, "human")?);
            }
            let results = track::apply_now(store, &items)?;
            if let Some(bad) = results.iter().find(|r| r.status != track::Outcome::Applied) {
                anyhow::bail!(
                    bad.message
                        .clone()
                        .unwrap_or_else(|| "discard failed".into())
                );
            }
            let note = results[0].message.clone().unwrap_or_default();
            Ok(report(format!(
                "Discarded {} ({note})",
                if paths.len() == 1 {
                    track::tilde(&paths[0])
                } else {
                    format!("{} items", paths.len())
                }
            )))
        }
        ActionReq::Apply(ids) => {
            let results = track::apply_pending(store, Some(&ids))?;
            let applied = results
                .iter()
                .filter(|r| r.status == track::Outcome::Applied)
                .count();
            Ok(ActionReport {
                message: format!("Applied {applied} of {} item(s)", results.len()),
                results: Some(results),
                amend_message: None,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::Dirs;
    use crate::toolchain::Toolchain;
    use crossterm::event::KeyEventState;
    use std::sync::mpsc::{Receiver, channel};
    use std::time::Duration;

    pub fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn app() -> (App, Receiver<Event>, tempfile::TempDir) {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir(home.path().join("dir")).unwrap();
        std::fs::write(home.path().join(".zshrc"), "hi").unwrap();
        let env = [("HOME", home.path().to_str().unwrap())]
            .iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        let dirs = Dirs::from_env(&env).unwrap();
        let store = Arc::new(Store::new(dirs, Toolchain::at("/nonexistent"), env));
        let rules = HideRules::defaults(home.path()).unwrap();
        let (tx, rx) = channel();
        (App::new(store, rules, tx), rx, home)
    }

    /// Handle events until the folder listing has loaded.
    fn pump(app: &mut App, rx: &Receiver<Event>) {
        while app.files.loading {
            let ev = rx.recv_timeout(Duration::from_secs(5)).unwrap();
            app.handle_event(ev);
        }
    }

    #[test]
    fn panel_focus_keys() {
        let (mut a, _rx, _h) = app();
        a.handle_key(key(KeyCode::Char('4')));
        assert_eq!(a.focus, Panel::Git);
        a.handle_key(key(KeyCode::Tab));
        assert_eq!(a.focus, Panel::Machines);
        a.handle_key(key(KeyCode::Tab));
        assert_eq!(a.focus, Panel::Pending);
        a.handle_key(key(KeyCode::Tab));
        assert_eq!(a.focus, Panel::Status);
        a.handle_key(key(KeyCode::BackTab));
        assert_eq!(a.focus, Panel::Pending);
        a.handle_key(key(KeyCode::Char('0')));
        assert_eq!(a.focus, Panel::Pending);
        a.handle_key(key(KeyCode::BackTab));
        assert_eq!(a.focus, Panel::Machines);
        a.handle_key(key(KeyCode::Char('h')));
        assert_eq!(a.focus, Panel::Git);
    }

    #[test]
    fn help_overlay_swallows_keys_until_closed() {
        let (mut a, _rx, _h) = app();
        a.handle_key(key(KeyCode::Char('?')));
        assert!(a.show_help);
        a.handle_key(key(KeyCode::Char('3')));
        assert_eq!(a.focus, Panel::Files);
        a.handle_key(key(KeyCode::Esc));
        assert!(!a.show_help);
        a.handle_key(key(KeyCode::Char('q')));
        assert!(a.quit);
    }

    #[test]
    fn browse_into_a_folder_and_back() {
        let (mut a, rx, home) = app();
        a.start();
        pump(&mut a, &rx);
        assert_eq!(a.files.selected_entry().unwrap().display(), "dir");
        a.handle_key(key(KeyCode::Char('l')));
        pump(&mut a, &rx);
        assert_eq!(a.files.cwd, home.path().join("dir"));
        a.handle_key(key(KeyCode::Char('h')));
        pump(&mut a, &rx);
        assert_eq!(a.files.cwd, home.path());
        assert_eq!(a.files.selected_entry().unwrap().display(), "dir");
    }

    #[test]
    fn stale_listing_is_ignored() {
        let (mut a, rx, home) = app();
        a.start();
        pump(&mut a, &rx);
        a.handle_key(key(KeyCode::Char('l'))); // enter dir
        // A stale result for the previous directory must not replace the view.
        a.handle_event(Event::Job(Done {
            id: 1,
            key: KEY_LISTING,
            payload: Payload::Listing {
                dir: home.path().to_path_buf(),
                entries: Ok(vec![]),
            },
        }));
        assert!(a.files.loading);
    }

    #[test]
    fn resize_is_recorded() {
        let (mut a, _rx, _h) = app();
        a.handle_event(Event::Input(TermEvent::Resize(45, 14)));
        assert_eq!(a.size, (45, 14));
    }
}
