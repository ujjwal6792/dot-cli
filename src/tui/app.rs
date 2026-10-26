//! TUI state and input handling. Nothing here performs I/O: anything that
//! touches the disk or runs a tool is a job whose result arrives as an event.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;
use std::sync::mpsc::Sender;

use crossterm::event::{Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::event::Event;
use super::files::{self, FilesState};
use super::jobs::{
    ActionReport, Done, Jobs, KEY_ACTION, KEY_LISTING, KEY_PENDING, KEY_PREVIEW, KEY_STATUS,
    KEY_TRACKED, Payload,
};
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
    Drop(u64),
    Apply(Vec<u64>),
}

/// A modal dialog over the main screen.
#[derive(Debug)]
pub enum Dialog {
    /// The one review listing every selected item; Enter or `y` applies.
    Review(Vec<Item>),
    /// Per-item outcome of an apply, partial results included.
    Result(Vec<ItemResult>),
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
        self.reload_listing(None);
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
            Payload::Action(run_action(&store, req).map_err(|e| format!("{e:#}")))
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
                }
            }
            Payload::Listing { dir, entries } => {
                if dir == self.files.cwd {
                    self.files.set_listing(entries);
                    self.request_preview();
                }
            }
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
                        if let Some(results) = report.results {
                            self.dialog = Some(Dialog::Result(results));
                        }
                    }
                    Err(e) => self.message = Some(e),
                }
                self.refresh_tracking();
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
            Some(Dialog::Result(_)) | None => {}
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
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
                self.refresh_status();
                return self.reload_listing(None);
            }
            _ => {}
        }
        match self.focus {
            Panel::Files => self.files_key(key),
            Panel::Pending => self.pending_key(key),
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

/// Run one queued mutation on a worker thread.
fn run_action(store: &Store, req: ActionReq) -> anyhow::Result<ActionReport> {
    match req {
        ActionReq::Toggle(paths) => {
            let lock = store.lock()?;
            let requester = "human";
            let mut messages = Vec::new();
            let mut refused = Vec::new();
            for p in &paths {
                let result =
                    track::resolve_home_path(store, &p.to_string_lossy(), &store.dirs.home)
                        .and_then(|hp| track::toggle(store, &lock, &hp, requester));
                match result {
                    Ok(t) => messages.push(t.message),
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
            })
        }
        ActionReq::Drop(id) => {
            let lock = store.lock()?;
            let gone = pending::drop_items(store, &lock, &[id])?;
            Ok(ActionReport {
                message: format!("Dropped {}: {}", gone[0].label(), gone[0].summary),
                results: None,
            })
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
