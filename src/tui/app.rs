//! TUI state and input handling. Nothing here performs I/O: anything that
//! touches the disk or runs a tool is a job whose result arrives as an event.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use crossterm::event::{
    Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind,
};

use super::event::Event;
use super::files::{self, FilesState};
use super::jobs::{
    ActionReport, Done, Jobs, KEY_ACTION, KEY_CONFLICT, KEY_DIFF, KEY_FETCH, KEY_HIDE, KEY_LISTING,
    KEY_MACHINE, KEY_PENDING, KEY_PREVIEW, KEY_REGISTRY, KEY_STAGING, KEY_STATUS, KEY_TRACKED,
    KEY_VALUES, Payload, RegistryView, ShownHunk, ValuesView,
};
use crate::assign::{self, Side, TargetRow, Tri};
use crate::changes::{self, Change, CommitOpts, State};
use crate::conflicts::Pick;
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

/// A visible entry of the folder a bulk selection acts on.
#[derive(Debug, Clone)]
pub struct BulkEntry {
    pub path: std::path::PathBuf,
    pub rel: String,
    pub is_dir: bool,
}

/// A mutation waiting for its turn on the action queue.
#[derive(Debug, Clone)]
pub enum ActionReq {
    Git(GitOp, String),
    /// Space: flip the tracked state of these HOME paths (absolute).
    Toggle(Vec<std::path::PathBuf>),
    /// `a` / `A` in Files. Decided when it runs, from the pending list and the
    /// tracked set as they are then, so presses queued back to back each see
    /// the result of the one before.
    Bulk {
        entries: Vec<BulkEntry>,
        drop_only: bool,
        refused: Arc<Mutex<BTreeSet<std::path::PathBuf>>>,
    },
    /// `i`: hide (or un-hide) this absolute path, through a pending hide item.
    Hide(std::path::PathBuf, bool),
    Drop(u64),
    Apply(Vec<u64>),
    /// Stage (or, from the Staged tab, unstage) these HOME paths, relative to
    /// HOME; `None` is every change that can be.
    Stage(Option<Vec<String>>),
    Unstage(Option<Vec<String>>),
    /// Stage or unstage selected hunks and lines of one file, as shown.
    Hunks {
        path: String,
        side: crate::hunks::Side,
        sel: crate::hunks::Selection,
        expect: String,
    },
    /// Commit with this message; `amend` replaces the last commit.
    Commit {
        message: String,
        amend: bool,
    },
    /// Read the last commit message to open the amend box with.
    LoadAmend,
    Discard(Vec<String>),
    /// Write incoming files to HOME: these paths, or all when `None`.
    ApplyIncoming(Option<Vec<String>>),
    Fetch,
    /// `z`: take back the last local action.
    Undo,
    Pull,
    MergePull,
    ContinuePull,
    AbortPull,
    Revert(String),
    Push,
    /// Add one pending assignment item for these HOME-relative paths.
    Assign(Vec<String>, Vec<crate::machines::Edit>),
    /// Add a pending variant of this HOME-relative file for a target.
    Variant(String, String),
    /// Set a variable's default (no target) or one override, as a pending item.
    Value {
        name: String,
        target: Option<String>,
        value: String,
    },
    /// `E` in Files: add a pending item storing this HOME-relative file encrypted.
    Encrypt(String),
    /// Set a secret's default or one override in the vault (immediately).
    SecretSet {
        name: String,
        target: Option<String>,
        value: Secret,
    },
    Unlock(Secret),
    Lock,
    /// Write this finished text over the conflicting file (HOME-relative) and stage it.
    Resolve(String, Secret),
}

#[derive(Debug, Clone)]
pub enum GitOp {
    Create(String),
    Switch(String),
    Delete(String, bool),
    StashPush,
    StashPop(String),
    StashDrop(String),
    Restore(String),
    RemoteAdd(String, String),
    RemoteRemove(String),
}

/// Tabs of the Git panel.
pub const GIT_TABS: [&str; 4] = ["Commits", "Branches", "Remotes", "Stash"];

fn preview_git(store: &Store, op: &GitOp) -> anyhow::Result<String> {
    use crate::remote::Host;
    let agent = changes::agent(store).is_some();
    Ok(match op {
        GitOp::RemoteAdd(name, url) => {
            crate::remote::plan_add(store, name, url)?;
            let check = match crate::remote::host_of(url) {
                Host::Local => "A local path: never public.".to_string(),
                Host::Known(_) => crate::remote::ensure_not_public(store, url, None)?
                    .unwrap_or_else(|| "Not public. Checked again before every push.".into()),
                Host::Unknown(_) if crate::remote::confirmed_private(store)?.contains(url) => {
                    "Already confirmed private.".into()
                }
                Host::Unknown(host) => format!(
                    "{host} cannot be checked for visibility.\ny confirms that {url} is private (stored in machine.toml)."
                ),
            };
            format!(
                "Add remote {name}\nURL: {url}\n{check}{}",
                if agent {
                    "\nQueued for human approval."
                } else {
                    ""
                }
            )
        }
        GitOp::RemoteRemove(name) => {
            let url = crate::remote::remote_url(store, name)?
                .ok_or_else(|| anyhow::anyhow!("no remote named {name}"))?;
            format!(
                "Remove remote {name}\nURL: {url}\nThe remote repository is not touched.{}",
                if agent {
                    "\nQueued for human approval."
                } else {
                    ""
                }
            )
        }
        GitOp::Create(n) | GitOp::Switch(n) => {
            let files = crate::branches::change(store, n, matches!(op, GitOp::Create(_)), true)?;
            format!(
                "{op:?}\nSource/index: {}\nHOME unchanged.{}",
                files.join(", "),
                if *n != crate::branches::default_name(store) {
                    " Non-default branch."
                } else {
                    ""
                }
            )
        }
        GitOp::Delete(n, f) => crate::branches::plan_delete(store, n, *f)?.render(),
        GitOp::StashDrop(id) => crate::stashes::plan_drop(store, Some(id))?.render(),
        GitOp::StashPush | GitOp::StashPop(_) => {
            let s = if let GitOp::StashPop(id) = op {
                crate::stashes::pop(store, Some(id), true)?
            } else {
                crate::stashes::push(store, true)?
            };
            format!(
                "{op:?}\nHOME: {}\nSource index: {}\nSnapshot before overwrite; untracked HOME excluded.",
                s.paths.join(", "),
                s.index_paths.join(", ")
            )
        }
        GitOp::Restore(hash) => {
            let cs = crate::restores::plan(store, hash, None, true)?;
            format!(
                "Review restore from {hash}\n{}\nHOME and HEAD unchanged until review/apply.",
                cs.iter()
                    .map(|c| format!(
                        "~/{}{}",
                        c.path,
                        if c.conflict { " (Conflict)" } else { "" }
                    ))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        }
    })
}

/// Typed-in secret text; never printed by `Debug`.
#[derive(Clone)]
pub struct Secret(pub String);

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(…)")
    }
}

/// What the Values input box is editing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueEdit {
    /// The default of this variable.
    Default(String),
    /// "TARGET VALUE" for an override of this variable.
    Override(String),
    /// "NAME VALUE" for a new variable.
    New,
    /// The default of a secret (typed text is masked).
    SecretDefault(String),
    /// "TARGET VALUE" for an override of a secret (the value is masked).
    SecretOverride(String),
    /// "NAME VALUE" for a new secret (the value is masked).
    NewSecret,
    /// The vault passphrase (masked).
    Unlock,
}

impl ValueEdit {
    /// What the input box shows for `text`: secret parts as bullets.
    pub fn shown(&self, text: &str) -> String {
        let dots = |s: &str| "•".repeat(s.chars().count());
        match self {
            ValueEdit::SecretDefault(_) | ValueEdit::Unlock => dots(text),
            ValueEdit::SecretOverride(_) | ValueEdit::NewSecret => match text.split_once(' ') {
                Some((a, b)) => format!("{a} {}", dots(b)),
                None => text.to_string(),
            },
            _ => text.to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ValueInput {
    pub edit: ValueEdit,
    pub text: String,
}

/// The Values screen (`V`).
#[derive(Debug, Default)]
pub struct ValuesScreen {
    pub rows: Vec<crate::values::Row>,
    /// Decrypted secrets, only while unlocked.
    pub reveal: std::collections::BTreeMap<String, crate::values::Reveal>,
    pub selected: usize,
    pub input: Option<ValueInput>,
}

/// The Conflicts screen (Enter on the Conflicts tab): hunks of one file.
#[derive(Debug)]
pub struct ConflictScreen {
    pub stored: bool,
    pub path: String,
    pub data: Load<Box<super::jobs::ConflictData>>,
    /// The conflicting hunk in view.
    pub cursor: usize,
    /// The choice per conflicting hunk.
    pub picks: Vec<Option<Pick>>,
    /// The finished text waiting for the y/n before it is written. Never shown.
    pub confirm: Option<Secret>,
}

/// The staging screen (Enter on a Local or Staged change): the hunks of one file.
#[derive(Debug)]
pub struct StagingScreen {
    pub path: String,
    pub side: crate::hunks::Side,
    pub view: Load<Box<crate::hunks::View>>,
    /// Index into [`staging_rows`].
    pub cursor: usize,
}

/// One row of the staging screen: a hunk header or a line of a hunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StagingRow {
    Header(usize),
    Line(usize, usize),
}

pub fn staging_rows(view: &crate::hunks::View) -> Vec<StagingRow> {
    let mut rows = Vec::new();
    for (h, hunk) in view.hunks.iter().enumerate() {
        rows.push(StagingRow::Header(h));
        rows.extend((0..hunk.lines.len()).map(|l| StagingRow::Line(h, l)));
    }
    rows
}

/// The editor waiting to be run with the terminal handed over (see `tui::run_loop`).
pub struct Handoff {
    pub command: std::process::Command,
    pub session: crate::conflicts::EditSession,
}

/// The assignment editor: one checkbox row per target, with the state over
/// the selected paths.
#[derive(Debug, Clone)]
pub struct AssignEditor {
    pub paths: Vec<String>,
    pub rows: Vec<TargetRow>,
    /// The state each row had when the editor opened (to find what changed).
    original: Vec<Tri>,
    pub cursor: usize,
    /// Picks the one target of a new variant instead of editing assignments.
    pub variant: bool,
}

impl AssignEditor {
    pub fn new(paths: Vec<String>, rows: Vec<TargetRow>) -> Self {
        let original = rows.iter().map(|r| r.state).collect();
        Self {
            paths,
            rows,
            original,
            cursor: 0,
            variant: false,
        }
    }

    fn cycle(&mut self) {
        if let Some(r) = self.rows.get_mut(self.cursor) {
            r.state = match r.state {
                Tri::None | Tri::Mixed => Tri::Include,
                Tri::Include => Tri::Exclude,
                Tri::Exclude => Tri::None,
            };
        }
    }

    /// The edits that differ from what the editor opened with.
    pub fn edits(&self) -> Vec<crate::machines::Edit> {
        self.rows
            .iter()
            .zip(&self.original)
            .filter(|(r, o)| r.state != **o)
            .map(|(r, _)| {
                let side = match r.state {
                    Tri::Include => Some(Side::Include),
                    Tri::Exclude => Some(Side::Exclude),
                    Tri::None | Tri::Mixed => None,
                };
                (r.target.clone(), side)
            })
            .collect()
    }
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
    GitCreate(String),
    /// `n` on the Remotes tab: "NAME URL".
    RemoteAdd(String),
    GitConfirm {
        op: GitOp,
        preview: String,
        guard: String,
    },
    /// The one review listing every selected item; Enter or `y` applies.
    Review(Vec<Item>),
    /// Per-item outcome of an apply, partial results included.
    Result(Vec<ItemResult>),
    /// The commit message box: Enter commits, Ctrl+J / Alt+Enter add a line.
    Commit(CommitBox),
    /// `d` in Changes: y/n before overwriting these HOME paths.
    ConfirmDiscard(Vec<String>),
    /// Enter / Space / `a` on the Incoming tab: y/n before writing to HOME.
    ConfirmApply(Option<Vec<String>>),
    /// `P`: y/n before pushing.
    ConfirmPush,
    ConfirmRevert(String),
    /// `m`: the assignment editor.
    Assign(AssignEditor),
    /// The full text of a failure or of refusals; j/k scroll, Esc or Enter closes.
    Details {
        text: String,
        scroll: u16,
    },
}

#[derive(Debug)]
pub enum Load<T> {
    Loading,
    Ready(T),
    Failed(String),
}

pub struct App {
    pub git_tab: usize,
    pub git_selected: usize,
    pub branches: Vec<crate::branches::Branch>,
    pub stashes: Vec<crate::stashes::Stash>,
    pub remotes: Vec<crate::remote::RemoteInfo>,
    pub commits: Vec<crate::history::Commit>,
    pub commit_selected: usize,
    /// Drill-in level of the Commits tab: 0 commits, 1 the commit's files, 2 the diff.
    pub commit_level: u8,
    /// (shown name, repository path) of the files of the selected commit.
    pub commit_files: Vec<(String, String)>,
    pub commit_file_selected: usize,
    /// First line shown of the main view (diffs and previews); reset on every reload.
    pub main_scroll: u16,
    commit_limit: usize,
    /// The file whose history the Commits tab shows (`H`); `None` is all history.
    pub commit_path: Option<String>,
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
    /// Paths in the current folder that a toggle refused in this session; `a`
    /// does not wait for them. Cleared whenever the listing reloads.
    refused_paths: Arc<Mutex<BTreeSet<std::path::PathBuf>>>,
    /// Active tab of the Changes panel (index into [`TABS`]) and the row in it.
    pub changes_tab: usize,
    pub changes_selected: usize,
    /// The diff shown for the selected change.
    pub diff: Load<ratatui::text::Text<'static>>,
    /// The machine registry and the assignments; `None` until first loaded.
    pub registry: Option<Arc<RegistryView>>,
    pub machines_selected: usize,
    /// The Values screen, open while `Some`.
    pub values: Option<ValuesScreen>,
    /// The Conflicts screen, open while `Some`.
    pub conflict: Option<ConflictScreen>,
    pub staging: Option<StagingScreen>,
    handoff: Option<Handoff>,
    /// The preview of the machine last opened with Enter in the Machines panel.
    pub machine_preview: Option<(String, Load<String>)>,
    /// Change marker of the `hide` file last loaded (`None` until the first load).
    hide_stamp: Option<crate::hide::HideStamp>,
    actions: VecDeque<ActionReq>,
    action_running: bool,
    last_fetch: Option<std::time::Instant>,
}

impl App {
    pub fn new(store: Arc<Store>, rules: HideRules, tx: Sender<Event>) -> Self {
        let root = store.dirs.home.clone();
        Self {
            git_tab: 0,
            git_selected: 0,
            branches: Vec::new(),
            stashes: Vec::new(),
            remotes: Vec::new(),
            commits: Vec::new(),
            commit_selected: 0,
            commit_level: 0,
            commit_files: Vec::new(),
            commit_file_selected: 0,
            main_scroll: 0,
            commit_limit: 50,
            commit_path: None,
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
            registry: None,
            values: None,
            conflict: None,
            staging: None,
            handoff: None,
            machines_selected: 0,
            machine_preview: None,
            hide_stamp: None,
            actions: VecDeque::new(),
            refused_paths: Arc::default(),
            action_running: false,
            last_fetch: None,
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
        self.background_fetch();
        self.refresh_commits();
    }

    /// Fetch every remote without blocking; it never pulls or applies.
    fn background_fetch(&mut self) {
        self.last_fetch = Some(std::time::Instant::now());
        let store = Arc::clone(&self.store);
        self.jobs.spawn(KEY_FETCH, move |cancel| {
            Payload::Fetch(
                (|| {
                    if !store.is_initialised() || crate::remote::names(&store)?.is_empty() {
                        return Ok(None);
                    }
                    let f = crate::remote::fetch(&store, true, Some(cancel))?;
                    Ok(Some(format!("Fetched {}", f.remotes.join(", "))))
                })()
                .map_err(|e: anyhow::Error| format!("fetch failed: {e:#}")),
            )
        });
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
        self.jobs.spawn(KEY_REGISTRY, move |_| {
            Payload::Registry(
                (|| {
                    Ok(Box::new(RegistryView {
                        machines: crate::machines::list(&store)?,
                        assignments: assign::read_assignments(&store.repo())?,
                    }))
                })()
                .map_err(|e: anyhow::Error| format!("{e:#}")),
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
        self.refused_paths.lock().expect("refused set").clear();
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
        self.main_scroll = 0;
        let Some(path) = self.files.selected_path() else {
            self.preview = Load::Ready(String::new());
            return;
        };
        self.preview = Load::Loading;
        let rules = Arc::clone(&self.rules);
        let show_hidden = self.files.show_hidden;
        self.jobs.spawn(KEY_PREVIEW, move |cancel| {
            let text = files::preview(&path, &rules, show_hidden, cancel);
            Payload::Preview { path, text }
        });
    }

    pub fn handle_event(&mut self, ev: Event) {
        match ev {
            Event::Input(TermEvent::Key(key)) if key.kind != KeyEventKind::Release => {
                self.handle_key(key)
            }
            Event::Input(TermEvent::Resize(w, h)) => self.size = (w, h),
            Event::Input(TermEvent::Mouse(m)) => match m.kind {
                MouseEventKind::ScrollDown => self.scroll_main(3),
                MouseEventKind::ScrollUp => self.scroll_main(-3),
                _ => {}
            },
            Event::Input(_) => {}
            Event::Job(done) => self.handle_job(done),
            Event::Tick => {
                if self.last_fetch.is_none_or(|t| t.elapsed() >= FETCH_EVERY) {
                    self.background_fetch();
                }
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
            Payload::GitMeta(result) => match result {
                Ok((branches, stashes, remotes)) => {
                    self.branches = branches;
                    self.stashes = stashes;
                    self.remotes = remotes;
                }
                Err(e) => self.message = Some(e),
            },
            Payload::GitPreview { op, preview } => match preview {
                Ok((preview, guard)) => {
                    self.dialog = Some(Dialog::GitConfirm { op, preview, guard })
                }
                Err(e) => self.message = Some(e),
            },
            Payload::Commits(r) => match r {
                Ok(commits) => {
                    let open = self
                        .commits
                        .get(self.commit_selected)
                        .map(|c| c.hash.clone());
                    self.commits = commits;
                    self.commit_selected = self
                        .commit_selected
                        .min(self.commits.len().saturating_sub(1));
                    let same =
                        self.commits.get(self.commit_selected).map(|c| &c.hash) == open.as_ref();
                    if self.commit_level > 0 && same {
                        return;
                    }
                    self.commit_level = 0;
                    if self.focus == Panel::Git {
                        self.commit_diff();
                    }
                }
                Err(e) => self.message = Some(e),
            },
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
            Payload::CommitFiles(hash, r) => {
                if self.commits.get(self.commit_selected).map(|c| &c.hash) == Some(&hash) {
                    self.commit_files = r.unwrap_or_default();
                    self.commit_file_selected = 0;
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
            Payload::Values(r) => match r {
                Ok(view) => {
                    if let Some(v) = &mut self.values {
                        v.selected = v.selected.min(view.rows.len().saturating_sub(1));
                        v.rows = view.rows;
                        v.reveal = view.reveal;
                    }
                }
                Err(e) => self.message = Some(e),
            },
            Payload::Conflict { path, data } => {
                if let Some(c) = self.conflict.as_mut().filter(|c| c.path == path) {
                    match data {
                        Ok(d) => {
                            c.picks = vec![None; d.shown.len()];
                            c.data = Load::Ready(d);
                        }
                        Err(e) => {
                            self.conflict = None;
                            self.message = Some(e);
                        }
                    }
                }
            }
            Payload::Staging { path, view } => {
                if let Some(sc) = self.staging.as_mut().filter(|c| c.path == path) {
                    match view {
                        Ok(v) if v.hunks.is_empty() => {
                            self.staging = None;
                            self.message = Some("nothing left to stage by hunk".into());
                        }
                        Ok(v) => {
                            let last = staging_rows(&v).len().saturating_sub(1);
                            sc.cursor = sc.cursor.min(last);
                            sc.view = Load::Ready(v);
                        }
                        Err(e) => {
                            self.staging = None;
                            self.message = Some(e);
                        }
                    }
                }
            }
            Payload::Registry(r) => {
                if let Ok(v) = r {
                    self.registry = Some(Arc::new(*v));
                    self.machines_selected = self
                        .machines_selected
                        .min(self.machine_count().saturating_sub(1));
                }
            }
            Payload::MachinePreview { name, text } => {
                if self
                    .machine_preview
                    .as_ref()
                    .is_some_and(|(n, _)| *n == name)
                {
                    self.machine_preview = Some((
                        name,
                        match text {
                            Ok(t) => Load::Ready(t),
                            Err(e) => Load::Failed(e),
                        },
                    ));
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
                        if report.merge_conflicts {
                            self.focus = Panel::Changes;
                            self.changes_tab = 3;
                            self.changes_selected = 0;
                        }
                        self.message = Some(report.message);
                        self.refused_paths
                            .lock()
                            .expect("refused set")
                            .extend(report.refused);
                        if let Some(detail) = report.detail {
                            self.show_details(detail);
                        }
                        if let Some(text) = report.amend_message {
                            self.dialog = Some(Dialog::Commit(CommitBox { text, amend: true }));
                        }
                        if let Some(results) = report.results {
                            self.dialog = Some(Dialog::Result(results));
                        }
                    }
                    Err(e) => {
                        if e.starts_with("Busy:") {
                            self.message = Some(e);
                        } else {
                            self.show_details(e);
                        }
                    }
                }
                self.reload_staging();
                self.reload_values();
                self.refresh_commits();
                self.refresh_tracking();
                self.refresh_hide();
                self.refresh_status();
                self.pump_actions();
            }
            Payload::Fetch(r) => match r {
                Ok(_) => self.refresh_status(),
                Err(e) => self.message = Some(e),
            },
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

    /// Log a failure, show it in full in a dialog and keep a short summary in the footer.
    fn show_details(&mut self, text: String) {
        let state = self.store.dirs.state_dir();
        super::log::append(&state, &text);
        let first = text.lines().next().unwrap_or_default();
        let short: String = first.chars().take(80).collect();
        let more = if short.len() < first.len() { "…" } else { "" };
        self.message = Some(format!("{short}{more} (details open)"));
        let text = format!("{text}\n\nlog: {}", super::log::path(&state));
        self.dialog = Some(Dialog::Details { text, scroll: 0 });
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
            Some(Dialog::ConfirmApply(sel)) => match key.code {
                KeyCode::Char('y') => self.request_action(ActionReq::ApplyIncoming(sel)),
                KeyCode::Esc | KeyCode::Char('n') => {}
                _ => self.dialog = Some(Dialog::ConfirmApply(sel)),
            },
            Some(Dialog::ConfirmPush) => match key.code {
                KeyCode::Char('y') => self.request_action(ActionReq::Push),
                KeyCode::Esc | KeyCode::Char('n') => {}
                _ => self.dialog = Some(Dialog::ConfirmPush),
            },
            Some(Dialog::GitCreate(mut name)) => match key.code {
                KeyCode::Esc => {}
                KeyCode::Enter => self.git_preview(GitOp::Create(name)),
                KeyCode::Backspace => {
                    name.pop();
                    self.dialog = Some(Dialog::GitCreate(name));
                }
                KeyCode::Char(c) => {
                    name.push(c);
                    self.dialog = Some(Dialog::GitCreate(name));
                }
                _ => self.dialog = Some(Dialog::GitCreate(name)),
            },
            Some(Dialog::RemoteAdd(mut text)) => match key.code {
                KeyCode::Esc => {}
                KeyCode::Enter => match text.trim().split_once(' ') {
                    Some((name, url)) if !url.trim().is_empty() => {
                        self.git_preview(GitOp::RemoteAdd(name.to_string(), url.trim().to_string()))
                    }
                    _ => {
                        self.message = Some("type NAME URL".into());
                        self.dialog = Some(Dialog::RemoteAdd(text));
                    }
                },
                KeyCode::Backspace => {
                    text.pop();
                    self.dialog = Some(Dialog::RemoteAdd(text));
                }
                KeyCode::Char(c) => {
                    text.push(c);
                    self.dialog = Some(Dialog::RemoteAdd(text));
                }
                _ => self.dialog = Some(Dialog::RemoteAdd(text)),
            },
            Some(Dialog::GitConfirm { op, preview, guard }) => match key.code {
                KeyCode::Char('y') => self.request_action(ActionReq::Git(op, guard)),
                KeyCode::Esc | KeyCode::Char('n') => {}
                _ => self.dialog = Some(Dialog::GitConfirm { op, preview, guard }),
            },
            Some(Dialog::ConfirmRevert(hash)) => match key.code {
                KeyCode::Char('y') => self.request_action(ActionReq::Revert(hash)),
                KeyCode::Esc | KeyCode::Char('n') => {}
                _ => self.dialog = Some(Dialog::ConfirmRevert(hash)),
            },
            Some(Dialog::Assign(mut e)) if e.variant => {
                let last = e.rows.len().saturating_sub(1);
                match key.code {
                    KeyCode::Esc => return,
                    KeyCode::Char('j') | KeyCode::Down => e.cursor = (e.cursor + 1).min(last),
                    KeyCode::Char('k') | KeyCode::Up => e.cursor = e.cursor.saturating_sub(1),
                    KeyCode::Enter => {
                        if let Some(r) = e.rows.get(e.cursor) {
                            return self.request_action(ActionReq::Variant(
                                e.paths[0].clone(),
                                r.target.clone(),
                            ));
                        }
                    }
                    _ => {}
                }
                self.dialog = Some(Dialog::Assign(e));
            }
            Some(Dialog::Assign(mut e)) => {
                let last = e.rows.len().saturating_sub(1);
                match key.code {
                    KeyCode::Esc => return,
                    KeyCode::Char('j') | KeyCode::Down => e.cursor = (e.cursor + 1).min(last),
                    KeyCode::Char('k') | KeyCode::Up => e.cursor = e.cursor.saturating_sub(1),
                    KeyCode::Char(' ') => e.cycle(),
                    KeyCode::Char(c @ ('i' | 'x' | '-')) => {
                        if let Some(r) = e.rows.get_mut(e.cursor) {
                            r.state = match c {
                                'i' => Tri::Include,
                                'x' => Tri::Exclude,
                                _ => Tri::None,
                            };
                        }
                    }
                    KeyCode::Enter => {
                        let edits = e.edits();
                        if edits.is_empty() {
                            self.message = Some("no changes to the assignment".into());
                            self.dialog = Some(Dialog::Assign(e));
                            return;
                        }
                        return self.request_action(ActionReq::Assign(e.paths, edits));
                    }
                    _ => {}
                }
                self.dialog = Some(Dialog::Assign(e));
            }
            Some(Dialog::Details { text, scroll }) => {
                let last = text.lines().count().saturating_sub(1) as u16;
                let scroll = match key.code {
                    KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => return,
                    KeyCode::Char('j') | KeyCode::Down => (scroll + 1).min(last),
                    KeyCode::Char('k') | KeyCode::Up => scroll.saturating_sub(1),
                    KeyCode::PageDown => (scroll + 10).min(last),
                    KeyCode::PageUp => scroll.saturating_sub(10),
                    _ => scroll,
                };
                self.dialog = Some(Dialog::Details { text, scroll });
            }
            Some(Dialog::Result(_)) | None => {}
        }
    }

    pub fn machine_count(&self) -> usize {
        self.registry.as_ref().map_or(0, |r| r.machines.len())
    }

    /// `m`: open the assignment editor for these HOME-relative paths. Each must
    /// be tracked, or a folder holding tracked files.
    fn open_assign_editor(&mut self, paths: Vec<String>) {
        let Some(reg) = self.registry.clone() else {
            self.message = Some("the machine registry is still loading".into());
            return;
        };
        if paths.is_empty() {
            return;
        }
        if let Some(bad) = paths.iter().find(|p| {
            !(self.tracked.is_tracked(p)
                || self.tracked.folders.contains(*p)
                || self.tracked.under(p).next().is_some())
        }) {
            self.message = Some(format!("~/{bad} is not tracked: track it first (Space)"));
            return;
        }
        let machines: Vec<_> = reg.machines.iter().map(|l| l.machine.clone()).collect();
        let rows = assign::editor_rows(&machines, &reg.assignments, &paths);
        self.dialog = Some(Dialog::Assign(AssignEditor::new(paths, rows)));
    }

    /// `t`: pick the target of a new variant of this tracked file; targets that
    /// already have a variant are not offered.
    fn open_variant_picker(&mut self, path: String) {
        let Some(reg) = self.registry.clone() else {
            self.message = Some("the machine registry is still loading".into());
            return;
        };
        if !self.tracked.is_tracked(&path) {
            self.message = Some(format!("~/{path} is not a tracked file"));
            return;
        }
        let taken: Vec<String> = match &self.status {
            Load::Ready(s) => s
                .variants
                .iter()
                .filter(|v| v.path == path)
                .flat_map(|v| v.targets.clone())
                .collect(),
            _ => Vec::new(),
        };
        let machines: Vec<_> = reg
            .machines
            .iter()
            .filter(|l| !l.machine.retired)
            .map(|l| l.machine.clone())
            .collect();
        let mut rows =
            assign::editor_rows(&machines, &Default::default(), std::slice::from_ref(&path));
        rows.retain(|r| !taken.contains(&r.target));
        let mut editor = AssignEditor::new(vec![path], rows);
        editor.variant = true;
        self.dialog = Some(Dialog::Assign(editor));
    }

    fn machines_key(&mut self, key: KeyEvent) {
        let n = self.machine_count();
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.machines_selected = (self.machines_selected + 1).min(n.saturating_sub(1));
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.machines_selected = self.machines_selected.saturating_sub(1);
            }
            KeyCode::Char('h') | KeyCode::Left => self.focus = self.focus.shifted(-1),
            KeyCode::Char('l') | KeyCode::Right => self.focus = self.focus.shifted(1),
            KeyCode::Enter => {
                let Some(name) = self
                    .registry
                    .as_ref()
                    .and_then(|r| r.machines.get(self.machines_selected))
                    .map(|l| l.machine.name.clone())
                else {
                    return;
                };
                self.machine_preview = Some((name.clone(), Load::Loading));
                let store = Arc::clone(&self.store);
                self.jobs.spawn(KEY_MACHINE, move |_| {
                    let text = crate::machines::preview_as(&store, &name, false)
                        .map(|p| {
                            let mut s =
                                format!("{} would receive {} file(s):\n\n", name, p.files.len());
                            for l in p.lines() {
                                s.push_str(&format!("{l}\n"));
                            }
                            s
                        })
                        .map_err(|e| format!("{e:#}"));
                    Payload::MachinePreview { name, text }
                });
            }
            _ => {}
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
        self.main_scroll = 0;
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
            KeyCode::Char('H') => {
                if let Some(c) = selected {
                    return self.open_history(c.path);
                }
            }
            KeyCode::Enter if tab == 2 => {
                if let Some(c) = selected {
                    self.dialog = Some(Dialog::ConfirmApply(Some(vec![c.path])));
                }
            }
            KeyCode::Enter if tab == 3 => {
                if let Some(c) = selected {
                    if c.kind == "restore" {
                        self.message = Some("Historical restore collides or is stale: clear with `dw restore --clear PATH`, preserve edits and re-plan".into());
                    } else {
                        self.open_conflict(c.path);
                    }
                }
            }
            KeyCode::Enter => match selected {
                Some(c) if c.kind == "repo" => {
                    self.message = Some("repository files are staged and unstaged whole".into())
                }
                Some(c) => self.open_staging(c.path, tab == 1),
                None => {}
            },
            KeyCode::Char(' ') => match (tab, selected) {
                (0, Some(c)) => self.request_action(ActionReq::Stage(Some(vec![c.path]))),
                (1, Some(c)) if c.kind == "repo" => {
                    self.message = Some("repository files are unstaged together: press a".into())
                }
                (1, Some(c)) => self.request_action(ActionReq::Unstage(Some(vec![c.path]))),
                (2, Some(c)) => self.dialog = Some(Dialog::ConfirmApply(Some(vec![c.path]))),
                (3, Some(_)) => self.message = Some(CONFLICT_BLOCKED.into()),
                _ => {}
            },
            KeyCode::Char('a') => match tab {
                0 => self.request_action(ActionReq::Stage(None)),
                1 => self.request_action(ActionReq::Unstage(None)),
                2 if rows > 0 => self.dialog = Some(Dialog::ConfirmApply(None)),
                2 => {}
                _ => self.message = Some(CONFLICT_BLOCKED.into()),
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
            KeyCode::Char('t') => match selected {
                Some(c) if c.kind != "repo" => self.open_variant_picker(c.path),
                _ => {}
            },
            KeyCode::Char('m') => match selected {
                Some(c) if c.kind != "repo" => self.open_assign_editor(vec![c.path]),
                Some(_) => self.message = Some("repository files have no assignment".into()),
                None => {}
            },
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
        if self.focus == Panel::Git && before != Panel::Git {
            self.commit_diff();
        }
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
        if self.conflict.is_some() {
            return match key.code {
                KeyCode::Char('c') if ctrl => self.quit = true,
                _ => self.conflict_key(key),
            };
        }
        if self.staging.is_some() {
            return match key.code {
                KeyCode::Char('c') if ctrl => self.quit = true,
                _ => self.staging_key(key),
            };
        }
        if self.values.is_some() {
            return match key.code {
                KeyCode::Char('c') if ctrl => self.quit = true,
                _ => self.values_key(key),
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
                self.focus = Panel::ALL[c as usize - '1' as usize];
                return;
            }
            KeyCode::Char('0') => return self.focus = Panel::Pending,
            KeyCode::Char('V') => {
                self.values = Some(ValuesScreen::default());
                return self.reload_values();
            }
            KeyCode::Char('f') => return self.request_action(ActionReq::Fetch),
            KeyCode::Char('z') => return self.request_action(ActionReq::Undo),
            KeyCode::Char('p') => return self.request_action(ActionReq::Pull),
            KeyCode::Char('P') => return self.dialog = Some(Dialog::ConfirmPush),
            KeyCode::Char('r') if self.focus != Panel::Git => {
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
            Panel::Machines => self.machines_key(key),
            Panel::Git => self.git_key(key),
            _ => match key.code {
                KeyCode::Char('h') | KeyCode::Left => self.focus = self.focus.shifted(-1),
                KeyCode::Char('l') | KeyCode::Right => self.focus = self.focus.shifted(1),
                _ => {}
            },
        }
    }

    pub fn take_handoff(&mut self) -> Option<Handoff> {
        self.handoff.take()
    }

    fn refresh_commits(&mut self) {
        let meta_store = Arc::clone(&self.store);
        self.jobs.spawn("git-meta", move |_| {
            Payload::GitMeta(
                (|| -> anyhow::Result<_> {
                    if !meta_store.is_initialised() {
                        return Ok(Default::default());
                    }
                    Ok((
                        crate::branches::list(&meta_store)?,
                        crate::stashes::list(&meta_store)?,
                        crate::remote::list(&meta_store)?,
                    ))
                })()
                .map_err(|e| format!("{e:#}")),
            )
        });
        let store = Arc::clone(&self.store);
        let count = self.commit_limit;
        let path = self.commit_path.clone();
        self.jobs.spawn("commits", move |_| {
            if !store.is_initialised() {
                return Payload::Commits(Ok(Vec::new()));
            }
            Payload::Commits(
                crate::history::log(&store, path.as_deref(), 0, count)
                    .map_err(|e| format!("{e:#}")),
            )
        });
    }

    /// `H`: show the history of this file (HOME-relative or absolute) on the Commits tab.
    fn open_history(&mut self, path: String) {
        self.commit_path = Some(path);
        self.commits.clear();
        self.commit_selected = 0;
        self.commit_limit = 50;
        self.git_tab = 0;
        self.diff = Load::Ready(Default::default());
        self.focus = Panel::Git;
        self.refresh_commits();
    }

    fn commit_diff(&mut self) {
        if self.git_tab != 0 {
            return;
        }
        let Some(c) = self.commits.get(self.commit_selected) else {
            return;
        };
        let hash = c.hash.clone();
        let path = self.commit_path.clone();
        let file = (self.commit_level > 0)
            .then(|| self.commit_files.get(self.commit_file_selected))
            .flatten()
            .map(|f| f.1.clone());
        let store = Arc::clone(&self.store);
        self.diff = Load::Loading;
        self.main_scroll = 0;
        self.jobs.spawn(KEY_DIFF, move |cancel| {
            use ansi_to_tui::IntoText;
            let shown = match &file {
                Some(f) => crate::history::diff_file(&store, &hash, f),
                None => crate::history::diff(&store, &hash, path.as_deref()),
            };
            let result = shown.and_then(|plain| {
                let shown = changes::through_delta(&store, &plain, Some(cancel)).unwrap_or(plain);
                Ok(shown.into_text()?)
            });
            Payload::Diff(result.map_err(|e| format!("{e:#}")))
        });
    }

    /// Enter on a commit: list its files and focus the list.
    fn open_commit_files(&mut self) {
        let Some(c) = self.commits.get(self.commit_selected) else {
            return;
        };
        let hash = c.hash.clone();
        self.commit_level = 1;
        self.commit_files.clear();
        self.commit_file_selected = 0;
        let store = Arc::clone(&self.store);
        self.jobs.spawn("commit-files", move |_| {
            let files = crate::history::commit_files(&store, &hash).map_err(|e| format!("{e:#}"));
            Payload::CommitFiles(hash, files)
        });
        self.commit_diff();
    }

    /// Number of lines in the main view's current text, to bound scrolling.
    fn main_lines(&self) -> usize {
        match (self.focus, &self.diff, &self.preview) {
            (Panel::Changes | Panel::Git, Load::Ready(t), _) => t.lines.len(),
            (Panel::Files, _, Load::Ready(t)) => t.lines().count() + 3,
            _ => 0,
        }
    }

    pub fn scroll_main(&mut self, delta: isize) {
        let last = self.main_lines().saturating_sub(1);
        self.main_scroll = self
            .main_scroll
            .saturating_add_signed(delta.clamp(-1000, 1000) as i16)
            .min(last as u16);
    }

    /// j/k/PgUp/PgDn/Home/End while the diff of a commit file has focus.
    fn commit_diff_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.scroll_main(1),
            KeyCode::Char('k') | KeyCode::Up => self.scroll_main(-1),
            KeyCode::PageDown => self.scroll_main(10),
            KeyCode::PageUp => self.scroll_main(-10),
            KeyCode::Char('<') | KeyCode::Home => self.main_scroll = 0,
            KeyCode::Char('>') | KeyCode::End => self.scroll_main(isize::MAX),
            KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left => self.commit_level = 1,
            _ => {}
        }
    }

    /// j/k/Enter/Esc while the files of a commit have focus.
    fn commit_files_key(&mut self, key: KeyEvent) {
        let last = self.commit_files.len().saturating_sub(1);
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.commit_file_selected = (self.commit_file_selected + 1).min(last);
                self.commit_diff();
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.commit_file_selected = self.commit_file_selected.saturating_sub(1);
                self.commit_diff();
            }
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => self.commit_level = 2,
            KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left => {
                self.commit_level = 0;
                self.commit_diff();
            }
            _ => {}
        }
    }

    fn git_key(&mut self, key: KeyEvent) {
        if self.git_tab == 0 && self.commit_level == 2 {
            return self.commit_diff_key(key);
        }
        if self.git_tab == 0 && self.commit_level == 1 {
            return self.commit_files_key(key);
        }
        match key.code {
            KeyCode::Char('[') | KeyCode::Char(']') => {
                let n = GIT_TABS.len();
                self.git_tab = (self.git_tab
                    + if key.code == KeyCode::Char(']') {
                        1
                    } else {
                        n - 1
                    })
                    % n;
                self.git_selected = 0;
                self.commit_diff();
                return;
            }
            KeyCode::Char('R') if self.git_tab == 0 => {
                if let Some(c) = self.commits.get(self.commit_selected) {
                    self.git_preview(GitOp::Restore(c.hash.clone()));
                }
                return;
            }
            KeyCode::Esc if self.git_tab == 0 && self.commit_path.is_some() => {
                self.commit_path = None;
                self.commit_selected = 0;
                self.commit_limit = 50;
                return self.refresh_commits();
            }
            _ => {}
        }
        if self.git_tab != 0 {
            let n = match self.git_tab {
                1 => self.branches.len(),
                2 => self.remotes.len(),
                _ => self.stashes.len(),
            };
            let branch = self.branches.get(self.git_selected).map(|b| b.name.clone());
            let remote = self.remotes.get(self.git_selected).map(|r| r.name.clone());
            let stash = self.stashes.get(self.git_selected).map(|s| s.id.clone());
            match key.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    self.git_selected = (self.git_selected + 1).min(n.saturating_sub(1))
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.git_selected = self.git_selected.saturating_sub(1)
                }
                KeyCode::Char('n') if self.git_tab == 1 => {
                    self.dialog = Some(Dialog::GitCreate(String::new()))
                }
                KeyCode::Enter if self.git_tab == 1 => {
                    if let Some(b) = branch {
                        self.git_preview(GitOp::Switch(b));
                    }
                }
                KeyCode::Char('d' | 'D') if self.git_tab == 1 => {
                    if let Some(b) = branch {
                        self.git_preview(GitOp::Delete(b, key.code == KeyCode::Char('D')));
                    }
                }
                KeyCode::Char('n') if self.git_tab == 2 => {
                    self.dialog = Some(Dialog::RemoteAdd(String::new()))
                }
                KeyCode::Char('d') if self.git_tab == 2 => {
                    if let Some(r) = remote {
                        self.git_preview(GitOp::RemoteRemove(r));
                    }
                }
                KeyCode::Char('s') if self.git_tab == 3 => self.git_preview(GitOp::StashPush),
                KeyCode::Enter if self.git_tab == 3 => {
                    if let Some(s) = stash {
                        self.git_preview(GitOp::StashPop(s));
                    }
                }
                KeyCode::Char('d') if self.git_tab == 3 => {
                    if let Some(s) = stash {
                        self.git_preview(GitOp::StashDrop(s));
                    }
                }
                _ => {}
            }
            return;
        }
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.commit_selected =
                    (self.commit_selected + 1).min(self.commits.len().saturating_sub(1));
                if self.commit_selected + 1 == self.commits.len()
                    && self.commits.len() == self.commit_limit
                {
                    self.commit_limit += 50;
                    self.refresh_commits();
                }
                self.commit_diff();
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.commit_selected = self.commit_selected.saturating_sub(1);
                self.commit_diff();
            }
            KeyCode::Enter => self.open_commit_files(),
            KeyCode::Char('r') => {
                if let Some(c) = self.commits.get(self.commit_selected) {
                    self.dialog = Some(Dialog::ConfirmRevert(c.hash.clone()));
                }
            }
            KeyCode::Char('M') => self.request_action(ActionReq::MergePull),
            KeyCode::Char('C') => self.request_action(ActionReq::ContinuePull),
            KeyCode::Char('X') => self.request_action(ActionReq::AbortPull),
            KeyCode::Char('h') | KeyCode::Left => self.focus = self.focus.shifted(-1),
            KeyCode::Char('l') | KeyCode::Right => self.focus = self.focus.shifted(1),
            _ => {}
        }
    }

    fn git_preview(&mut self, op: GitOp) {
        let store = Arc::clone(&self.store);
        self.jobs.spawn("git-preview", move |_| {
            let preview = (|| -> anyhow::Result<_> {
                let guard = crate::git_state::preview_guard(&store)?;
                let text = preview_git(&store, &op)?;
                if crate::git_state::preview_guard(&store)? != guard {
                    anyhow::bail!("state changed during preview; try again");
                }
                Ok((text, guard))
            })()
            .map_err(|e| format!("{e:#}"));
            Payload::GitPreview { op, preview }
        });
    }

    /// The editor returned: its merge file becomes the result awaiting y/n.
    pub fn finish_handoff(
        &mut self,
        session: crate::conflicts::EditSession,
        status: Result<std::process::ExitStatus, String>,
    ) {
        let Some(c) = &mut self.conflict else { return };
        match status {
            Err(e) => self.message = Some(format!("could not start the editor: {e}")),
            Ok(s) if !s.success() => {
                self.message = Some(format!("the editor exited with {s}; nothing was written"))
            }
            Ok(_) => match session.read_result() {
                Ok(text) => c.confirm = Some(Secret(text)),
                Err(e) => self.message = Some(format!("{e:#}")),
            },
        }
    }

    fn open_staging(&mut self, path: String, staged: bool) {
        self.staging = Some(StagingScreen {
            path,
            side: if staged {
                crate::hunks::Side::Unstage
            } else {
                crate::hunks::Side::Stage
            },
            view: Load::Loading,
            cursor: 0,
        });
        self.reload_staging();
    }

    /// Re-read the hunks of the open staging screen (after every action).
    fn reload_staging(&mut self) {
        let Some(sc) = &self.staging else { return };
        let (path, side) = (sc.path.clone(), sc.side);
        let store = Arc::clone(&self.store);
        self.jobs.spawn(KEY_STAGING, move |_| {
            let view = crate::hunks::open(&store, &path, side)
                .map(Box::new)
                .map_err(|e| format!("{e:#}"));
            Payload::Staging { path, view }
        });
    }

    fn staging_key(&mut self, key: KeyEvent) {
        let Some(sc) = &mut self.staging else { return };
        let Load::Ready(view) = &sc.view else {
            if key.code == KeyCode::Esc {
                self.staging = None;
            }
            return;
        };
        let rows = staging_rows(view);
        let at = rows.get(sc.cursor).copied();
        let hunk_of = |r: StagingRow| match r {
            StagingRow::Header(h) | StagingRow::Line(h, _) => h,
        };
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.staging = None,
            KeyCode::Char('j') | KeyCode::Down => {
                sc.cursor = (sc.cursor + 1).min(rows.len().saturating_sub(1))
            }
            KeyCode::Char('k') | KeyCode::Up => sc.cursor = sc.cursor.saturating_sub(1),
            KeyCode::Char(']') | KeyCode::Tab => {
                let h = at.map_or(0, hunk_of);
                if let Some(i) = rows.iter().position(|r| *r == StagingRow::Header(h + 1)) {
                    sc.cursor = i;
                }
            }
            KeyCode::Char('[') | KeyCode::BackTab => {
                let h = at.map_or(0, hunk_of);
                let to = match at {
                    Some(StagingRow::Line(..)) => h,
                    _ => h.saturating_sub(1),
                };
                if let Some(i) = rows.iter().position(|r| *r == StagingRow::Header(to)) {
                    sc.cursor = i;
                }
            }
            KeyCode::Char(' ') | KeyCode::Enter | KeyCode::Char('a') => {
                if let Some(why) = &view.readonly {
                    self.message = Some(format!("read-only: {why}"));
                    return;
                }
                let mut sel = crate::hunks::Selection::default();
                match at {
                    Some(StagingRow::Line(h, _)) if key.code == KeyCode::Char('a') => {
                        sel.hunks.push(view.hunks[h].number)
                    }
                    Some(StagingRow::Line(h, l)) => match view.hunks[h].lines[l].id {
                        Some(id) => sel.lines.push(id),
                        None => {
                            self.message =
                                Some("that line is context; move to a + or - line".into());
                            return;
                        }
                    },
                    Some(StagingRow::Header(h)) => sel.hunks.push(view.hunks[h].number),
                    None => return,
                }
                let req = ActionReq::Hunks {
                    path: sc.path.clone(),
                    side: sc.side,
                    sel,
                    expect: view.fingerprint.clone(),
                };
                self.request_action(req);
            }
            _ => {}
        }
    }

    fn open_conflict(&mut self, path: String) {
        self.conflict = Some(ConflictScreen {
            stored: self.selected_change().is_some_and(|c| c.kind == "merge"),
            path: path.clone(),
            data: Load::Loading,
            cursor: 0,
            picks: Vec::new(),
            confirm: None,
        });
        let store = Arc::clone(&self.store);
        self.jobs.spawn(KEY_CONFLICT, move |_| {
            let data = crate::conflicts::load(&store, &path)
                .map(|loaded| {
                    let shown = crate::conflicts::hunks(&loaded.chunks)
                        .into_iter()
                        .map(|h| ShownHunk {
                            line: h.line,
                            base: crate::conflicts::masked(&store, &h.base.concat()),
                            local: crate::conflicts::masked(&store, &h.local.concat()),
                            incoming: crate::conflicts::masked(&store, &h.incoming.concat()),
                        })
                        .collect();
                    Box::new(super::jobs::ConflictData { loaded, shown })
                })
                .map_err(|e| format!("{e:#}"));
            Payload::Conflict { path, data }
        });
    }

    fn conflict_key(&mut self, key: KeyEvent) {
        let Some(c) = &mut self.conflict else { return };
        if let Some(text) = c.confirm.take() {
            match key.code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    let path = c.path.clone();
                    self.conflict = None;
                    return self.request_action(ActionReq::Resolve(path, text));
                }
                KeyCode::Char('n') | KeyCode::Esc => {}
                _ => c.confirm = Some(text),
            }
            return;
        }
        let Load::Ready(data) = &c.data else {
            if key.code == KeyCode::Esc {
                self.conflict = None;
            }
            return;
        };
        let n = data.shown.len();
        match key.code {
            KeyCode::Esc => self.conflict = None,
            KeyCode::Char('j') | KeyCode::Down => {
                c.cursor = (c.cursor + 1).min(n.saturating_sub(1))
            }
            KeyCode::Char('k') | KeyCode::Up => c.cursor = c.cursor.saturating_sub(1),
            KeyCode::Char(ch @ ('l' | 'i' | 'b')) if n > 0 => {
                c.picks[c.cursor] = Some(match ch {
                    'l' => Pick::Local,
                    'i' => Pick::Incoming,
                    _ => Pick::Both,
                });
                if let Some(next) = (0..n)
                    .map(|k| (c.cursor + 1 + k) % n)
                    .find(|k| c.picks[*k].is_none())
                {
                    c.cursor = next;
                }
            }
            KeyCode::Char('e') => {
                match crate::conflicts::EditSession::new(&self.store, &data.loaded, &c.picks) {
                    Ok(session) => {
                        let command = session.command(&self.store);
                        self.handoff = Some(Handoff { command, session });
                    }
                    Err(e) => self.message = Some(format!("{e:#}")),
                }
            }
            KeyCode::Char('w') | KeyCode::Enter => {
                match crate::conflicts::result(&data.loaded.chunks, &c.picks) {
                    Some(text) => c.confirm = Some(Secret(text)),
                    None => self.message = Some("choose l / i / b for every hunk first".into()),
                }
            }
            _ => {}
        }
    }

    /// Re-read the variables for the open Values screen.
    fn reload_values(&mut self) {
        if self.values.is_none() {
            return;
        }
        let store = Arc::clone(&self.store);
        self.jobs.spawn(KEY_VALUES, move |_| {
            Payload::Values(
                crate::values::list(&store)
                    .map(|rows| {
                        Box::new(ValuesView {
                            rows,
                            reveal: crate::values::reveal(&store),
                        })
                    })
                    .map_err(|e| format!("{e:#}")),
            )
        });
    }

    fn values_key(&mut self, key: KeyEvent) {
        let Some(v) = &mut self.values else { return };
        if let Some(input) = &mut v.input {
            match key.code {
                KeyCode::Esc => v.input = None,
                KeyCode::Backspace => {
                    input.text.pop();
                }
                KeyCode::Char(c) => input.text.push(c),
                KeyCode::Enter => {
                    let input = v.input.take().expect("checked");
                    let text = input.text.trim().to_string();
                    let secret_req = match &input.edit {
                        ValueEdit::Unlock => {
                            // The passphrase is not trimmed.
                            return self.request_action(ActionReq::Unlock(Secret(input.text)));
                        }
                        ValueEdit::SecretDefault(n) => Some((n.clone(), None, text.clone())),
                        ValueEdit::SecretOverride(n) => text.split_once(' ').map(|(t, val)| {
                            (n.clone(), Some(t.to_string()), val.trim().to_string())
                        }),
                        ValueEdit::NewSecret => text
                            .split_once(' ')
                            .map(|(n, val)| (n.to_string(), None, val.trim().to_string())),
                        _ => None,
                    };
                    if matches!(
                        input.edit,
                        ValueEdit::SecretDefault(_)
                            | ValueEdit::SecretOverride(_)
                            | ValueEdit::NewSecret
                    ) {
                        return match secret_req {
                            Some((name, target, value)) => {
                                self.request_action(ActionReq::SecretSet {
                                    name,
                                    target,
                                    value: Secret(value),
                                })
                            }
                            None => self.message = Some("type: TARGET VALUE or NAME VALUE".into()),
                        };
                    }
                    let req = match &input.edit {
                        ValueEdit::Default(name) => Some((name.clone(), None, text)),
                        ValueEdit::Override(name) => text
                            .split_once(' ')
                            .map(|(t, val)| (name.clone(), Some(t.to_string()), val.trim().into())),
                        ValueEdit::New => text
                            .split_once(' ')
                            .map(|(n, val)| (n.to_string(), None, val.trim().into())),
                        _ => None,
                    };
                    match req {
                        Some((name, target, value)) => {
                            self.request_action(ActionReq::Value {
                                name,
                                target,
                                value,
                            });
                        }
                        None => {
                            self.message = Some(match input.edit {
                                ValueEdit::New => "type: NAME VALUE".to_string(),
                                _ => "type: TARGET VALUE (for example machine:mac 14)".to_string(),
                            });
                        }
                    }
                }
                _ => {}
            }
            return;
        }
        let last = v.rows.len().saturating_sub(1);
        match key.code {
            KeyCode::Esc => self.values = None,
            KeyCode::Char('j') | KeyCode::Down => v.selected = (v.selected + 1).min(last),
            KeyCode::Char('k') | KeyCode::Up => v.selected = v.selected.saturating_sub(1),
            KeyCode::Char('n') => {
                v.input = Some(ValueInput {
                    edit: ValueEdit::New,
                    text: String::new(),
                })
            }
            KeyCode::Char('s') => {
                v.input = Some(ValueInput {
                    edit: ValueEdit::NewSecret,
                    text: String::new(),
                })
            }
            KeyCode::Char('u') => {
                v.input = Some(ValueInput {
                    edit: ValueEdit::Unlock,
                    text: String::new(),
                })
            }
            KeyCode::Char('L') => self.request_action(ActionReq::Lock),
            KeyCode::Enter | KeyCode::Char('e') | KeyCode::Char('o') => {
                if let Some(row) = v.rows.get(v.selected) {
                    let over = key.code == KeyCode::Char('o');
                    let secret = row.kind == "secret";
                    v.input = Some(ValueInput {
                        edit: match (secret, over) {
                            (true, true) => ValueEdit::SecretOverride(row.name.clone()),
                            (true, false) => ValueEdit::SecretDefault(row.name.clone()),
                            (false, true) => ValueEdit::Override(row.name.clone()),
                            (false, false) => ValueEdit::Default(row.name.clone()),
                        },
                        text: if over || secret {
                            String::new()
                        } else {
                            row.default.clone()
                        },
                    });
                }
            }
            _ => {}
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

    /// `a` (track the rest, or deselect all when nothing is left to track) and
    /// `A` (drop every pending item) in Files. Both are queued as one action and
    /// decided when it runs; see [`ActionReq::Bulk`].
    fn bulk_selection(&mut self, drop_only: bool) {
        let entries = self
            .files
            .visible()
            .iter()
            .map(|e| BulkEntry {
                path: self.files.cwd.join(&e.name),
                rel: self.files.rel_of(e),
                is_dir: e.is_dir,
            })
            .zip(self.files.visible().iter().map(|e| e.hidden))
            .filter(|(_, hidden)| !hidden || drop_only)
            .map(|(b, _)| b)
            .collect();
        self.range_anchor = None;
        self.request_action(ActionReq::Bulk {
            entries,
            drop_only,
            refused: Arc::clone(&self.refused_paths),
        });
    }

    fn files_key(&mut self, key: KeyEvent) {
        let before = self.files.selected_path();
        match key.code {
            KeyCode::Char(' ') => return self.toggle_selection(),
            KeyCode::Char('a') => return self.bulk_selection(false),
            KeyCode::Char('A') => return self.bulk_selection(true),
            KeyCode::Char('H') => {
                if let Some(p) = before {
                    self.open_history(p.to_string_lossy().into_owned());
                }
                return;
            }
            KeyCode::Char('i') => {
                if let Some(e) = self.files.selected_entry() {
                    let (path, is_dir) = (self.files.cwd.join(&e.name), e.is_dir);
                    self.request_action(ActionReq::Hide(path, is_dir));
                }
                return;
            }
            KeyCode::Char('t') => {
                if let Some(e) = self.files.selected_entry() {
                    let rel = self.files.rel_of(e);
                    return self.open_variant_picker(rel);
                }
                return;
            }
            KeyCode::Char('E') => {
                if let Some(e) = self.files.selected_entry() {
                    let rel = self.files.rel_of(e);
                    return self.request_action(ActionReq::Encrypt(rel));
                }
                return;
            }
            KeyCode::Char('m') => {
                let visible = self.files.visible();
                let (lo, hi) = self
                    .range_rows()
                    .unwrap_or((self.files.selected, self.files.selected));
                let paths: Vec<String> = (lo..=hi)
                    .filter_map(|i| visible.get(i))
                    .map(|e| self.files.rel_of(e))
                    .collect();
                self.range_anchor = None;
                return self.open_assign_editor(paths);
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
            KeyCode::Char('.') => {
                self.files.toggle_hidden();
                return self.request_preview();
            }
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
        merge_conflicts: false,
        message,
        results: None,
        amend_message: None,
        detail: None,
        refused: Vec::new(),
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
    if let Some(n) = d.notes.first() {
        m.push_str(&format!("; {n}"));
    }
    m
}

const CONFLICT_BLOCKED: &str =
    "conflicts are not applied: they changed here and in the repository; Enter resolves them";

/// How often the background fetch repeats (the preference comes later).
const FETCH_EVERY: std::time::Duration = std::time::Duration::from_secs(300);

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
    if c.kind == "restore" {
        let plain = crate::restores::preview(store, &c.path)?;
        return Ok(changes::through_delta(store, &plain, Some(cancel))
            .unwrap_or(plain)
            .into_text()?);
    }
    if c.state == State::Incoming {
        return Ok(format!(
            "~/{} has an incoming change.\n\nSpace or Enter applies it to HOME (a snapshot is taken first); a applies all.",
            c.path
        )
        .into());
    }
    if c.state == State::Conflict {
        return Ok(format!(
            "~/{} changed here and in the repository.\n\nIt is not applied. Enter picks local / incoming per hunk (or `dw resolve`).",
            c.path
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
        ActionReq::Git(op, guard) => {
            if crate::git_state::preview_guard(store)? != guard {
                return Err(crate::error::DwError::new(
                    crate::error::Kind::Stale,
                    "state changed since preview; review again",
                )
                .into());
            }
            match &op {
                GitOp::Create(n) | GitOp::Switch(n) => {
                    crate::branches::change(store, n, matches!(op, GitOp::Create(_)), false)?;
                }
                GitOp::StashPush => {
                    crate::stashes::push(store, false)?;
                }
                GitOp::StashPop(id) => {
                    crate::stashes::pop(store, Some(id), false)?;
                }
                GitOp::Restore(hash) => {
                    crate::restores::plan(store, hash, None, false)?;
                }
                GitOp::RemoteAdd(..) | GitOp::RemoteRemove(_) => {
                    if changes::agent(store).is_some() {
                        let requester = pending::requester(store);
                        let item = match &op {
                            GitOp::RemoteAdd(name, url) => {
                                crate::remote::plan_add_item(store, name, url, &requester)?
                            }
                            GitOp::RemoteRemove(name) => {
                                crate::remote::plan_remove_item(store, name, &requester)?
                            }
                            _ => unreachable!(),
                        };
                        let lock = store.lock()?;
                        let ctx = track::Ctx::load(store)?;
                        pending::add(store, &lock, vec![item], &track::stale_checker(store, &ctx))?;
                        return Ok(report("Queued for human approval".into()));
                    }
                    let _lock = store.lock()?;
                    return Ok(report(match &op {
                        GitOp::RemoteAdd(name, url) => {
                            // The y on the preview is the human confirming an unverifiable host.
                            match crate::remote::add_locked(store, name, url, Some(&|_| Ok(true)))?
                            {
                                Some(w) => format!("Added remote {name}; {w}"),
                                None => format!("Added remote {name}"),
                            }
                        }
                        GitOp::RemoteRemove(name) => {
                            crate::remote::remove_locked(store, name)?;
                            format!("Removed remote {name}")
                        }
                        _ => unreachable!(),
                    }));
                }
                GitOp::Delete(..) | GitOp::StashDrop(_) => {
                    let item = match &op {
                        GitOp::Delete(n, f) => crate::branches::plan_delete(store, n, *f)?,
                        GitOp::StashDrop(id) => crate::stashes::plan_drop(store, Some(id))?,
                        _ => unreachable!(),
                    };
                    if changes::agent(store).is_some() {
                        let lock = store.lock()?;
                        let ctx = track::Ctx::load(store)?;
                        pending::add(store, &lock, vec![item], &track::stale_checker(store, &ctx))?;
                        return Ok(report("Queued for human approval".into()));
                    }
                    let done = track::apply_now(store, &[item])?;
                    if let Some(bad) = done.iter().find(|r| r.status != track::Outcome::Applied) {
                        anyhow::bail!("{}", bad.message.as_deref().unwrap_or("operation refused"));
                    }
                }
            }
            Ok(report(format!(
                "Completed {op:?}; review Changes for Incoming/Conflicts"
            )))
        }
        ActionReq::Revert(hash) => {
            let c = crate::history::revert(store, &hash, changes::agent(store).as_deref())?;
            Ok(report(format!("Reverted in {}; see Incoming", c.short)))
        }
        ActionReq::MergePull => {
            let p = crate::remote::pull(store, None, crate::remote::PullMode::Merge)?;
            let mut r = report(if p.conflicts.is_empty() {
                "Pulled; see Incoming".into()
            } else {
                "Merge conflicts: resolve in Changes → Conflicts, then Git C (continue), or X (abort)".into()
            });
            r.merge_conflicts = !p.conflicts.is_empty();
            Ok(r)
        }
        ActionReq::ContinuePull => {
            crate::remote::pull_continue(store)?;
            Ok(report("Merge committed; see Incoming".into()))
        }
        ActionReq::AbortPull => {
            crate::remote::pull_abort(store)?;
            Ok(report("Merge aborted".into()))
        }
        ActionReq::Bulk {
            entries,
            drop_only,
            refused,
        } => {
            let marks = pending::marks(&pending::load(store)?);
            let tracked = source::load(&store.repo())?;
            let skip = refused.lock().expect("refused set").clone();
            let settled = |e: &BulkEntry| {
                skip.contains(&e.path)
                    || marks.contains_key(&e.rel)
                    || !matches!(
                        tracked.state_of(&e.rel, e.is_dir),
                        source::TrackState::New | source::TrackState::Untracked
                    )
            };
            let pick = |keep: &dyn Fn(&BulkEntry) -> bool| -> Vec<std::path::PathBuf> {
                entries
                    .iter()
                    .filter(|e| keep(e))
                    .map(|e| e.path.clone())
                    .collect()
            };
            let (paths, none) = if drop_only {
                (
                    pick(&|e| marks.contains_key(&e.rel)),
                    "nothing pending in this folder",
                )
            } else {
                let rest = pick(&|e| !settled(e));
                let all = if rest.is_empty() {
                    pick(&|e| !skip.contains(&e.path))
                } else {
                    rest
                };
                (all, "nothing to track here")
            };
            if paths.is_empty() {
                return Ok(report(none.into()));
            }
            let one = (paths.len() == 1).then(|| paths[0].clone());
            let mut r = match run_action(store, ActionReq::Toggle(paths)) {
                Err(e) if !crate::lock::is_busy(&e) => {
                    refused.lock().expect("refused set").extend(one);
                    return Err(e);
                }
                other => other?,
            };
            refused
                .lock()
                .expect("refused set")
                .extend(r.refused.iter().cloned());
            r.refused.clear();
            Ok(r)
        }
        ActionReq::Toggle(paths) => {
            let requester = "human";
            let mut messages = Vec::new();
            let (mut tracks, mut untracks, mut dropped) = (0, 0, 0);
            let mut refused = Vec::new();
            let mut hps = Vec::new();
            let mut origin = Vec::new();
            for p in &paths {
                match track::resolve_home_path(store, &p.to_string_lossy(), &store.dirs.home) {
                    Ok(hp) => {
                        hps.push(hp);
                        origin.push(p);
                    }
                    Err(e) => refused.push((p.clone(), format!("{e:#}"))),
                }
            }
            for (p, result) in origin
                .into_iter()
                .zip(track::toggle_many(store, &hps, requester)?)
            {
                match result {
                    Ok(t) => {
                        match (&t.added, &t.dropped) {
                            (Some(i), _) if matches!(i.action, pending::Action::Untrack { .. }) => {
                                untracks += 1
                            }
                            (Some(_), _) => tracks += 1,
                            (_, Some(_)) => dropped += 1,
                            _ => {}
                        }
                        messages.push(t.message)
                    }
                    Err(e) => refused.push((p.clone(), format!("{e:#}"))),
                }
            }
            let items = |n: usize| if n == 1 { "item" } else { "items" };
            let mut parts = Vec::new();
            if tracks > 0 {
                parts.push(format!("Added {tracks} pending {}", items(tracks)));
            }
            if dropped > 0 {
                parts.push(format!("Dropped {dropped} pending {}", items(dropped)));
            }
            if untracks > 0 {
                let s = if untracks == 1 { "untrack" } else { "untracks" };
                parts.push(format!("{untracks} {s} added"));
            }
            if !refused.is_empty() {
                parts.push(format!("{} refused", refused.len()));
            }
            let message = match (messages.len(), refused.len()) {
                (1, 0) => messages.remove(0),
                (0, 1) => return Err(anyhow::anyhow!(refused.remove(0).1)),
                _ if parts.is_empty() => "No pending changes".to_string(),
                _ => parts.join(", "),
            };
            let detail = (!refused.is_empty()).then(|| {
                let mut d = format!("{}:", message);
                for (p, why) in &refused {
                    let rel = p.strip_prefix(&store.dirs.home).unwrap_or(p).display();
                    d.push_str(&format!("\n~/{rel}: {why}"));
                }
                d
            });
            let refused: Vec<_> = refused.into_iter().map(|(p, _)| p).collect();
            Ok(ActionReport {
                merge_conflicts: false,
                message,
                results: None,
                amend_message: None,
                detail,
                refused,
            })
        }
        ActionReq::Hide(path, is_dir) => {
            let hp = track::resolve_home_path(store, &path.to_string_lossy(), &store.dirs.home)?;
            let t = track::toggle_hide(store, &hp, is_dir, "human")?;
            Ok(ActionReport {
                merge_conflicts: false,
                message: t.message,
                results: None,
                amend_message: None,
                detail: None,
                refused: Vec::new(),
            })
        }
        ActionReq::Drop(id) => {
            let lock = store.lock()?;
            let gone = pending::drop_items(store, &lock, &[id])?;
            Ok(ActionReport {
                merge_conflicts: false,
                message: format!("Dropped {}: {}", gone[0].label(), gone[0].summary),
                results: None,
                amend_message: None,
                detail: None,
                refused: Vec::new(),
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
        ActionReq::Hunks {
            path,
            side,
            sel,
            expect,
        } => {
            let d = crate::hunks::apply(store, &path, side, &sel, Some(&expect), false)?;
            Ok(report(format!(
                "{} {} changed line(s) of {}",
                if side == crate::hunks::Side::Stage {
                    "Staged"
                } else {
                    "Unstaged"
                },
                d.lines,
                d.path
            )))
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
        ActionReq::ApplyIncoming(sel) => {
            let a = changes::apply_incoming(store, sel.as_deref(), false)?;
            if !a.failed.is_empty() {
                anyhow::bail!(
                    "could not write {}: {}",
                    a.failed.join(", "),
                    a.error.unwrap_or_default()
                );
            }
            let snap = a
                .snapshot
                .map_or(String::new(), |id| format!(", snapshot {id}"));
            Ok(report(format!("Applied {} file(s){snap}", a.applied.len())))
        }
        ActionReq::Resolve(path, text) => {
            let r = crate::conflicts::resolve(
                store,
                &path,
                crate::conflicts::How::Text(text.0),
                "human",
            )?;
            Ok(report(format!("Resolved {}; staged", r.path)))
        }
        ActionReq::Undo => {
            let done = crate::undo::undo(store, false)?;
            Ok(report(format!("Undid: {}", done.summary)))
        }
        ActionReq::Fetch => {
            let f = crate::remote::fetch(store, true, None)?;
            Ok(report(match (f.ahead, f.behind) {
                (Some(a), Some(b)) => {
                    format!("Fetched {}: ahead {a}, behind {b}", f.remotes.join(", "))
                }
                _ => format!("Fetched {}", f.remotes.join(", ")),
            }))
        }
        ActionReq::Pull => {
            let p = crate::remote::pull(store, None, crate::remote::PullMode::Rebase)?;
            Ok(report(if p.commits == 0 {
                format!("Up to date with {}", p.remote)
            } else {
                format!(
                    "Pulled {} commit(s) from {}: see the Incoming tab",
                    p.commits, p.remote
                )
            }))
        }
        ActionReq::Push => {
            let done = crate::remote::push(store, &[crate::remote::default_remote(store)?])?;
            Ok(report(format!(
                "Pushed {} to {}",
                done[0].branch, done[0].remote
            )))
        }
        ActionReq::Assign(paths, edits) => {
            let mut homes = Vec::new();
            for p in &paths {
                homes.push(track::resolve_home_path(store, p, &store.dirs.home)?);
            }
            let ctx = track::Ctx::load(store)?;
            let item = crate::machines::plan_assign(store, &ctx, &homes, &edits, false, "human")?;
            let lock = store.lock()?;
            track::recheck(store, &lock, std::slice::from_ref(&item))?;
            let ctx = track::Ctx::load(store)?;
            let check = track::stale_checker(store, &ctx);
            let mut added = pending::add(store, &lock, vec![item], &check)?;
            let item = added.remove(0).item;
            Ok(report(format!("Added {}: {}", item.label(), item.summary)))
        }
        ActionReq::Variant(path, target) => {
            let hp = track::resolve_home_path(store, &path, &store.dirs.home)?;
            let ctx = track::Ctx::load(store)?;
            let item = crate::variants::plan_add(store, &ctx, &hp, &target, "human")?;
            let lock = store.lock()?;
            track::recheck(store, &lock, std::slice::from_ref(&item))?;
            let ctx = track::Ctx::load(store)?;
            let check = track::stale_checker(store, &ctx);
            let mut added = pending::add(store, &lock, vec![item], &check)?;
            let item = added.remove(0).item;
            Ok(report(format!("Added {}: {}", item.label(), item.summary)))
        }
        ActionReq::Value {
            name,
            target,
            value,
        } => {
            let ctx = track::Ctx::load(store)?;
            let item =
                crate::values::plan_set(store, &ctx, &name, target.as_deref(), &value, "human")?;
            let lock = store.lock()?;
            track::recheck(store, &lock, std::slice::from_ref(&item))?;
            let ctx = track::Ctx::load(store)?;
            let check = track::stale_checker(store, &ctx);
            let mut added = pending::add(store, &lock, vec![item], &check)?;
            let item = added.remove(0).item;
            Ok(report(format!("Added {}: {}", item.label(), item.summary)))
        }
        ActionReq::Encrypt(path) => {
            let hp = track::resolve_home_path(store, &path, &store.dirs.home)?;
            let ctx = track::Ctx::load(store)?;
            let item = crate::encfile::plan(store, &ctx, &hp, "human")?;
            let lock = store.lock()?;
            track::recheck(store, &lock, std::slice::from_ref(&item))?;
            let ctx = track::Ctx::load(store)?;
            let check = track::stale_checker(store, &ctx);
            let mut added = pending::add(store, &lock, vec![item], &check)?;
            let item = added.remove(0).item;
            Ok(report(format!("Added {}: {}", item.label(), item.summary)))
        }
        ActionReq::SecretSet {
            name,
            target,
            value,
        } => {
            crate::secrets::set(store, &name, target.as_deref(), &value.0)?;
            Ok(report(format!(
                "Secret {name} stored in the vault (staged)"
            )))
        }
        ActionReq::Unlock(pass) => {
            crate::vault::unlock(store, &pass.0)?;
            Ok(report("Vault unlocked".into()))
        }
        ActionReq::Lock => {
            crate::vault::session::end(store)?;
            Ok(report("Vault locked".into()))
        }
        ActionReq::Apply(ids) => {
            let results = track::apply_pending(store, Some(&ids))?;
            let applied = results
                .iter()
                .filter(|r| r.status == track::Outcome::Applied)
                .count();
            Ok(ActionReport {
                merge_conflicts: false,
                message: format!("Applied {applied} of {} item(s)", results.len()),
                results: Some(results),
                amend_message: None,
                detail: None,
                refused: Vec::new(),
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
