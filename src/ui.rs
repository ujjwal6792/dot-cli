use crate::{
    assignment_screen::{self, Command as AssignmentCommand},
    assignments,
    backend::{self, Filter, Row, Snapshot},
    config::{Config, atomic_write},
    dialog, git, help, migration,
    onboarding::{self, SetupKind, SetupReport},
    operation_screen::{self, View as OperationView},
    operations::{self, Action, Outcome, Plan},
    sync, syntax, toast,
};
use anyhow::{Context, Result, bail};
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind,
    },
    execute,
};
use ratatui::{prelude::*, widgets::*};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

const TOAST_MIN_VISIBLE: Duration = Duration::from_millis(800);

const UNTRACKED_TEXT: Color = Color::Rgb(160, 168, 180);
const FOCUS_BORDER: Color = Color::Rgb(97, 218, 179);
fn selection_style() -> Style {
    Style::default()
        .fg(Color::Rgb(248, 250, 252))
        .bg(Color::Rgb(55, 65, 81))
        .add_modifier(Modifier::BOLD)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pane {
    Status,
    Files,
    Git,
    Machines,
    Details,
    Activity,
}
impl Pane {
    fn next(self) -> Self {
        match self {
            Self::Status => Self::Files,
            Self::Files => Self::Git,
            Self::Git => Self::Machines,
            Self::Machines => Self::Details,
            Self::Details => Self::Activity,
            Self::Activity => Self::Status,
        }
    }
}
#[derive(Clone, Debug)]
enum Purpose {
    AssignmentMachines(String),
    Setting(crate::settings::Scope),
    Repo,
    Commit,
    Source(String),
    Switch,
    Machine,
    Recover,
}
#[derive(Clone, Debug)]
enum Control {
    Policy(String, bool),
    Switch(PathBuf),
    Recover(String),
    Service,
    Packages,
}
#[derive(Clone, Debug)]
enum Modal {
    Help(help::View),
    Setup,
    Busy {
        label: String,
    },
    Report {
        title: String,
        text: String,
        scroll: u16,
    },
    Welcome {
        report: SetupReport,
        scroll: u16,
    },
    Confirm {
        command: Control,
        text: String,
        scroll: u16,
    },
    Menu(usize),
    Input {
        purpose: Purpose,
        text: String,
    },
    Review {
        plan: Plan,
        view: OperationView,
    },
    Outcome {
        outcome: Outcome,
        view: OperationView,
    },
    Text {
        title: String,
        text: String,
        scroll: u16,
    },
}
enum Work {
    NothingToRemove(String),
    ConfigMessage(Config, String),
    Welcome(Config, SetupReport),
    Snapshot(Snapshot),
    Directory {
        directory: String,
        entries: BTreeSet<String>,
    },
    Preview {
        key: String,
        text: String,
    },
    Plan(Plan),
    Applied(Config, Outcome),
    Message(String),
}
struct Job {
    rx: Receiver<Result<Work>>,
    label: String,
    kind: JobKind,
    discard: bool,
    started: Instant,
    toast_started: Option<Instant>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum JobKind {
    Operation,
    Preparation,
    Inspection,
    Preview,
}
#[derive(Default)]
struct Areas {
    status: Rect,
    files: Rect,
    git: Rect,
    machines: Rect,
    details: Rect,
    activity: Rect,
    modal: Rect,
    footer: Vec<(Rect, KeyCode)>,
    setup: Vec<(Rect, KeyCode)>,
    welcome_scroll_max: u16,
    modal_scroll_max: u16,
    modal_controls: Vec<(Rect, KeyCode)>,
    modal_body: Rect,
    operation: operation_screen::Areas,
    help: help::Areas,
    assignment: assignment_screen::Areas,
    toast: toast::Areas,
    tabs: Vec<(Rect, Pane, usize)>,
}

pub struct App {
    pub config: Config,
    pub config_path: PathBuf,
    pub pane: Pane,
    detail_owner: Pane,
    pub snapshot: Snapshot,
    pub directory: String,
    pub filter: Filter,
    pub query: String,
    pub searching: bool,
    pub cursor: usize,
    pub git_cursor: usize,
    pub selected: BTreeSet<String>,
    pub git_selected: BTreeSet<String>,
    pub detail: String,
    pub notice: String,
    rows: Vec<Row>,
    file_state: ListState,
    git_state: ListState,
    menu_state: ListState,
    modal: Option<Modal>,
    pending_review: Option<Plan>,
    job: Option<Job>,
    preview_key: String,
    scroll: u16,
    expanded: bool,
    status_scroll: u16,
    machines_scroll: u16,
    activity_scroll: u16,
    history_view: bool,
    detail_tab: usize,
    status_tab: usize,
    machines_tab: usize,
    activity_tab: usize,
    areas: Areas,
    last_refresh: Instant,
    refresh_failed: bool,
    last_click: Option<(String, Instant)>,
    input_cursor: usize,
    external: bool,
    editor_expanded: bool,
    assignment_drafts: BTreeMap<String, assignments::Scope>,
}
impl App {
    pub fn new(config: Config, path: PathBuf) -> Self {
        let mut app = Self {
            config,
            config_path: path,
            pane: Pane::Files,
            detail_owner: Pane::Files,
            snapshot: Snapshot::default(),
            directory: String::new(),
            filter: Filter::All,
            query: String::new(),
            searching: false,
            cursor: 0,
            git_cursor: 0,
            selected: BTreeSet::new(),
            git_selected: BTreeSet::new(),
            detail: String::new(),
            notice: "Connecting to chezmoi…".into(),
            rows: vec![],
            file_state: ListState::default(),
            git_state: ListState::default(),
            menu_state: ListState::default(),
            modal: None,
            pending_review: None,
            job: None,
            preview_key: String::new(),
            scroll: 0,
            expanded: false,
            status_scroll: 0,
            machines_scroll: 0,
            activity_scroll: 0,
            history_view: false,
            detail_tab: 0,
            status_tab: 0,
            machines_tab: 0,
            activity_tab: 0,
            areas: Areas::default(),
            last_refresh: Instant::now(),
            refresh_failed: false,
            last_click: None,
            input_cursor: 0,
            external: false,
            editor_expanded: false,
            assignment_drafts: BTreeMap::new(),
        };
        // Local browsing must not depend on a successful chezmoi/Git inspection.
        // The native snapshot enriches these rows when it becomes available.
        app.rebuild();
        app
    }
    fn launch(&mut self, label: &str, f: impl FnOnce() -> Result<Work> + Send + 'static) {
        self.launch_job(JobKind::Operation, label, f);
    }
    fn launch_job(
        &mut self,
        kind: JobKind,
        label: &str,
        f: impl FnOnce() -> Result<Work> + Send + 'static,
    ) {
        if self.job.is_some() {
            self.notice = "An operation is already running".into();
            return;
        }
        if matches!(kind, JobKind::Operation | JobKind::Preparation) {
            self.pending_review = None;
            self.searching = false;
            self.modal = if kind == JobKind::Operation {
                Some(Modal::Busy {
                    label: label.into(),
                })
            } else {
                None
            };
        }
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        self.job = Some(Job {
            rx,
            label: label.into(),
            kind,
            discard: false,
            started: Instant::now(),
            toast_started: None,
        });
    }
    fn refresh(&mut self) {
        let c = self.config.clone();
        let directory = self.directory.clone();
        self.launch_job(
            JobKind::Inspection,
            "Inspecting chezmoi and Git",
            move || {
                let mut snapshot = backend::snapshot(&c)?;
                if !directory.is_empty() {
                    backend::inspect_directory(&c, &mut snapshot, &directory)?;
                }
                Ok(Work::Snapshot(snapshot))
            },
        );
    }
    fn rebuild(&mut self) {
        match backend::rows(
            &self.config,
            &self.snapshot,
            &self.directory,
            self.filter,
            &self.query,
        ) {
            Ok(r) => self.rows = r,
            Err(e) => self.notice = e.to_string(),
        };
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
        self.file_state.select(if self.rows.is_empty() {
            None
        } else {
            Some(self.cursor)
        });
    }
    fn pump(&mut self) {
        let received = self.job.as_ref().and_then(|j| {
            // Keep a visible status readable without sleeping or delaying the worker.
            // Cancellation bypasses the minimum and discards the queued result.
            if !j.discard
                && (j
                    .toast_started
                    .is_some_and(|shown| shown.elapsed() < TOAST_MIN_VISIBLE)
                    || j.kind == JobKind::Preparation && j.toast_started.is_none())
            {
                return None;
            }
            match j.rx.try_recv() {
                Ok(r) => Some(r),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err(anyhow::anyhow!("Worker stopped unexpectedly")))
                }
                Err(_) => None,
            }
        });
        if let Some(result) = received {
            let completed = self.job.take().unwrap();
            let kind = completed.kind;
            if kind == JobKind::Inspection {
                // Failed inspection must not turn dismissal into an immediate retry.
                // Keep the workspace available; R or setup explicitly retries.
                self.last_refresh = Instant::now();
                self.refresh_failed = result.is_err();
            }
            // A cancelled preparation may finish later; it must never reopen a dialog.
            if !completed.discard {
                match result {
                    Ok(Work::NothingToRemove(text)) => {
                        self.notice = "Nothing to remove".into();
                        self.modal = Some(Modal::Text {
                            title: "Nothing to remove".into(),
                            text,
                            scroll: 0,
                        });
                    }
                    Ok(Work::Snapshot(s)) => {
                        self.assignment_drafts.retain(|path, draft| {
                            s.assignments.get(path).cloned().unwrap_or_default() != *draft
                        });
                        self.snapshot = s;
                        self.rebuild();
                        self.preview_key.clear();
                        self.last_refresh = Instant::now();
                        self.notice = "Ready · HOME, source and Git state are separate".into();
                    }
                    Ok(Work::Directory { directory, entries }) => {
                        self.snapshot
                            .directory_entries
                            .insert(directory.clone(), entries);
                        if directory == self.directory {
                            self.rebuild();
                            self.preview_key.clear();
                        }
                    }
                    Ok(Work::Preview { key, text }) => {
                        if key == self.current_key() {
                            self.detail = text;
                            self.scroll = 0;
                        }
                    }
                    Ok(Work::Plan(p)) => {
                        self.pending_review = Some(p);
                    }
                    Ok(Work::Applied(c, r)) => {
                        self.config = c;
                        self.notice = format!(
                            "{} updated · {} failed · {} not attempted. {}",
                            r.succeeded.len(),
                            usize::from(r.failed.is_some()),
                            r.not_attempted.len(),
                            r.message
                        );
                        self.modal = Some(Modal::Outcome {
                            outcome: r,
                            view: OperationView::default(),
                        });
                        self.selected.clear();
                        self.git_selected.clear();
                        self.refresh();
                    }
                    Ok(Work::ConfigMessage(c, t)) => {
                        if self.config.source != c.source {
                            self.directory.clear();
                            self.assignment_drafts.clear();
                        }
                        self.config = c;
                        self.notice = t.clone();
                        self.modal = Some(Modal::Report {
                            title: "Result".into(),
                            text: t,
                            scroll: 0,
                        });
                        self.refresh();
                    }
                    Ok(Work::Welcome(c, report)) => {
                        self.config = c;
                        self.directory.clear();
                        self.modal = Some(Modal::Welcome { report, scroll: 0 });
                        self.refresh();
                    }
                    Ok(Work::Message(t)) => {
                        self.modal = Some(Modal::Report {
                            title: "Result".into(),
                            text: t,
                            scroll: 0,
                        });
                        self.refresh();
                    }
                    Err(e) => {
                        self.notice = e.to_string();
                        if matches!(kind, JobKind::Inspection | JobKind::Preview)
                            && self.modal.is_some()
                        {
                            return;
                        }
                        self.modal = Some(
                            if kind == JobKind::Inspection && !self.config.source.exists() {
                                Modal::Setup
                            } else if matches!(kind, JobKind::Operation | JobKind::Preparation) {
                                Modal::Report {
                                    title: "Needs attention".into(),
                                    text: format!("{e:#}"),
                                    scroll: 0,
                                }
                            } else {
                                Modal::Text {
                                    title: "Needs attention".into(),
                                    text: format!("{e:#}"),
                                    scroll: 0,
                                }
                            },
                        );
                    }
                }
            }
        }
        // A nonmodal preparation must not interrupt typing or replace another form.
        if self.modal.is_none()
            && !self.searching
            && let Some(plan) = self.pending_review.take()
        {
            self.modal = Some(Modal::Review {
                plan,
                view: OperationView::default(),
            });
        }
        if self.job.is_none()
            && self.modal.is_none()
            && !self.refresh_failed
            && self.last_refresh.elapsed() > Duration::from_secs(15)
        {
            self.refresh();
        }
        if self.job.is_none()
            && self.modal.is_none()
            && !self.refresh_failed
            && !self.snapshot.observed_at.is_empty()
            && !self
                .snapshot
                .directory_entries
                .contains_key(&self.directory)
        {
            let c = self.config.clone();
            let mut snapshot = self.snapshot.clone();
            let directory = self.directory.clone();
            self.launch_job(
                JobKind::Inspection,
                "Checking folder ignore rules",
                move || {
                    backend::inspect_directory(&c, &mut snapshot, &directory)?;
                    let entries = snapshot
                        .directory_entries
                        .remove(&directory)
                        .unwrap_or_default();
                    Ok(Work::Directory { directory, entries })
                },
            );
        }
        if self.job.is_none() && self.modal.is_none() {
            let key = self.current_key();
            if key != self.preview_key {
                self.preview_key = key.clone();
                let c = self.config.clone();
                let mut s = self.snapshot.clone();
                let pane = self.context();
                let tab = self.detail_tab;
                if pane == Pane::Files && tab == 2 {
                    return;
                }
                let selected = self.rows.get(self.cursor).cloned();
                let gf = self.snapshot.git.files.get(self.git_cursor).cloned();
                let hist = self.history_view;
                self.launch_job(JobKind::Preview,"Loading selected details",move||{
                    let text=if pane==Pane::Machines{format!("LAST REPORTED · not live status\n\n{}\n\nLocal automatic scope: {} files\nTrust: {}\nReports are summary metadata; detailed activity stays local.",serde_json::to_string_pretty(&sync::cached_reports(&c))?,c.enabled.len(),if c.trusted{"trusted repository"}else{"strict"})}
                    else if pane==Pane::Status{format!("Machine: {}\nHOME: {}\nSource: {}\nObserved: {}\n\nGit: {}\nUpstream: {}\n↑{} outgoing · ↓{} incoming (last fetched)\n{}\n\nHOME differences: {}\nSource changes: {}\nAutomatic sync: {}\nReports: {}",c.machine,c.home.display(),c.source.display(),s.observed_at,s.git.branch,s.git.upstream,s.git.ahead,s.git.behind,s.git.error.clone().unwrap_or_default(),s.changes.len(),s.git.files.len(),c.sync_enabled,c.reporting)}
                    else if pane==Pane::Activity{serde_json::to_string_pretty(&operations::history(&c)?)?}
                    else if pane==Pane::Git{if hist{s.git.commits.join("\n")}else if let Some(f)=gf{format!("Source ↔ Git · {}\n\nUNSTAGED\n{}\nSTAGED\n{}",f.path,git::diff(&c,std::slice::from_ref(&f.path),false)?,git::diff(&c,std::slice::from_ref(&f.path),true)?)}else{"No source changes. Commit history: press h in Git.".into()}}
                    else if let Some(row)=selected{file_detail(&c, &mut s, &row, tab)?}
                    else{"Select a file or source change.".into()};Ok(Work::Preview{key,text})
                });
            }
        }
    }
    fn context(&self) -> Pane {
        if self.pane == Pane::Details {
            self.detail_owner
        } else {
            self.pane
        }
    }
    fn assignment_scope(&self, path: &str) -> assignments::Scope {
        self.assignment_drafts
            .get(path)
            .or_else(|| self.snapshot.assignments.get(path))
            .cloned()
            .unwrap_or_default()
    }
    fn open_assignment(&mut self) {
        if self.context() != Pane::Files {
            self.notice = "Choose a file or folder in Files to assign".into();
            return;
        }
        self.pane = Pane::Details;
        self.detail_owner = Pane::Files;
        self.set_tab(Pane::Details, 2);
        self.scroll = 0;
    }
    fn assignment_command(&mut self, command: AssignmentCommand) {
        let Some(row) = self.rows.get(self.cursor).cloned() else {
            return;
        };
        self.pane = Pane::Details;
        self.detail_owner = Pane::Files;
        if !row.managed {
            self.notice = "Capture this file or folder before assigning it".into();
            return;
        }
        if self.snapshot.identity.hostname.is_empty() {
            self.notice = "Wait for machine inspection to finish".into();
            return;
        }
        let mut scope = self.assignment_scope(&row.path);
        match command {
            AssignmentCommand::Os(os) => scope.os = os,
            AssignmentCommand::AllMachines => scope.machines.clear(),
            AssignmentCommand::ToggleMachine(name) => {
                if !scope.machines.remove(&name) {
                    scope.machines.insert(name);
                }
            }
            AssignmentCommand::ThisMachine => {
                scope.machines = BTreeSet::from([self.snapshot.identity.hostname.clone()])
            }
            AssignmentCommand::CustomMachines => {
                let text = scope
                    .machines
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                self.input_cursor = text.chars().count();
                self.modal = Some(Modal::Input {
                    purpose: Purpose::AssignmentMachines(row.path),
                    text,
                });
                return;
            }
            AssignmentCommand::Reset => {
                self.assignment_drafts.remove(&row.path);
                self.notice = "Assignment edits reset".into();
                return;
            }
            AssignmentCommand::Review => {
                if scope
                    == self
                        .snapshot
                        .assignments
                        .get(&row.path)
                        .cloned()
                        .unwrap_or_default()
                {
                    self.notice = "This assignment is already saved".into();
                    return;
                }
                if self.job.is_some() {
                    self.notice = "Wait for the current operation to finish".into();
                    return;
                }
                let c = self.config.clone();
                let paths = vec![row.path];
                self.launch_job(
                    JobKind::Preparation,
                    "Reviewing OS and machine assignments",
                    move || {
                        operations::plan(
                            &c,
                            Action::Assign,
                            &paths,
                            Some(serde_json::to_string(&scope)?),
                        )
                        .map(Work::Plan)
                    },
                );
                return;
            }
        }
        self.assignment_drafts.insert(row.path, scope);
        self.notice = "Assignment edited · s review and save · u reset".into();
    }
    fn current_key(&self) -> String {
        format!(
            "{:?}|{}|{}|{}|{}",
            self.context(),
            self.directory,
            if self.context() == Pane::Git {
                self.git_cursor
            } else {
                self.cursor
            },
            self.detail_tab,
            self.history_view
        )
    }
    fn paths(&self, action: Action) -> Vec<String> {
        if action.files() || (action == Action::Ignore && self.context() != Pane::Git) {
            if self.selected.is_empty() {
                self.rows
                    .get(self.cursor)
                    .map(|r| vec![r.path.clone()])
                    .unwrap_or_default()
            } else {
                self.selected.iter().cloned().collect()
            }
        } else if self.git_selected.is_empty() {
            self.snapshot
                .git
                .files
                .get(self.git_cursor)
                .map(|r| vec![r.path.clone()])
                .unwrap_or_default()
        } else {
            self.git_selected.iter().cloned().collect()
        }
    }
    fn action(&mut self, action: Action, payload: Option<String>) {
        if self.job.is_some() {
            self.notice = "Wait for the current operation to finish".into();
            return;
        }
        let c = self.config.clone();
        let paths = if matches!(
            action,
            Action::Commit
                | Action::Fetch
                | Action::Pull
                | Action::Push
                | Action::Setting
                | Action::IgnoreDefaults
        ) {
            vec![]
        } else {
            self.paths(action)
        };
        let payload = if action == Action::Ignore && self.context() == Pane::Git {
            Some("source".into())
        } else {
            payload
        };
        self.launch_job(JobKind::Preparation, action.label(), move || {
            match operations::plan(&c, action, &paths, payload) {
                Err(error) if action == Action::Forget && error.is::<backend::NoEligibleFiles>() => {
                    let subject = match paths.as_slice() {
                        [path] if c.home.join(path).is_dir() => "This folder",
                        [_] => "This file",
                        _ => "This selection",
                    };
                    Ok(Work::NothingToRemove(format!(
                        "{subject} contains no files tracked by dot.\nYour original files are unchanged."
                    )))
                }
                result => result.map(Work::Plan),
            }
        });
    }
    fn tab_index(&self, pane: Pane) -> usize {
        match pane {
            Pane::Files => match self.filter {
                Filter::All => 0,
                Filter::Managed => 1,
                Filter::Changed => 2,
            },
            Pane::Git => usize::from(self.history_view),
            Pane::Details => self.detail_tab,
            Pane::Status => self.status_tab,
            Pane::Machines => self.machines_tab,
            Pane::Activity => self.activity_tab,
        }
    }
    fn set_tab(&mut self, pane: Pane, index: usize) {
        if index >= pane_tabs(pane).len() || index == self.tab_index(pane) {
            return;
        }
        match pane {
            Pane::Files => {
                self.filter = [Filter::All, Filter::Managed, Filter::Changed][index];
                self.cursor = 0;
                self.file_state = ListState::default();
                self.rebuild();
            }
            Pane::Git => {
                self.history_view = index == 1;
                self.git_cursor = 0;
                self.git_state = ListState::default();
                let count = if self.history_view {
                    self.snapshot.git.commits.len()
                } else {
                    self.snapshot.git.files.len()
                };
                self.git_state.select((count > 0).then_some(0));
            }
            Pane::Details => self.detail_tab = index,
            Pane::Status => self.status_tab = index,
            Pane::Machines => self.machines_tab = index,
            Pane::Activity => self.activity_tab = index,
        }
        match pane {
            Pane::Status => self.status_scroll = 0,
            Pane::Machines => self.machines_scroll = 0,
            Pane::Activity => self.activity_scroll = 0,
            Pane::Details => self.scroll = 0,
            _ => {}
        }
        self.preview_key.clear();
    }
    fn cycle_tab(&mut self, delta: isize) {
        let count = pane_tabs(self.pane).len();
        let next = (self.tab_index(self.pane) as isize + delta).rem_euclid(count as isize) as usize;
        self.set_tab(self.pane, next);
    }
    fn move_cursor(&mut self, delta: isize) {
        self.move_pane_cursor(self.pane, delta);
    }
    fn move_pane_cursor(&mut self, pane: Pane, delta: isize) {
        if pane == Pane::Git {
            let n = if self.history_view {
                self.snapshot.git.commits.len()
            } else {
                self.snapshot.git.files.len()
            };
            self.git_cursor = self
                .git_cursor
                .saturating_add_signed(delta)
                .min(n.saturating_sub(1));
            self.git_state.select(Some(self.git_cursor));
        } else if pane == Pane::Files {
            self.cursor = self
                .cursor
                .saturating_add_signed(delta)
                .min(self.rows.len().saturating_sub(1));
            self.file_state.select(Some(self.cursor));
        } else {
            let scroll = match pane {
                Pane::Status => &mut self.status_scroll,
                Pane::Machines => &mut self.machines_scroll,
                Pane::Activity => &mut self.activity_scroll,
                _ => &mut self.scroll,
            };
            *scroll = scroll
                .saturating_add_signed(delta.clamp(i16::MIN as isize, i16::MAX as isize) as i16);
        }
    }
    fn enter_dir(&mut self) {
        if let Some(r) = self.rows.get(self.cursor)
            && r.directory
        {
            self.directory = r.path.clone();
            self.cursor = 0;
            self.query.clear();
            self.rebuild();
            self.preview_key.clear();
        }
    }
    fn parent(&mut self) {
        let old = self.directory.clone();
        self.directory = Path::new(&self.directory)
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.query.clear();
        self.rebuild();
        if let Some(i) = self.rows.iter().position(|r| r.path == old) {
            self.cursor = i;
            self.file_state.select(Some(i));
        }
    }
    fn request_quit(&mut self) -> bool {
        if self
            .job
            .as_ref()
            .is_none_or(|j| j.kind != JobKind::Operation)
        {
            return true;
        }
        self.notice = "An operation is running; wait for its result before quitting".into();
        false
    }
    pub fn key(&mut self, key: KeyEvent) -> bool {
        if key.kind == KeyEventKind::Release {
            return false;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return self.request_quit();
        }
        if self.modal.is_some() {
            if key.code == KeyCode::Char('q')
                && matches!(
                    self.modal,
                    Some(
                        Modal::Help(_)
                            | Modal::Text { .. }
                            | Modal::Report { .. }
                            | Modal::Setup
                            | Modal::Welcome { .. }
                            | Modal::Outcome { .. }
                    )
                )
            {
                return self.request_quit();
            }
            self.modal_key(key);
            return false;
        }
        if key.code == KeyCode::Esc && self.pending_review.take().is_some() {
            self.notice = "Review cancelled".into();
            return false;
        }
        if key.code == KeyCode::Esc
            && let Some(job) = &mut self.job
            && job.kind == JobKind::Preparation
            && !job.discard
        {
            job.discard = true;
            self.notice = "Review cancelled".into();
            return false;
        }
        // Inputs own character keys before every global shortcut, including q and panel numbers.
        if self.searching {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => self.searching = false,
                KeyCode::Backspace => {
                    self.query.pop();
                }
                KeyCode::Char(c) => self.query.push(c),
                _ => {}
            }
            self.rebuild();
            return false;
        }
        if self.pane == Pane::Details && self.context() == Pane::Files && self.detail_tab == 2 {
            let command = match key.code {
                KeyCode::Char('o') => self
                    .rows
                    .get(self.cursor)
                    .map(|row| AssignmentCommand::Os(self.assignment_scope(&row.path).os.next())),
                KeyCode::Char('m') => self.rows.get(self.cursor).map(|row| {
                    if self.assignment_scope(&row.path).machines.is_empty() {
                        AssignmentCommand::ThisMachine
                    } else {
                        AssignmentCommand::AllMachines
                    }
                }),
                KeyCode::Char('M') => Some(AssignmentCommand::CustomMachines),
                KeyCode::Char('s') => Some(AssignmentCommand::Review),
                KeyCode::Char('u') => Some(AssignmentCommand::Reset),
                _ => None,
            };
            if let Some(command) = command {
                self.assignment_command(command);
                return false;
            }
        }
        match key.code {
            KeyCode::Char('q') => {
                return self.request_quit();
            }
            KeyCode::Char('1') => self.pane = Pane::Status,
            KeyCode::Char('2') => self.pane = Pane::Files,
            KeyCode::Char('3') => self.pane = Pane::Git,
            KeyCode::Char('4') => self.pane = Pane::Machines,
            KeyCode::Char('0') => self.pane = Pane::Details,
            KeyCode::Tab => self.pane = self.pane.next(),
            KeyCode::Char('z') => self.expanded = !self.expanded,
            KeyCode::Char('s') if self.context() == Pane::Files => self.open_assignment(),
            KeyCode::Esc => {
                self.expanded = false;
                self.pane = Pane::Files;
            }
            KeyCode::Char('?') => {
                self.modal = Some(Modal::Help(help::View::new(match self.context() {
                    Pane::Files => 1,
                    Pane::Git => 2,
                    _ => 0,
                })))
            }
            KeyCode::Char(':') => self.modal = Some(Modal::Menu(0)),
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => self.refresh(),
            KeyCode::Char('/') => {
                self.pane = Pane::Files;
                self.searching = true;
            }
            KeyCode::Char('f') if self.pane == Pane::Files => {
                self.cycle_tab(1);
            }
            KeyCode::Char('t') if self.pane == Pane::Files => {
                let index = usize::from(self.filter != Filter::Managed);
                self.set_tab(Pane::Files, index);
            }
            KeyCode::Down | KeyCode::Char('j') => self.move_cursor(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_cursor(-1),
            KeyCode::PageDown => self.move_cursor(10),
            KeyCode::PageUp => self.move_cursor(-10),
            KeyCode::Home => self.move_cursor(-100000),
            KeyCode::End => self.move_cursor(100000),
            KeyCode::Right | KeyCode::Char('l') if self.pane == Pane::Files => self.enter_dir(),
            KeyCode::Left | KeyCode::Char('h') if self.pane == Pane::Files => self.parent(),
            KeyCode::Char('h') if self.pane == Pane::Git => {
                self.cycle_tab(1);
            }
            KeyCode::Enter => {
                if self.pane == Pane::Files {
                    self.pane = Pane::Details;
                } else if self.pane == Pane::Status || self.pane == Pane::Machines {
                    self.modal = Some(Modal::Menu(0));
                }
            }
            KeyCode::Char(' ') if self.pane == Pane::Files => {
                if let Some(r) = self.rows.get(self.cursor)
                    && !self.selected.remove(&r.path)
                {
                    self.selected.insert(r.path.clone());
                }
            }
            KeyCode::Char(' ') if self.pane == Pane::Git && !self.history_view => {
                if let Some(f) = self.snapshot.git.files.get(self.git_cursor) {
                    let action = if f.index != ' ' && f.index != '?' {
                        Action::Unstage
                    } else {
                        Action::Stage
                    };
                    self.action(action, None);
                }
            }
            KeyCode::Char('v') if self.pane == Pane::Git && !self.history_view => {
                if let Some(f) = self.snapshot.git.files.get(self.git_cursor)
                    && !self.git_selected.remove(&f.path)
                {
                    self.git_selected.insert(f.path.clone());
                }
            }
            KeyCode::Char('c') if self.pane == Pane::Git => {
                self.modal = Some(Modal::Input {
                    purpose: Purpose::Commit,
                    text: String::new(),
                })
            }
            KeyCode::Char('c') if self.pane == Pane::Files => self.action(Action::Capture, None),
            KeyCode::Char('i') if self.pane == Pane::Files => self.action(Action::Ignore, None),
            KeyCode::Char('i') if self.pane == Pane::Git && !self.history_view => {
                self.action(Action::Ignore, None)
            }
            KeyCode::Char('a') if self.pane == Pane::Files => self.action(Action::Apply, None),
            KeyCode::Char('R') if self.pane == Pane::Files => self.action(Action::Restore, None),
            KeyCode::Char('r') if self.pane == Pane::Files => self.action(Action::Forget, None),
            KeyCode::Char('d') | KeyCode::Delete if self.pane == Pane::Files => {
                self.action(Action::Trash, None)
            }
            KeyCode::Char('d') if self.pane == Pane::Git && !self.history_view => {
                self.action(Action::Discard, None)
            }
            KeyCode::Char('F') => self.action(Action::Fetch, None),
            KeyCode::Char('L') => self.action(Action::Pull, None),
            KeyCode::Char('P') => self.action(Action::Push, None),
            KeyCode::Char('e') if self.pane == Pane::Files => self.edit_source(),
            KeyCode::Char(']') => {
                self.cycle_tab(1);
            }
            KeyCode::Char('[') => {
                self.cycle_tab(-1);
            }
            _ => {}
        }
        if self.pane != Pane::Details {
            self.detail_owner = self.pane;
        }
        false
    }
    fn edit_source(&mut self) {
        self.editor_expanded = false;
        if let Some(row) = self.rows.get(self.cursor)
            && let Some(m) = self.snapshot.managed.get(&row.path)
        {
            match fs::read_to_string(&m.source_absolute) {
                Ok(text) => {
                    self.selected.clear();
                    self.selected.insert(row.path.clone());
                    self.modal = Some(Modal::Input {
                        purpose: Purpose::Source(row.path.clone()),
                        text,
                    });
                    self.input_cursor = 0;
                }
                Err(e) => self.notice = e.to_string(),
            }
        }
    }
    fn modal_key(&mut self, key: KeyEvent) {
        // Keyboard/scroll interactions break a pending menu double-click.
        self.last_click = None;
        let modal = self.modal.take().unwrap();
        match modal {
            Modal::Busy { label } => {
                self.modal = Some(Modal::Busy { label });
            }
            Modal::Welcome { report, mut scroll } => {
                match key.code {
                    KeyCode::Enter | KeyCode::Esc => {
                        self.pane = Pane::Files;
                        self.detail_owner = Pane::Files;
                        return;
                    }
                    KeyCode::Char('d') => {
                        self.modal = Some(Modal::Text {
                            title: "Setup details".into(),
                            text: report.message(&self.config.source),
                            scroll: 0,
                        });
                        return;
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        scroll = scroll.saturating_add(1).min(self.areas.welcome_scroll_max)
                    }
                    KeyCode::Up | KeyCode::Char('k') => scroll = scroll.saturating_sub(1),
                    _ => {}
                }
                self.modal = Some(Modal::Welcome { report, scroll });
            }
            Modal::Setup => match key.code {
                KeyCode::Char('i') => self.menu_action(26),
                KeyCode::Char('c') => {
                    self.input_cursor = 0;
                    self.menu_action(27);
                }
                KeyCode::Char('o') => {
                    self.input_cursor = 0;
                    self.menu_action(21);
                }
                KeyCode::Esc | KeyCode::Enter => {}
                _ => self.modal = Some(Modal::Setup),
            },
            Modal::Confirm {
                command,
                text,
                mut scroll,
            } => {
                if matches!(key.code, KeyCode::Char('y') | KeyCode::Enter) {
                    self.confirm_control(command);
                } else if !matches!(key.code, KeyCode::Esc | KeyCode::Char('n')) {
                    let mut view = OperationView { tab: 0, scroll };
                    view.key(key.code, self.areas.modal_scroll_max);
                    scroll = view.scroll;
                    self.modal = Some(Modal::Confirm {
                        command,
                        text,
                        scroll,
                    });
                }
            }
            Modal::Input { purpose, mut text } => {
                if matches!(purpose, Purpose::Source(_)) && key.code == KeyCode::F(2) {
                    self.editor_expanded = !self.editor_expanded;
                    self.modal = Some(Modal::Input { purpose, text });
                    return;
                }
                if key.code == KeyCode::Esc {
                    return;
                }
                let submit = (key.code == KeyCode::Enter
                    && (!matches!(purpose, Purpose::Source(_))
                        || key.modifiers.contains(KeyModifiers::CONTROL)))
                    || (matches!(purpose, Purpose::Source(_))
                        && key.code == KeyCode::Char('s')
                        && key.modifiers.contains(KeyModifiers::CONTROL));
                if submit {
                    match purpose {
                        Purpose::AssignmentMachines(path) => {
                            match assignments::Scope::parse_machines(&text) {
                                Ok(machines) => {
                                    let mut scope = self.assignment_scope(&path);
                                    scope.machines = machines;
                                    self.assignment_drafts.insert(path, scope);
                                    self.notice =
                                        "Machine choices updated · s review and save".into();
                                }
                                Err(e) => {
                                    self.notice = e.to_string();
                                    self.modal = Some(Modal::Input {
                                        purpose: Purpose::AssignmentMachines(path),
                                        text,
                                    });
                                }
                            }
                        }
                        Purpose::Setting(scope) => match text.split_once('=') {
                            Some((key, value)) => {
                                match crate::settings::identity(&self.config, scope) {
                                    Ok(identity) => {
                                        let change = crate::settings::Change {
                                            key: key.into(),
                                            scope,
                                            value: Some(value.into()),
                                            identity,
                                        };
                                        self.action(
                                            Action::Setting,
                                            Some(serde_json::to_string(&change).unwrap()),
                                        );
                                    }
                                    Err(e) => self.notice = e.to_string(),
                                }
                            }
                            None => self.notice = "Use key=value".into(),
                        },
                        Purpose::Repo => {
                            let c = self.config.clone();
                            let p = self.config_path.clone();
                            self.launch("Initializing source", move || {
                                let report = initialize(&c, &p, Some(&text))?;
                                Ok(Work::Welcome(c, report))
                            });
                        }
                        Purpose::Commit => self.action(Action::Commit, Some(text)),
                        Purpose::Source(_) => self.action(Action::EditSource, Some(text)),
                        Purpose::Switch => {
                            self.modal = Some(Modal::Confirm {
                                scroll: 0,
                                command: Control::Switch(PathBuf::from(&text)),
                                text: format!(
                                    "Archive the existing source at {} and connect {}. HOME stays unchanged. Local automatic scope and trust will be reset.",
                                    self.config.source.display(),
                                    text
                                ),
                            });
                        }
                        Purpose::Machine => {
                            self.config.machine = text;
                            match self.config.save(&self.config_path) {
                                Ok(_) => self.notice = "Machine label saved".into(),
                                Err(e) => self.notice = e.to_string(),
                            }
                        }
                        Purpose::Recover => {
                            self.modal = Some(Modal::Confirm {
                                scroll: 0,
                                command: Control::Recover(text.clone()),
                                text: format!(
                                    "Restore backed-up files for {text}. A changed state blocks recovery; arbitrary script effects are not reversed."
                                ),
                            });
                        }
                    }
                    return;
                }
                let mut chars: Vec<char> = text.chars().collect();
                self.input_cursor = self.input_cursor.min(chars.len());
                match key.code {
                    KeyCode::Left => self.input_cursor = self.input_cursor.saturating_sub(1),
                    KeyCode::Right => self.input_cursor = (self.input_cursor + 1).min(chars.len()),
                    KeyCode::Up => {
                        self.input_cursor = vertical_cursor(&chars, self.input_cursor, -1)
                    }
                    KeyCode::Down => {
                        self.input_cursor = vertical_cursor(&chars, self.input_cursor, 1)
                    }
                    KeyCode::Home => self.input_cursor = 0,
                    KeyCode::End => self.input_cursor = chars.len(),
                    KeyCode::Backspace => {
                        if self.input_cursor > 0 {
                            self.input_cursor -= 1;
                            chars.remove(self.input_cursor);
                        }
                    }
                    KeyCode::Delete => {
                        if self.input_cursor < chars.len() {
                            chars.remove(self.input_cursor);
                        }
                    }
                    KeyCode::Enter => {
                        chars.insert(self.input_cursor, '\n');
                        self.input_cursor += 1;
                    }
                    KeyCode::Char(ch) => {
                        chars.insert(self.input_cursor, ch);
                        self.input_cursor += 1;
                    }
                    _ => {}
                }
                text = chars.into_iter().collect();
                self.modal = Some(Modal::Input { purpose, text });
            }
            Modal::Review { plan, mut view } => match key.code {
                KeyCode::Esc | KeyCode::Char('n') => {}
                KeyCode::Char('y') | KeyCode::Enter => {
                    let mut c = self.config.clone();
                    let path = self.config_path.clone();
                    self.launch("Executing reviewed operation", move || {
                        let result = operations::execute(&mut c, &path, &plan, &plan.id)?;
                        Ok(Work::Applied(c, result))
                    });
                }
                _ => {
                    view.key(key.code, self.areas.operation.max_scroll);
                    self.modal = Some(Modal::Review { plan, view });
                }
            },
            Modal::Outcome { outcome, mut view } => {
                match key.code {
                    KeyCode::Enter | KeyCode::Esc => return,
                    KeyCode::Char('g') if outcome.action != Action::Trash => {
                        self.pane = Pane::Git;
                        self.detail_owner = Pane::Git;
                        self.history_view = false;
                        return;
                    }
                    _ => view.key(key.code, self.areas.operation.max_scroll),
                }
                self.modal = Some(Modal::Outcome { outcome, view });
            }
            Modal::Help(mut view) => {
                if !view.key(key.code, &self.areas.help) {
                    self.modal = Some(Modal::Help(view));
                }
            }
            Modal::Text { .. } | Modal::Report { .. } => {
                let report = matches!(modal, Modal::Report { .. });
                let (title, text, mut scroll) = match modal {
                    Modal::Text {
                        title,
                        text,
                        scroll,
                    }
                    | Modal::Report {
                        title,
                        text,
                        scroll,
                    } => (title, text, scroll),
                    _ => unreachable!(),
                };
                if matches!(key.code, KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q')) {
                    return;
                }
                scroll = match key.code {
                    KeyCode::Down | KeyCode::Char('j') => scroll.saturating_add(1),
                    KeyCode::Up | KeyCode::Char('k') => scroll.saturating_sub(1),
                    KeyCode::PageDown => scroll.saturating_add(10),
                    KeyCode::PageUp => scroll.saturating_sub(10),
                    KeyCode::Home => 0,
                    KeyCode::End => self.areas.modal_scroll_max,
                    _ => scroll,
                }
                .min(self.areas.modal_scroll_max);
                self.modal = Some(if report {
                    Modal::Report {
                        title,
                        text,
                        scroll,
                    }
                } else {
                    Modal::Text {
                        title,
                        text,
                        scroll,
                    }
                });
            }
            Modal::Menu(mut i) => {
                match key.code {
                    KeyCode::Esc => return,
                    KeyCode::Down | KeyCode::Char('j') => i = (i + 1).min(MENU.len() - 1),
                    KeyCode::Up | KeyCode::Char('k') => i = i.saturating_sub(1),
                    KeyCode::Enter => {
                        self.menu_action(i);
                        return;
                    }
                    _ => {}
                }
                self.modal = Some(Modal::Menu(i));
            }
        }
    }
    fn menu_action(&mut self, i: usize) {
        match i {
            0 => self.action(Action::Capture, None),
            1 => self.action(Action::Apply, None),
            2 => self.action(Action::Restore, None),
            3 => self.action(Action::Forget, None),
            4 => self.action(Action::Enable, None),
            5 => self.action(Action::Disable, None),
            6 => self.edit_source(),
            7 => self.action(Action::Stage, None),
            8 => self.action(Action::Unstage, None),
            9 => {
                self.modal = Some(Modal::Input {
                    purpose: Purpose::Commit,
                    text: String::new(),
                })
            }
            10 => self.action(Action::Fetch, None),
            11 => self.action(Action::Pull, None),
            12 => self.action(Action::Push, None),
            13 => {
                self.modal = Some(Modal::Text {
                    title: "Settings · local machine".into(),
                    text: serde_json::to_string_pretty(&self.config).unwrap_or_default(),
                    scroll: 0,
                })
            }
            14 => self.toggle_policy("sync"),
            15 => self.toggle_policy("trusted"),
            16 => self.toggle_policy("reporting"),
            17 => {
                let mut c = self.config.clone();
                let p = self.config_path.clone();
                self.launch("Synchronizing", move || {
                    Ok(Work::Message(serde_json::to_string_pretty(&sync::once(
                        &mut c, &p,
                    )?)?))
                });
            }
            18 => {
                let c = self.config.clone();
                self.launch("Fetching machine reports", move || {
                    Ok(Work::Message(serde_json::to_string_pretty(
                        &sync::fetch_reports(&c)?,
                    )?))
                });
            }
            19 => {
                let c = self.config.clone();
                self.launch("Reading activity", move || {
                    Ok(Work::Message(serde_json::to_string_pretty(
                        &operations::history(&c)?,
                    )?))
                });
            }
            20 => {
                self.modal = Some(Modal::Input {
                    purpose: Purpose::Recover,
                    text: String::new(),
                })
            }
            21 => {
                self.modal = Some(Modal::Input {
                    purpose: Purpose::Switch,
                    text: String::new(),
                })
            }
            22 => {
                self.modal = Some(Modal::Input {
                    purpose: Purpose::Machine,
                    text: self.config.machine.clone(),
                })
            }
            23=>{self.modal=Some(Modal::Confirm{scroll:0,command:Control::Service,text:format!("Install and start a local user service for {}? Sync must already be enabled. It runs while the TUI is closed, checking every {} seconds.",self.config.machine,self.config.interval_seconds)});},
            24=>self.modal=Some(Modal::Confirm{scroll:0,command:Control::Packages,text:"Install missing Git and chezmoi using Homebrew (macOS) or pacman (Linux)? The terminal will return to normal mode for package-manager authentication.".into()}),
            25 => {
                let c = self.config.clone();
                self.launch("Installing agent skill", move || {
                    Ok(Work::Message(install_skill(&c)?))
                });
            }
            26 => {
                let c = self.config.clone();
                let p = self.config_path.clone();
                self.launch("Initializing local source", move || {
                    let report = initialize(&c, &p, None)?;
                    Ok(Work::Welcome(c, report))
                });
            }
            27 => {
                self.modal = Some(Modal::Input {
                    purpose: Purpose::Repo,
                    text: String::new(),
                })
            }
            28 => {
                self.modal = Some(Modal::Text {
                    title: "Shared settings".into(),
                    text: crate::settings::describe(&self.config).unwrap_or_else(|e| e.to_string()),
                    scroll: 0,
                })
            }
            32 => self.action(Action::Ignore, None),
            33 => self.action(Action::IgnoreDefaults, None),
            34 => self.action(Action::Trash, None),
            35 => self.open_assignment(),
            29..=31 => {
                self.modal = Some(Modal::Input {
                    purpose: Purpose::Setting(match i {
                        29 => crate::settings::Scope::Shared,
                        30 => crate::settings::Scope::Os,
                        _ => crate::settings::Scope::Machine,
                    }),
                    text: String::new(),
                })
            }
            _ => {}
        }
    }
    fn toggle_policy(&mut self, key: &str) {
        let current = match key {
            "sync" => self.config.sync_enabled,
            "trusted" => self.config.trusted,
            _ => self.config.reporting,
        };
        let detail = match key {
            "trusted" => {
                "Trusted mode permits repository code, templates and hooks to have broader unattended effects. Strict mode pauses unproven effects."
            }
            "reporting" => {
                "Reporting automatically commits and pushes summary metadata to separate branches in this remote. Configuration publication stays manual."
            }
            _ => {
                "Automatic sync pulls and applies only enabled configurations under this machine's trust policy. It pauses on local edits and conflicts."
            }
        };
        self.modal = Some(Modal::Confirm {
            scroll: 0,
            command: Control::Policy(key.into(), !current),
            text: format!("Set {key} = {} on this machine?\n\n{detail}", !current),
        });
    }
    fn confirm_control(&mut self, command: Control) {
        match command {
            Control::Policy(key, value) => {
                let c = self.config.clone();
                let path = self.config_path.clone();
                self.launch("Updating local policy", move || {
                    let _lock = operations::Lock::acquire(&c)?;
                    let mut next = Config::load(&path)?;
                    match key.as_str() {
                        "sync" => next.sync_enabled = value,
                        "trusted" => next.trusted = value,
                        "reporting" => next.reporting = value,
                        _ => bail!("Unknown policy"),
                    };
                    next.validate()?;
                    next.save(&path)?;
                    Ok(Work::ConfigMessage(next, format!("{key} = {value}")))
                });
            }
            Control::Switch(target) => {
                let mut c = self.config.clone();
                let p = self.config_path.clone();
                self.launch("Archiving and switching source", move || {
                    let first_setup = !c.source.exists();
                    let msg = switch_source(&mut c, &p, &target)?;
                    if first_setup {
                        Ok(Work::Welcome(
                            c,
                            SetupReport {
                                kind: SetupKind::Existing,
                                details: msg,
                            },
                        ))
                    } else {
                        Ok(Work::ConfigMessage(c, msg))
                    }
                });
            }
            Control::Recover(id) => {
                let c = self.config.clone();
                self.launch("Restoring file backups", move || {
                    Ok(Work::Message(operations::recover(&c, &id)?))
                });
            }
            Control::Service => {
                let c = self.config.clone();
                let p = self.config_path.clone();
                self.launch("Installing and starting local service", move || {
                    Ok(Work::Message(sync::install_service(&c, &p, true)?))
                });
            }
            Control::Packages => self.external = true,
        }
    }
    fn mouse(&mut self, mouse: event::MouseEvent) {
        let pos = Position::new(mouse.column, mouse.row);
        if self.modal.is_none() && self.areas.toast.bounds.contains(pos) {
            if mouse.kind == MouseEventKind::Down(event::MouseButton::Left)
                && self
                    .areas
                    .toast
                    .cancel
                    .is_some_and(|rect| rect.contains(pos))
            {
                self.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
            }
            return;
        }
        if self.modal.is_some() {
            if !self.areas.modal.contains(pos) {
                return;
            }
            match mouse.kind {
                MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                    if self.areas.modal_body.contains(pos) {
                        let key = if mouse.kind == MouseEventKind::ScrollDown {
                            KeyCode::Down
                        } else {
                            KeyCode::Up
                        };
                        self.modal_key(KeyEvent::new(key, KeyModifiers::NONE));
                    }
                }
                MouseEventKind::Down(event::MouseButton::Left) => {
                    let hit = self
                        .areas
                        .modal_controls
                        .iter()
                        .chain(self.areas.operation.controls.iter())
                        .chain(self.areas.setup.iter())
                        .find(|(r, _)| r.contains(pos))
                        .map(|(_, key)| *key);
                    if let Some(key) = hit {
                        let modifiers = if key == KeyCode::Char('s')
                            && matches!(
                                self.modal,
                                Some(Modal::Input {
                                    purpose: Purpose::Source(_),
                                    ..
                                })
                            ) {
                            KeyModifiers::CONTROL
                        } else {
                            KeyModifiers::NONE
                        };
                        self.key(KeyEvent::new(key, modifiers));
                    } else if self.areas.modal_body.contains(pos)
                        && let Some(Modal::Menu(i)) = &mut self.modal
                    {
                        let index = self.menu_state.offset()
                            + (mouse.row - self.areas.modal_body.y) as usize;
                        if index < MENU.len() {
                            *i = index;
                            let key = format!("menu-{index}");
                            let twice = self.last_click.as_ref().is_some_and(|(s, t)| {
                                s == &key && t.elapsed() < Duration::from_millis(500)
                            });
                            self.last_click = Some((key, Instant::now()));
                            if twice {
                                self.modal_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                            }
                        }
                    }
                }
                _ => {}
            }
            return;
        }
        if mouse.kind == MouseEventKind::Down(event::MouseButton::Left)
            && let Some((_, command)) = self
                .areas
                .assignment
                .controls
                .iter()
                .find(|(r, _)| r.contains(pos))
                .cloned()
        {
            self.assignment_command(command);
            return;
        }
        if mouse.kind == MouseEventKind::Down(event::MouseButton::Left)
            && let Some((_, pane, index)) = self
                .areas
                .tabs
                .iter()
                .find(|(r, _, _)| r.contains(pos))
                .copied()
        {
            self.pane = pane;
            if pane != Pane::Details {
                self.detail_owner = pane;
            }
            self.set_tab(pane, index);
            return;
        }
        if mouse.kind == MouseEventKind::Down(event::MouseButton::Left)
            && let Some((_, code)) = self.areas.footer.iter().find(|(r, _)| r.contains(pos))
        {
            self.key(KeyEvent::new(*code, KeyModifiers::NONE));
            return;
        }
        let pane = if self.areas.files.contains(pos) {
            Some(Pane::Files)
        } else if self.areas.git.contains(pos) {
            Some(Pane::Git)
        } else if self.areas.machines.contains(pos) {
            Some(Pane::Machines)
        } else if self.areas.status.contains(pos) {
            Some(Pane::Status)
        } else if self.areas.details.contains(pos) {
            Some(Pane::Details)
        } else if self.areas.activity.contains(pos) {
            Some(Pane::Activity)
        } else {
            None
        };
        if let Some(pane) = pane {
            match mouse.kind {
                MouseEventKind::ScrollDown => self.move_pane_cursor(pane, 3),
                MouseEventKind::ScrollUp => self.move_pane_cursor(pane, -3),
                MouseEventKind::Down(event::MouseButton::Left) => {
                    self.pane = pane;
                    if pane != Pane::Details {
                        self.detail_owner = pane;
                    }
                    if pane == Pane::Files && mouse.row >= self.areas.files.y + 2 {
                        let index = self.file_state.offset()
                            + (mouse.row - (self.areas.files.y + 2)) as usize;
                        if index < self.rows.len() {
                            self.cursor = index;
                            self.file_state.select(Some(index));
                            let key = format!("file-{}", self.rows[index].path);
                            let twice = self.last_click.as_ref().is_some_and(|(s, t)| {
                                s == &key && t.elapsed() < Duration::from_millis(500)
                            });
                            self.last_click = Some((key, Instant::now()));
                            if mouse.column <= self.areas.files.x + 3 {
                                self.key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
                            } else if twice {
                                if self.rows[index].directory {
                                    self.enter_dir();
                                } else {
                                    self.pane = Pane::Details;
                                }
                            }
                        }
                    }
                    if pane == Pane::Git && mouse.row > self.areas.git.y {
                        let index =
                            self.git_state.offset() + (mouse.row - (self.areas.git.y + 1)) as usize;
                        let count = if self.history_view {
                            self.snapshot.git.commits.len()
                        } else {
                            self.snapshot.git.files.len()
                        };
                        self.git_cursor = index.min(count.saturating_sub(1));
                        self.git_state.select(Some(self.git_cursor));
                    }
                }
                _ => {}
            }
        }
    }
    pub fn draw(&mut self, f: &mut Frame) {
        let area = f.area();
        self.areas = Areas::default();
        if let Some(Modal::Welcome { report, scroll }) = &self.modal {
            let (button, max_scroll) =
                onboarding::draw(f, area, &self.config.source, report, *scroll);
            self.areas.modal = area;
            self.areas.modal_body = area;
            self.areas.setup.push((button, KeyCode::Enter));
            self.areas.welcome_scroll_max = max_scroll;
            return;
        }
        let outer = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);
        if area.width < 80 || area.height < 22 || self.expanded {
            match self.pane {
                Pane::Status => self.draw_status(f, outer[0]),
                Pane::Files => self.draw_files(f, outer[0]),
                Pane::Git => self.draw_git(f, outer[0]),
                Pane::Machines => self.draw_machines(f, outer[0]),
                Pane::Details => self.draw_detail(f, outer[0]),
                Pane::Activity => self.draw_activity(f, outer[0]),
            }
        } else {
            let columns =
                Layout::horizontal([Constraint::Percentage(35), Constraint::Percentage(65)])
                    .split(outer[0]);
            let left = Layout::vertical([
                Constraint::Length(4),
                Constraint::Min(6),
                Constraint::Length(7),
                Constraint::Length(4),
            ])
            .split(columns[0]);
            let right =
                Layout::vertical([Constraint::Min(6), Constraint::Length(5)]).split(columns[1]);
            self.draw_status(f, left[0]);
            self.draw_files(f, left[1]);
            self.draw_git(f, left[2]);
            self.draw_machines(f, left[3]);
            self.draw_detail(f, right[0]);
            self.draw_activity(f, right[1]);
        }
        let message = self
            .job
            .as_ref()
            .filter(|j| j.kind == JobKind::Operation)
            .map(|j| format!("{}…", j.label))
            .unwrap_or_else(|| self.notice.clone());
        f.render_widget(
            Paragraph::new(message).style(Style::default().fg(Color::Gray)),
            outer[1],
        );
        let mut x = outer[2].x;
        let mut bindings: Vec<(&str, KeyCode)> = if self.pane == Pane::Files {
            vec![
                ("Space Select", KeyCode::Char(' ')),
                ("r Remove", KeyCode::Char('r')),
                ("d Trash", KeyCode::Char('d')),
                ("R Restore", KeyCode::Char('R')),
                ("c Capture", KeyCode::Char('c')),
                ("s Assign", KeyCode::Char('s')),
                ("a Apply", KeyCode::Char('a')),
                ("t Tracked/All", KeyCode::Char('t')),
                (": Actions", KeyCode::Char(':')),
                ("? Help", KeyCode::Char('?')),
            ]
        } else if self.pane == Pane::Details
            && self.context() == Pane::Files
            && self.detail_tab == 2
        {
            vec![
                ("o OS", KeyCode::Char('o')),
                ("m Machine", KeyCode::Char('m')),
                ("M Choose", KeyCode::Char('M')),
                ("s Save", KeyCode::Char('s')),
                ("u Reset", KeyCode::Char('u')),
                ("z Expand", KeyCode::Char('z')),
            ]
        } else if self.pane == Pane::Git && !self.history_view {
            vec![
                ("Space Stage", KeyCode::Char(' ')),
                ("c Commit", KeyCode::Char('c')),
                ("i Ignore", KeyCode::Char('i')),
                ("P Push", KeyCode::Char('P')),
                ("L Pull", KeyCode::Char('L')),
                (": Actions", KeyCode::Char(':')),
                ("? Help", KeyCode::Char('?')),
            ]
        } else {
            vec![
                ("2 Files", KeyCode::Char('2')),
                ("3 Git", KeyCode::Char('3')),
                ("z Expand", KeyCode::Char('z')),
                (": Actions", KeyCode::Char(':')),
                ("? Help", KeyCode::Char('?')),
            ]
        };
        let tabs = [
            ("[ Prev", KeyCode::Char('[')),
            ("] Next", KeyCode::Char(']')),
        ];
        if self.pane == Pane::Files {
            // File actions take priority when the footer has limited space.
            if area.width < 80 {
                bindings.retain(|(_, code)| *code != KeyCode::Char(' '));
            }
            bindings.extend(tabs);
        } else {
            bindings.splice(0..0, tabs);
        }
        for (label, code) in bindings {
            let width = (label.len() + 3) as u16;
            if matches!(code, KeyCode::Char(':') | KeyCode::Char('?')) {
                continue;
            }
            if x + width > outer[2].right().saturating_sub(20) {
                break;
            }
            let rect = Rect::new(x, outer[2].y, width, 1);
            f.render_widget(
                Paragraph::new(label).style(Style::default().fg(Color::LightGreen)),
                rect,
            );
            self.areas.footer.push((rect, code));
            x += width;
        }
        let tail = Rect::new(
            outer[2].right().saturating_sub(20).max(outer[2].x),
            outer[2].y,
            outer[2].width.min(20),
            1,
        );
        f.render_widget(
            Paragraph::new(": Actions   ? Help").style(Style::default().fg(Color::LightCyan)),
            tail,
        );
        self.areas.footer.push((
            Rect::new(tail.x, tail.y, tail.width.min(11), 1),
            KeyCode::Char(':'),
        ));
        if tail.width > 11 {
            self.areas.footer.push((
                Rect::new(tail.x + 11, tail.y, tail.width - 11, 1),
                KeyCode::Char('?'),
            ));
        }
        self.draw_modal(f, outer[0]);
        if self.modal.is_none() && self.pending_review.is_some() {
            self.areas.toast = toast::draw(
                f,
                outer[0],
                "Review ready",
                "Finish your current input to open the review.",
                true,
            );
        } else if self.modal.is_none()
            && let Some(job) = &mut self.job
            && job.kind != JobKind::Operation
            && !job.discard
            && (job.kind == JobKind::Preparation
                || job.started.elapsed() >= Duration::from_millis(250))
        {
            job.toast_started.get_or_insert_with(Instant::now);
            let title = match job.kind {
                JobKind::Preparation => "Preparing review",
                JobKind::Inspection => "Refreshing",
                _ => "Loading details",
            };
            self.areas.toast = toast::draw(
                f,
                outer[0],
                title,
                &format!("{}…", job.label),
                job.kind == JobKind::Preparation,
            );
        }
    }

    fn draw_modal(&mut self, f: &mut Frame, workspace: Rect) {
        let Some(modal) = self.modal.clone() else {
            return;
        };
        let area = f.area();
        let bounds = match &modal {
            Modal::Help(_) => dialog::bounds(area, 92, 36),
            Modal::Setup => workspace,
            Modal::Welcome { .. } => unreachable!("Welcome is a startup screen"),
            Modal::Review { .. }
            | Modal::Outcome { .. }
            | Modal::Busy { .. }
            | Modal::Report { .. } => dialog::operation_bounds(area),
            Modal::Menu(_) => dialog::bounds(area, 76, 24),
            Modal::Input {
                purpose: Purpose::Source(_),
                ..
            } => {
                if self.editor_expanded {
                    area
                } else {
                    dialog::operation_bounds(area)
                }
            }
            Modal::Input {
                purpose: Purpose::AssignmentMachines(_),
                ..
            } => dialog::bounds(area, 76, 12),
            Modal::Input { .. } => dialog::bounds(area, 72, 8),
            Modal::Confirm { text, .. } => {
                let width = dialog::bounds(area, 72, 20).width.saturating_sub(2);
                let lines = Paragraph::new(text.as_str())
                    .wrap(Wrap { trim: false })
                    .line_count(width);
                dialog::bounds(area, 72, (lines.saturating_add(5).min(24) as u16).max(8))
            }
            Modal::Text { text, .. } => {
                let width = dialog::bounds(area, 88, 26).width.saturating_sub(2);
                let lines = Paragraph::new(text.as_str())
                    .wrap(Wrap { trim: false })
                    .line_count(width);
                dialog::bounds(area, 88, lines.saturating_add(5).min(26) as u16)
            }
        };
        self.areas.modal = bounds;
        if !matches!(modal, Modal::Setup) {
            dialog::dim(f);
        }
        let editor_title = match &modal {
            Modal::Input {
                purpose: Purpose::Source(path),
                ..
            } => format!("Edit source · {path}"),
            _ => String::new(),
        };
        let title = match &modal {
            Modal::Help(_) => "Help · Keyboard shortcuts",
            Modal::Setup => "Welcome to dot · Save and reuse your app settings",
            Modal::Review { .. } => "Review operation",
            Modal::Outcome { .. } => "Operation result",
            Modal::Busy { .. } => "Operation in progress",
            Modal::Report { title, .. } | Modal::Text { title, .. } => title,
            Modal::Confirm { .. } => "Review local action",
            Modal::Menu(_) => "Actions · Enter activate · Esc cancel",
            Modal::Input { purpose, .. } => match purpose {
                Purpose::Setting(_) => "Setting key=value",
                Purpose::AssignmentMachines(_) => "Assign machines · comma-separated hostnames",
                Purpose::Repo => "Repository URL",
                Purpose::Commit => "Commit message",
                Purpose::Source(_) => &editor_title,
                Purpose::Switch => "New source path",
                Purpose::Machine => "Machine label",
                Purpose::Recover => "Recovery id",
            },
            Modal::Welcome { .. } => unreachable!(),
        };
        let inner = dialog::frame(f, bounds, title);
        let regions =
            Layout::vertical([Constraint::Min(0), Constraint::Length(2.min(inner.height))])
                .split(inner);
        self.areas.modal_body = regions[0];
        match modal {
            Modal::Help(view) => {
                self.areas.help = help::draw(f, inner, &view);
                self.areas.modal_body = self.areas.help.body;
                self.areas.modal_controls = self.areas.help.controls.clone();
            }
            Modal::Welcome { .. } => unreachable!(),
            Modal::Review { plan, view } => {
                self.areas.operation =
                    operation_screen::review(f, inner, &self.config, &plan, &view);
                self.areas.modal_body = self.areas.operation.body;
            }
            Modal::Outcome { outcome, view } => {
                self.areas.operation =
                    operation_screen::result(f, inner, &self.config, &outcome, &view);
                self.areas.modal_body = self.areas.operation.body;
            }
            Modal::Busy { label } => {
                f.render_widget(
                    Paragraph::new(format!("{label}…\n\nPlease wait for the result. This operation cannot be cancelled."))
                        .wrap(Wrap { trim: false }),
                    regions[0],
                );
            }
            Modal::Setup => {
                let areas = Layout::vertical([
                    Constraint::Length(4),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Min(0),
                ])
                .split(inner);
                f.render_widget(Paragraph::new(format!(
                    "Choose where dot will keep copies of your settings.\nThe folder below hasn't been created yet:\n{}\nYour apps' current settings won't change during setup.",
                    self.config.source.display()
                )).wrap(Wrap { trim: false }), areas[0]);
                for (index, (label, code)) in [
                    (
                        "[i] Start fresh · Create a settings folder",
                        KeyCode::Char('i'),
                    ),
                    (
                        "[c] Download settings from a Git repository",
                        KeyCode::Char('c'),
                    ),
                    (
                        "[o] Use a settings folder already on this computer",
                        KeyCode::Char('o'),
                    ),
                    ("[Esc] Explore my files without setup", KeyCode::Esc),
                ]
                .into_iter()
                .enumerate()
                {
                    f.render_widget(Paragraph::new(label).fg(Color::LightCyan), areas[index + 1]);
                    self.areas.setup.push((areas[index + 1], code));
                }
                // Keep the existing startup exit affordance.
                f.render_widget(
                    Paragraph::new("Enter / Esc Back · q / Ctrl+C Quit"),
                    Rect::new(
                        bounds.x + 1,
                        bounds.bottom().saturating_sub(1),
                        bounds.width.saturating_sub(2),
                        1,
                    ),
                );
            }
            Modal::Confirm { text, scroll, .. } => {
                self.areas.modal_scroll_max = Paragraph::new(text.as_str())
                    .wrap(Wrap { trim: false })
                    .line_count(regions[0].width)
                    .saturating_sub(regions[0].height as usize)
                    .min(u16::MAX as usize) as u16;
                f.render_widget(
                    Paragraph::new(text)
                        .wrap(Wrap { trim: false })
                        .scroll((scroll.min(self.areas.modal_scroll_max), 0)),
                    regions[0],
                );
                self.areas.modal_controls = dialog::controls(
                    f,
                    regions[1],
                    &[
                        ("Enter/y Execute", KeyCode::Char('y')),
                        ("Esc Cancel", KeyCode::Esc),
                    ],
                );
            }
            Modal::Menu(i) => {
                self.menu_state.select(Some(i));
                let list = List::new(MENU.iter().map(|s| ListItem::new(*s)))
                    .highlight_style(selection_style())
                    .highlight_symbol("› ");
                f.render_stateful_widget(list, regions[0], &mut self.menu_state);
                self.areas.modal_controls = dialog::controls(
                    f,
                    regions[1],
                    &[
                        ("Enter Activate", KeyCode::Enter),
                        ("Esc Cancel", KeyCode::Esc),
                    ],
                );
            }
            Modal::Text { text, scroll, .. } | Modal::Report { text, scroll, .. } => {
                let paragraph = Paragraph::new(text).wrap(Wrap { trim: false });
                self.areas.modal_scroll_max = paragraph
                    .line_count(regions[0].width)
                    .saturating_sub(regions[0].height as usize)
                    .min(u16::MAX as usize) as u16;
                f.render_widget(
                    paragraph.scroll((scroll.min(self.areas.modal_scroll_max), 0)),
                    regions[0],
                );
                self.areas.modal_controls =
                    dialog::controls(f, regions[1], &[("Enter / Esc Close", KeyCode::Esc)]);
            }
            Modal::Input { purpose, text } => {
                let source = matches!(purpose, Purpose::Source(_));
                let machine_assignment = matches!(purpose, Purpose::AssignmentMachines(_));
                let mut input_area = regions[0];
                if machine_assignment {
                    let known: BTreeSet<_> = self
                        .snapshot
                        .assignments
                        .values()
                        .flat_map(|scope| scope.machines.iter().cloned())
                        .chain(std::iter::once(self.snapshot.identity.hostname.clone()))
                        .collect();
                    let hint = Paragraph::new(format!(
                        "Use commas for several machines. Empty means all machines.\nThis machine: {}\nKnown: {}",
                        self.snapshot.identity.hostname, known.iter().take(6).cloned().collect::<Vec<_>>().join(", ")
                    )).fg(UNTRACKED_TEXT).wrap(Wrap { trim: false });
                    let height = (hint.line_count(input_area.width) as u16)
                        .min(input_area.height.saturating_sub(1));
                    let sections =
                        Layout::vertical([Constraint::Length(height), Constraint::Min(1)])
                            .split(input_area);
                    f.render_widget(hint, sections[0]);
                    input_area = sections[1];
                }
                let byte = text
                    .char_indices()
                    .nth(self.input_cursor)
                    .map(|(i, _)| i)
                    .unwrap_or(text.len());
                let styled = if let Purpose::Source(path) = &purpose {
                    syntax::source(path, &text)
                } else {
                    Text::from(text.clone())
                };
                let styled = syntax::caret(styled, &text, byte);
                let mut display = text;
                // Include the cursor marker in wrapping so it remains visible at line ends.
                display.insert(byte, '▏');
                let prefix = &display[..byte + '▏'.len_utf8()];
                let cursor_rows = Paragraph::new(prefix)
                    .wrap(Wrap { trim: false })
                    .line_count(input_area.width);
                let offset = cursor_rows
                    .saturating_sub(input_area.height as usize)
                    .min(u16::MAX as usize) as u16;
                f.render_widget(
                    Paragraph::new(styled)
                        .scroll((offset, 0))
                        .wrap(Wrap { trim: false }),
                    input_area,
                );
                let submit = if source {
                    ("Ctrl+S Review", KeyCode::Char('s'))
                } else if machine_assignment {
                    ("Enter Use machines", KeyCode::Enter)
                } else {
                    ("Enter Continue", KeyCode::Enter)
                };
                // Source review clicks use the same Ctrl+S modifiers as the keyboard.
                let mut controls = vec![submit, ("Esc Cancel", KeyCode::Esc)];
                if source {
                    controls.push(("F2 Expand/restore", KeyCode::F(2)));
                }
                self.areas.modal_controls = dialog::controls(f, regions[1], &controls);
            }
        }
    }
    fn block(&self, title: String, pane: Pane) -> Block<'static> {
        let focused = self.pane == pane;
        let block = Block::bordered()
            .border_type(if focused {
                BorderType::Double
            } else {
                BorderType::Plain
            })
            .border_style(if focused {
                Style::default().fg(FOCUS_BORDER).bold()
            } else {
                Style::default().fg(Color::Rgb(73, 80, 94))
            });
        if title.is_empty() {
            block
        } else {
            block.title(title)
        }
    }
    fn tab_block(&mut self, r: Rect, pane: Pane, prefix: &str) -> Block<'static> {
        let labels = pane_tabs(pane);
        let selected = self.tab_index(pane);
        let available = r.width.saturating_sub(2) as usize;
        let mut prefix = prefix.to_string();
        if prefix.chars().count() + labels[selected].len() > available {
            prefix = format!("{} ", prefix.split_whitespace().next().unwrap_or(""));
        }
        let all_width = labels.iter().map(|s| s.len()).sum::<usize>() + (labels.len() - 1) * 3;
        let visible: Vec<_> = if prefix.chars().count() + all_width <= available {
            (0..labels.len()).collect()
        } else {
            vec![selected]
        };
        let mut spans = vec![Span::styled(
            prefix.clone(),
            Style::default()
                .fg(if self.pane == pane {
                    FOCUS_BORDER
                } else {
                    Color::Rgb(196, 167, 231)
                })
                .add_modifier(Modifier::BOLD),
        )];
        let mut x =
            r.x.saturating_add(1)
                .saturating_add(prefix.chars().count() as u16);
        for (offset, index) in visible.into_iter().enumerate() {
            if offset > 0 {
                spans.push(Span::styled(" - ", Style::default().fg(UNTRACKED_TEXT)));
                x = x.saturating_add(3);
            }
            let style = if index == selected {
                Style::default()
                    .fg(if self.pane == pane {
                        FOCUS_BORDER
                    } else {
                        Color::Rgb(224, 229, 238)
                    })
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(UNTRACKED_TEXT)
            };
            spans.push(Span::styled(labels[index], style));
            let width =
                (labels[index].len() as u16).min(r.right().saturating_sub(1).saturating_sub(x));
            if width > 0 && r.height > 0 {
                self.areas
                    .tabs
                    .push((Rect::new(x, r.y, width, 1), pane, index));
            }
            x = x.saturating_add(labels[index].len() as u16);
        }
        self.block(String::new(), pane).title(Line::from(spans))
    }
    fn draw_status(&mut self, f: &mut Frame, r: Rect) {
        self.areas.status = r;
        let s = &self.snapshot;
        let text = if self.status_tab == 1 {
            format!(
                "HOME: {}\nSource: {}\nConfig: {}",
                self.config.home.display(),
                self.config.source.display(),
                self.config_path.display()
            )
        } else {
            format!(
                "{} · {} ↑{} ↓{}\n{} HOME differences · {} source changes",
                self.config.machine,
                s.git.branch,
                s.git.ahead,
                s.git.behind,
                s.changes.len(),
                s.git.files.len()
            )
        };
        let block = self.tab_block(r, Pane::Status, "1 Status · ");
        f.render_widget(
            Paragraph::new(text)
                .block(block)
                .scroll((self.status_scroll, 0)),
            r,
        );
    }
    fn draw_files(&mut self, f: &mut Frame, r: Rect) {
        self.areas.files = r;
        let b = self.tab_block(r, Pane::Files, "2 Files · ");
        let inner = b.inner(r);
        f.render_widget(b, r);
        if inner.height == 0 {
            return;
        }
        f.render_widget(
            Paragraph::new(format!(
                "{}{} · ~/{} · {} selected",
                if self.searching { "/ " } else { "Filter: " },
                self.query,
                self.directory,
                self.selected.len()
            )),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );
        let items = self
            .rows
            .iter()
            .map(|row| {
                let check = if self.selected.contains(&row.path) {
                    "●"
                } else {
                    " "
                };
                let kind = if row.directory { "▸" } else { " " };
                let count = if row.directory && row.managed {
                    format!(" · {} managed", row.children)
                } else {
                    String::new()
                };
                ListItem::new(format!(
                    "{check} {kind} {}{} {count} [{}]",
                    row.name,
                    if row.directory { "/" } else { "" },
                    row.status
                ))
                .style(Style::default().fg(if row.status == "OFF" {
                    UNTRACKED_TEXT
                } else if row.changed {
                    Color::Yellow
                } else if !row.managed {
                    UNTRACKED_TEXT
                } else {
                    Color::White
                }))
            })
            .collect::<Vec<_>>();
        let list = List::new(items)
            .highlight_style(selection_style())
            .highlight_symbol("› ");
        f.render_stateful_widget(
            list,
            Rect::new(
                inner.x,
                inner.y + 1,
                inner.width,
                inner.height.saturating_sub(1),
            ),
            &mut self.file_state,
        );
    }
    fn draw_git(&mut self, f: &mut Frame, r: Rect) {
        self.areas.git = r;
        let items: Vec<ListItem> = if self.history_view {
            self.snapshot
                .git
                .commits
                .iter()
                .map(|s| ListItem::new(s.clone()))
                .collect()
        } else {
            self.snapshot
                .git
                .files
                .iter()
                .map(|s| {
                    ListItem::new(format!(
                        "{} {}{} {}",
                        if self.git_selected.contains(&s.path) {
                            "●"
                        } else {
                            " "
                        },
                        s.index,
                        s.worktree,
                        s.path
                    ))
                })
                .collect()
        };
        let block = self.tab_block(r, Pane::Git, "3 Git · ");
        let list = List::new(items)
            .block(block)
            .highlight_style(selection_style())
            .highlight_symbol("› ");
        f.render_stateful_widget(list, r, &mut self.git_state);
    }
    fn draw_machines(&mut self, f: &mut Frame, r: Rect) {
        self.areas.machines = r;
        let reports = sync::cached_reports(&self.config);
        let text = if self.machines_tab == 0 {
            format!(
                "{} · {} enabled\nSync: {} · Trust: {}",
                self.config.machine,
                self.config.enabled.len(),
                if self.config.sync_enabled {
                    "on"
                } else {
                    "off"
                },
                if self.config.trusted {
                    "trusted"
                } else {
                    "strict"
                }
            )
        } else if reports.is_empty() {
            "No cached reports · : Actions → Fetch machine reports".into()
        } else {
            reports
                .iter()
                .map(|r| {
                    format!(
                        "{} · {} · {}",
                        r.installation.chars().take(8).collect::<String>(),
                        r.outcome,
                        r.observed_at
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let block = self.tab_block(r, Pane::Machines, "4 Machines · ");
        f.render_widget(
            Paragraph::new(text)
                .block(block)
                .scroll((self.machines_scroll, 0)),
            r,
        );
    }
    fn draw_detail(&mut self, f: &mut Frame, r: Rect) {
        self.areas.details = r;
        let block = self.tab_block(r, Pane::Details, "0 Details · ");
        if self.context() == Pane::Files && self.detail_tab == 2 {
            let inner = block.inner(r);
            f.render_widget(block, r);
            let row = self.rows.get(self.cursor);
            let scope = row
                .map(|row| self.assignment_scope(&row.path))
                .unwrap_or_default();
            let dirty = row.is_some_and(|row| {
                scope
                    != self
                        .snapshot
                        .assignments
                        .get(&row.path)
                        .cloned()
                        .unwrap_or_default()
            });
            self.areas.assignment =
                assignment_screen::draw(f, inner, row, &self.snapshot, &scope, dirty, self.scroll);
            self.scroll = self.scroll.min(self.areas.assignment.max_scroll);
            return;
        }
        f.render_widget(
            Paragraph::new(syntax::preview(&self.detail))
                .block(block)
                .scroll((self.scroll, 0)),
            r,
        );
    }
    fn draw_activity(&mut self, f: &mut Frame, r: Rect) {
        self.areas.activity = r;
        let events = operations::history(&self.config).unwrap_or_default();
        let text = events
            .iter()
            .rev()
            .take(if self.activity_tab == 0 {
                10
            } else {
                usize::MAX
            })
            .map(|e| format!("{} · {} · {}", e.time.format("%H:%M"), e.intent, e.outcome))
            .collect::<Vec<_>>()
            .join("\n");
        let block = self.tab_block(r, Pane::Activity, "Activity · ");
        f.render_widget(
            Paragraph::new(text)
                .block(block)
                .scroll((self.activity_scroll, 0)),
            r,
        );
    }
}
fn file_detail(c: &Config, snapshot: &mut Snapshot, row: &Row, tab: usize) -> Result<String> {
    if tab == 3 {
        return Ok(serde_json::to_string_pretty(
            &operations::history(c)?
                .into_iter()
                .filter(|event| event.paths.contains(&row.path))
                .collect::<Vec<_>>(),
        )?);
    }
    if row.directory {
        backend::inspect_directory(c, snapshot, &row.path)?;
        let rows = backend::rows(c, snapshot, &row.path, Filter::All, "")?;
        return Ok(format!(
            "~/{} · {} managed descendants\n\n{}",
            row.path,
            row.children,
            rows.iter()
                .map(|r| format!(
                    "{} {}{}",
                    r.status,
                    r.name,
                    if r.directory { "/" } else { "" }
                ))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    if let Some(saved) = snapshot
        .managed
        .get(&row.path)
        .or_else(|| snapshot.scoped_out.get(&row.path))
    {
        if tab == 1 {
            return Ok(format!(
                "SOURCE {}\n\n{}",
                saved.source_relative,
                fs::read_to_string(&saved.source_absolute)
                    .unwrap_or_else(|_| "Binary or unavailable source".into())
            ));
        }
        if snapshot.scoped_out.contains_key(&row.path) {
            return Ok(format!(
                "NOT APPLIED HERE · ~/{}\n\nThis saved copy is assigned to other operating systems or machines.\n\nPress s to change the assignment in Details → Rules.\nOriginal files stay unchanged.",
                row.path
            ));
        }
        let diff = backend::preview(c, std::slice::from_ref(&row.path), false)?;
        return Ok(format!(
            "SOURCE → HOME · {}\n\n{}",
            row.path,
            if diff.is_empty() {
                "HOME matches rendered source. Check Git for unpublished changes.".into()
            } else {
                diff
            }
        ));
    }
    if crate::ignores::eligible(c, std::slice::from_ref(&row.path))?.is_empty() {
        return Ok(format!(
            "IGNORED ~/{}\n\nCapture skips this file because of .chezmoiignore. Edit that file to change the rule.\n\nPress i to add an explicit ignore rule to both chezmoi and Git.",
            row.path
        ));
    }
    Ok(format!(
        "UNTRACKED ~/{}\n\nSelect with Space, then Capture to add through chezmoi.\n\n{}",
        row.path,
        fs::read_to_string(c.home.join(&row.path))
            .unwrap_or_else(|_| "Binary or unavailable file".into())
    ))
}
fn pane_tabs(pane: Pane) -> &'static [&'static str] {
    match pane {
        Pane::Status => &["Overview", "Paths"],
        Pane::Files => &["All", "Managed", "Changed"],
        Pane::Git => &["Changes", "Commits"],
        Pane::Machines => &["Local", "Reports"],
        Pane::Details => &["Diff", "Source", "Rules", "Activity"],
        Pane::Activity => &["Recent", "All"],
    }
}
fn vertical_cursor(chars: &[char], cursor: usize, delta: isize) -> usize {
    let mut starts = vec![0];
    for (i, c) in chars.iter().enumerate() {
        if *c == '\n' {
            starts.push(i + 1);
        }
    }
    let row = starts.iter().rposition(|i| *i <= cursor).unwrap_or(0);
    let column = cursor - starts[row];
    let target = row.saturating_add_signed(delta).min(starts.len() - 1);
    let end = starts.get(target + 1).map(|v| v - 1).unwrap_or(chars.len());
    (starts[target] + column).min(end)
}
const MENU: &[&str] = &[
    "Capture selected HOME changes → source",
    "Apply selected source → HOME",
    "Restore selected HOME from source",
    "Remove saved copies from dot (keep original files)",
    "Enable selected for automatic updates here",
    "Disable selected automatic updates here",
    "Edit selected source",
    "Stage selected source files",
    "Unstage selected source files",
    "Commit actual staged changes",
    "Fetch remote",
    "Pull fast-forward",
    "Push configuration commits",
    "Settings / paths",
    "Toggle local automatic sync",
    "Trusted mode policy",
    "Report publishing policy",
    "Sync now",
    "Fetch machine reports",
    "Full activity history",
    "Recover files from backup id",
    "Archive and switch source",
    "Rename machine label",
    "Background service setup",
    "Packages / dependencies",
    "Install or update agent skill",
    "Initialize local source",
    "Clone a source repository",
    "Inspect shared settings",
    "Set shared setting",
    "Set OS override",
    "Set machine override",
    "Ignore selected files in chezmoi and Git",
    "Add developer ignore defaults to both files",
    "Delete untracked originals · move to system Trash",
    "Assign file OS and machines · Details / Rules",
];

struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(
            std::io::stdout(),
            DisableMouseCapture,
            DisableBracketedPaste
        );
        ratatui::restore();
    }
}
pub fn run(config: Config, path: PathBuf) -> Result<()> {
    let mut terminal = ratatui::init();
    let _guard = TerminalGuard;
    execute!(std::io::stdout(), EnableMouseCapture, EnableBracketedPaste)?;
    let mut app = App::new(config, path);
    app.refresh();
    loop {
        app.pump();
        if app.external {
            app.external = false;
            execute!(
                std::io::stdout(),
                DisableMouseCapture,
                DisableBracketedPaste
            )?;
            ratatui::restore();
            let status = std::process::Command::new(std::env::current_exe()?)
                .args(["--config"])
                .arg(&app.config_path)
                .args(["packages", "--install"])
                .status();
            terminal = ratatui::init();
            execute!(std::io::stdout(), EnableMouseCapture, EnableBracketedPaste)?;
            terminal.clear()?;
            app.notice = match status {
                Ok(s) => format!("Package manager exited {s}"),
                Err(e) => e.to_string(),
            };
            app.refresh();
        }
        terminal.draw(|f| app.draw(f))?;
        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(k) => {
                    if app.key(k) {
                        break;
                    }
                }
                Event::Mouse(m) => app.mouse(m),
                Event::Resize(_, _) => {}
                Event::Paste(s) => {
                    if let Some(Modal::Input { text, .. }) = &mut app.modal {
                        let mut chars: Vec<_> = text.chars().collect();
                        let i = app.input_cursor.min(chars.len());
                        let add: Vec<_> = s.chars().collect();
                        app.input_cursor = i + add.len();
                        chars.splice(i..i, add);
                        *text = chars.into_iter().collect();
                    } else if app.modal.is_none() && app.searching {
                        app.query.push_str(&s);
                        app.rebuild();
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}
pub fn install_skill(c: &Config) -> Result<String> {
    let path = c.home.join(".agents/skills/dot/SKILL.md");
    let body = include_str!("../skills/dot/SKILL.md");
    let digest = operations::hash(body.as_bytes());
    let hash_path = c.state.join("installed-skill.sha256");
    if path.exists() {
        let existing = fs::read(&path)?;
        if existing == body.as_bytes() {
            return Ok("Agent skill is current".into());
        }
        let previous = fs::read_to_string(&hash_path).unwrap_or_default();
        if operations::hash(&existing) != previous {
            let proposed = path.with_file_name("SKILL.md.dot-new");
            if proposed.exists() {
                bail!(
                    "Skill and proposed update already exist; merge manually: {}",
                    proposed.display()
                );
            }
            atomic_write(&proposed, body.as_bytes())?;
            return Ok(format!(
                "Preserved existing skill; proposed update: {}",
                proposed.display()
            ));
        }
    }
    atomic_write(&path, body.as_bytes())?;
    atomic_write(&hash_path, digest.as_bytes())?;
    Ok(format!("Installed {}", path.display()))
}
pub fn switch_source(c: &mut Config, config_path: &Path, target: &Path) -> Result<String> {
    let _lock = operations::Lock::acquire(c)?;
    let target = target
        .canonicalize()
        .context("New source must be an existing directory")?;
    if target == c.source
        || target.starts_with(&c.source)
        || c.source.starts_with(&target)
        || target == c.home
    {
        bail!("Choose a separate source repository");
    }
    let mut next = c.clone();
    next.source = target.clone();
    next.validate()?;
    git::run(&next, &["rev-parse", "--show-toplevel"])?;
    let archive = c
        .state
        .join("archives")
        .join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&archive)?;
    if c.source.exists() {
        migration::copy_tree(&c.source, &archive.join("source"))?;
    }
    atomic_write(&archive.join("config.json"), &serde_json::to_vec_pretty(c)?)?;
    next.enabled.clear();
    next.sync_enabled = false;
    next.trusted = false;
    next.reporting = false;
    next.save(config_path)?;
    *c = next;
    Ok(format!(
        "Archived original at {}. Connected {}. HOME unchanged; automatic scope is empty.",
        archive.display(),
        target.display()
    ))
}

pub fn initialize(c: &Config, path: &Path, repo: Option<&str>) -> Result<SetupReport> {
    let existed = c.source.exists();
    if let Some(repo) = repo {
        if repo.starts_with('-') || repo.contains('\n') {
            bail!("Invalid repository address");
        }
        if c.source.exists() {
            bail!("Source already exists; switch to a separate source path first");
        }
        fs::create_dir_all(c.source.parent().context("Source parent missing")?)?;
        crate::process::run(
            &c.git,
            &crate::process::strings(&["clone", "--", repo, &c.source.to_string_lossy()]),
            None,
            None,
        )?;
    } else {
        fs::create_dir_all(&c.source)?;
        if git::run(c, &["rev-parse", "--show-toplevel"]).is_err() {
            git::run(c, &["init", "-b", "main"])?;
        }
    }
    crate::ignores::install_defaults(c)?;
    c.save(path)?;
    let skill = install_skill(c)?;
    Ok(SetupReport {
        kind: if repo.is_some() {
            SetupKind::Cloned
        } else if existed {
            SetupKind::Existing
        } else {
            SetupKind::Created
        },
        details: format!(
            "Developer ignore defaults are ready in .chezmoiignore and .gitignore.\n{skill}"
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app() -> (tempfile::TempDir, App) {
        let tmp = tempfile::tempdir().unwrap();
        let c = Config {
            home: tmp.path().join("home"),
            source: tmp.path().join("source"),
            state: tmp.path().join("state"),
            ..Default::default()
        };
        fs::create_dir_all(&c.home).unwrap();
        fs::create_dir_all(&c.source).unwrap();
        fs::write(c.home.join(".untracked"), "untracked").unwrap();
        let mut a = App::new(c, tmp.path().join("config.toml"));
        a.snapshot.git.branch = "main".into();
        a.snapshot.git.ahead = 1;
        for p in [".config/ghostty/config", ".agents/skills/dot/SKILL.md"] {
            a.snapshot.managed.insert(
                p.into(),
                backend::Managed {
                    absolute: a.config.home.join(p).to_string_lossy().into_owned(),
                    source_absolute: a.config.source.join(p).to_string_lossy().into_owned(),
                    source_relative: p.into(),
                },
            );
        }
        a.snapshot
            .changes
            .insert(".config/ghostty/config".into(), " M".into());
        a.rebuild();
        a.detail="SOURCE → HOME · .config/ghostty/config\n\n@@ -1 +1 @@\n-font-size = 14\n+font-size = 16".into();
        a.notice = "2 configurations · 1 changed · 1 unpublished commit".into();
        (tmp, a)
    }
    #[test]
    fn syntax_colors_reach_file_details_editor_and_review() {
        let (_t, mut a) = app();
        let mut term = Terminal::new(ratatui::backend::TestBackend::new(120, 35)).unwrap();
        let mint = Color::Rgb(97, 218, 179);
        fn has_colored_symbol(
            term: &Terminal<ratatui::backend::TestBackend>,
            area: Rect,
            symbol: &str,
            color: Color,
        ) -> bool {
            (area.y..area.bottom()).any(|y| {
                (area.x..area.right()).any(|x| {
                    let cell = &term.backend().buffer()[(x, y)];
                    cell.symbol() == symbol && cell.fg == color
                })
            })
        }
        a.detail = "SOURCE private_dot_zshrc.tmpl\n\nexport NAME=\"Ω\"\n".into();
        term.draw(|f| a.draw(f)).unwrap();
        assert!(has_colored_symbol(&term, a.areas.details, "Ω", mint));
        a.modal = Some(Modal::Input {
            purpose: Purpose::Source(".zshrc".into()),
            text: "export NAME=\"Ω\"\n".into(),
        });
        term.draw(|f| a.draw(f)).unwrap();
        assert!(has_colored_symbol(&term, a.areas.modal_body, "Ω", mint));
        a.modal = Some(Modal::Review { plan: Plan {
            id: "syntax".into(), action: Action::Capture, paths: vec![".zshrc".into()], payload: None,
            fingerprint: String::new(), created_at: String::new(),
            preview: "--- a/.zshrc\n+++ b/.zshrc\n@@ -1 +1 @@\n-export NAME=\"Ω\"\n+export NAME=\"Ψ\"\n".into(),
        }, view: OperationView { tab: 1, scroll: 0 } });
        term.draw(|f| a.draw(f)).unwrap();
        assert!(has_colored_symbol(&term, a.areas.operation.body, "Ω", mint));
        assert!(has_colored_symbol(&term, a.areas.operation.body, "Ψ", mint));
    }

    #[test]
    fn action_dialogs_preserve_the_workspace_around_their_bounds() {
        let (_t, mut a) = app();
        let plan = Plan {
            id: "dialog-fixture".into(),
            action: Action::Capture,
            paths: vec![".untracked".into()],
            payload: None,
            fingerprint: String::new(),
            created_at: String::new(),
            preview: "New saved copy: .untracked".into(),
        };
        let outcome = Outcome {
            id: plan.id.clone(),
            action: plan.action,
            succeeded: plan.paths.clone(),
            failed: None,
            not_attempted: vec![],
            message: String::new(),
        };
        for modal in [
            Modal::Menu(0),
            Modal::Input {
                purpose: Purpose::Commit,
                text: "Save settings".into(),
            },
            Modal::Confirm {
                command: Control::Service,
                text: "Start the service?".into(),
                scroll: 0,
            },
            Modal::Help(help::View::default()),
            Modal::Review {
                plan,
                view: OperationView::default(),
            },
            Modal::Outcome {
                outcome,
                view: OperationView::default(),
            },
        ] {
            a.modal = None;
            let mut term = Terminal::new(ratatui::backend::TestBackend::new(120, 35)).unwrap();
            term.draw(|f| a.draw(f)).unwrap();
            let workspace = term.backend().buffer().clone();
            a.modal = Some(modal.clone());
            term.draw(|f| a.draw(f)).unwrap();
            let bounds = a.areas.modal;
            assert!(bounds.x > 0 && bounds.y > 0, "not an overlay: {modal:?}");
            assert!(bounds.right() < 120 && bounds.bottom() < 35);
            for y in 0..35 {
                for x in 0..120 {
                    if !bounds.contains(Position::new(x, y)) {
                        assert_eq!(
                            term.backend().buffer()[(x, y)].symbol(),
                            workspace[(x, y)].symbol()
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn brackets_switch_only_the_focused_section_and_wrap() {
        let (_t, mut a) = app();
        let panes = [
            Pane::Status,
            Pane::Files,
            Pane::Git,
            Pane::Machines,
            Pane::Details,
            Pane::Activity,
        ];
        for pane in panes {
            a.pane = pane;
            let before = panes.map(|p| a.tab_index(p));
            a.key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE));
            assert_eq!(a.tab_index(pane), 1);
            for (index, other) in panes.iter().enumerate() {
                if *other != pane {
                    assert_eq!(a.tab_index(*other), before[index]);
                }
            }
            a.key(KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE));
            assert_eq!(a.tab_index(pane), 0);
            a.key(KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE));
            assert_eq!(a.tab_index(pane), pane_tabs(pane).len() - 1);
            a.key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE));
            assert_eq!(a.tab_index(pane), 0);
        }
        a.searching = true;
        a.key(KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE));
        a.key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE));
        assert_eq!(a.query, "[]");
        assert_eq!(a.activity_tab, 0);
    }

    #[test]
    fn dialog_mouse_and_keyboard_never_reach_the_workspace() {
        let (_t, mut a) = app();
        a.query = "keep-search".into();
        a.searching = true;
        a.selected.insert(".untracked".into());
        let before = (
            a.pane,
            a.directory.clone(),
            a.cursor,
            a.query.clone(),
            a.selected.clone(),
        );
        for (w, h) in [(120, 35), (80, 24), (45, 14), (20, 8)] {
            a.modal = Some(Modal::Menu(4));
            let mut term = Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
            term.draw(|f| a.draw(f)).unwrap();
            for kind in [
                MouseEventKind::ScrollDown,
                MouseEventKind::Down(event::MouseButton::Left),
            ] {
                a.mouse(event::MouseEvent {
                    kind,
                    column: 0,
                    row: 0,
                    modifiers: KeyModifiers::NONE,
                });
            }
            a.key(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE));
            assert!(matches!(a.modal, Some(Modal::Menu(4))));
            assert_eq!(
                (
                    a.pane,
                    a.directory.clone(),
                    a.cursor,
                    a.query.clone(),
                    a.selected.clone()
                ),
                before
            );
            // Border/blank-area clicks cannot select an action.
            a.mouse(event::MouseEvent {
                kind: MouseEventKind::Down(event::MouseButton::Left),
                column: a.areas.modal.x,
                row: a.areas.modal.y,
                modifiers: KeyModifiers::NONE,
            });
            assert!(matches!(a.modal, Some(Modal::Menu(4))));
            let body = a.areas.modal_body;
            let expected = a.menu_state.offset();
            a.mouse(event::MouseEvent {
                kind: MouseEventKind::Down(event::MouseButton::Left),
                column: body.x,
                row: body.y,
                modifiers: KeyModifiers::NONE,
            });
            assert!(matches!(a.modal, Some(Modal::Menu(i)) if i == expected));
            let cancel = a
                .areas
                .modal_controls
                .iter()
                .find(|(_, key)| *key == KeyCode::Esc)
                .unwrap()
                .0;
            a.mouse(event::MouseEvent {
                kind: MouseEventKind::Down(event::MouseButton::Left),
                column: cancel.x,
                row: cancel.y,
                modifiers: KeyModifiers::NONE,
            });
            assert!(a.modal.is_none());
            assert_eq!(
                (
                    a.pane,
                    a.directory.clone(),
                    a.cursor,
                    a.query.clone(),
                    a.selected.clone()
                ),
                before
            );
        }
    }

    #[test]
    fn quick_preparation_keeps_its_toast_visible_before_opening_review() {
        let (_t, mut a) = app();
        let (tx, rx) = mpsc::channel();
        a.job = Some(Job {
            rx,
            label: "Capture".into(),
            kind: JobKind::Preparation,
            discard: false,
            started: Instant::now(),
            toast_started: None,
        });
        tx.send(Ok(Work::Plan(Plan {
            id: "quick".into(),
            action: Action::Capture,
            paths: vec![".untracked".into()],
            payload: None,
            fingerprint: String::new(),
            created_at: String::new(),
            preview: String::new(),
        })))
        .unwrap();
        let mut term = Terminal::new(ratatui::backend::TestBackend::new(120, 35)).unwrap();
        term.draw(|f| a.draw(f)).unwrap();
        a.pump();
        assert!(
            a.modal.is_none(),
            "a quick result must not immediately replace the preparation toast"
        );
        assert!(a.job.is_some());
        term.draw(|f| a.draw(f)).unwrap();
        assert!(!a.areas.toast.bounds.is_empty());
        a.job.as_mut().unwrap().toast_started = Some(Instant::now() - Duration::from_millis(200));
        a.pump();
        assert!(
            a.modal.is_none(),
            "the toast must still be visible before 0.8 seconds"
        );
        a.job.as_mut().unwrap().toast_started = Some(Instant::now() - Duration::from_millis(801));
        a.pump();
        assert!(matches!(a.modal, Some(Modal::Review { ref plan, .. }) if plan.id == "quick"));
        term.draw(|f| a.draw(f)).unwrap();
        assert!(a.areas.toast.bounds.is_empty());
    }

    #[test]
    fn preparation_uses_a_bottom_right_toast_without_stealing_focus() {
        let (_t, mut a) = app();
        let mut term = Terminal::new(ratatui::backend::TestBackend::new(120, 35)).unwrap();
        term.draw(|f| a.draw(f)).unwrap();
        let workspace = term.backend().buffer().clone();
        let (tx, rx) = mpsc::channel::<Result<Work>>();
        a.launch_job(JobKind::Preparation, "Capture HOME → source", move || {
            rx.recv().unwrap()
        });
        let non_modal = a.modal.is_none();
        tx.send(Err(anyhow::anyhow!("cancelled fixture"))).unwrap();
        assert!(
            non_modal,
            "preparation must leave the workspace interactive"
        );
        term.draw(|f| a.draw(f)).unwrap();
        let bounds = a.areas.toast.bounds;
        assert!(
            bounds.x > 60 && bounds.y > 25,
            "toast must be at the bottom right"
        );
        assert!(bounds.width <= 48 && bounds.height <= 6);
        for y in 0..35 {
            for x in 0..120 {
                if !bounds.contains(Position::new(x, y)) {
                    assert_eq!(
                        term.backend().buffer()[(x, y)],
                        workspace[(x, y)],
                        "toast must not dim or replace the workspace"
                    );
                }
            }
        }
        let text = term
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text.contains("Preparing review"));
        assert!(text.contains("Capture HOME → source"));
        if let Ok(path) = std::env::var("DOT_TOAST_RENDER_PATH") {
            let rows: Vec<_> = term.backend().buffer().content.chunks(120).map(|row| {
                row.iter().map(|cell| serde_json::json!({
                    "text": cell.symbol(), "fg": format!("{:?}", cell.fg), "bg": format!("{:?}", cell.bg),
                })).collect::<Vec<_>>()
            }).collect();
            fs::write(path, serde_json::to_vec(&rows).unwrap()).unwrap();
        }
        a.key(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE));
        assert_eq!(
            a.pane,
            Pane::Git,
            "preparation must not trap workspace navigation"
        );
        for (w, h) in [(80, 24), (45, 14), (20, 8)] {
            term.backend_mut().resize(w, h);
            term.resize(Rect::new(0, 0, w, h)).unwrap();
            term.draw(|f| a.draw(f)).unwrap();
            assert!(a.areas.toast.bounds.right() >= w - 1);
            assert!(a.areas.toast.bounds.bottom() >= h - 3);
            assert!(
                a.areas.toast.bounds.bottom() <= h - 2,
                "keep footer controls available"
            );
        }
        let cancel = a.areas.toast.cancel.unwrap();
        a.mouse(event::MouseEvent {
            kind: MouseEventKind::Down(event::MouseButton::Left),
            column: cancel.x,
            row: cancel.y,
            modifiers: KeyModifiers::NONE,
        });
        assert!(a.job.as_ref().unwrap().discard);
        assert_eq!(
            a.pane,
            Pane::Git,
            "toast clicks must not reach the panel underneath"
        );
        term.draw(|f| a.draw(f)).unwrap();
        assert!(a.areas.toast.bounds.is_empty());
        a.modal = Some(Modal::Menu(0));
        let deadline = Instant::now() + Duration::from_secs(2);
        while a.job.is_some() && Instant::now() < deadline {
            a.pump();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            matches!(a.modal, Some(Modal::Menu(0))),
            "cancelled preparation must not show its result"
        );
    }

    #[test]
    fn ready_review_waits_for_search_or_another_form_to_close() {
        for search in [true, false] {
            let (_t, mut a) = app();
            a.preview_key = a.current_key();
            let (tx, rx) = mpsc::channel::<Result<Work>>();
            a.launch_job(JobKind::Preparation, "Capture", move || rx.recv().unwrap());
            if search {
                a.searching = true;
            } else {
                a.modal = Some(Modal::Input {
                    purpose: Purpose::Commit,
                    text: String::new(),
                });
            }
            tx.send(Ok(Work::Plan(Plan {
                id: "ready".into(),
                action: Action::Capture,
                paths: vec![".untracked".into()],
                payload: None,
                fingerprint: String::new(),
                created_at: String::new(),
                preview: String::new(),
            })))
            .unwrap();
            if let Some(job) = &mut a.job {
                job.toast_started = Some(Instant::now() - TOAST_MIN_VISIBLE);
            }
            let deadline = Instant::now() + Duration::from_secs(2);
            while a.pending_review.is_none() && Instant::now() < deadline {
                a.pump();
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(a.pending_review.is_some());
            a.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
            assert!(
                a.job.is_none(),
                "typing y must not approve the pending operation"
            );
            if search {
                assert_eq!(a.query, "y");
                a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
            } else {
                assert!(matches!(a.modal, Some(Modal::Input { ref text, .. }) if text == "y"));
                a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
            }
            a.pump();
            assert!(matches!(a.modal, Some(Modal::Review { ref plan, .. }) if plan.id == "ready"));
            assert!(a.pending_review.is_none());
        }
    }

    #[test]
    fn slow_background_work_uses_toasts_that_disappear_on_completion() {
        for (kind, title) in [
            (JobKind::Inspection, "Refreshing"),
            (JobKind::Preview, "Loading details"),
        ] {
            let (_t, mut a) = app();
            let (tx, rx) = mpsc::channel::<Result<Work>>();
            a.launch_job(kind, "Background fixture", move || rx.recv().unwrap());
            let mut term = Terminal::new(ratatui::backend::TestBackend::new(120, 35)).unwrap();
            term.draw(|f| a.draw(f)).unwrap();
            assert!(
                a.areas.toast.bounds.is_empty(),
                "fast refreshes should not flash a toast"
            );
            a.job.as_mut().unwrap().started = Instant::now() - Duration::from_millis(300);
            term.draw(|f| a.draw(f)).unwrap();
            assert!(a.modal.is_none());
            assert!(a.areas.toast.bounds.x > 60);
            let text = term
                .backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect::<String>();
            assert!(text.contains(title));
            assert!(a.areas.toast.cancel.is_none());
            let result = if kind == JobKind::Inspection {
                Work::Snapshot(a.snapshot.clone())
            } else {
                Work::Preview {
                    key: a.current_key(),
                    text: "Ready".into(),
                }
            };
            a.job.as_mut().unwrap().toast_started = Some(Instant::now() - TOAST_MIN_VISIBLE);
            tx.send(Ok(result)).unwrap();
            // Open help while the worker finishes; it must not be replaced by status UI.
            a.modal = Some(Modal::Menu(0));
            let deadline = Instant::now() + Duration::from_secs(2);
            while a.job.is_some() && Instant::now() < deadline {
                a.pump();
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(matches!(a.modal, Some(Modal::Menu(0))));
            a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
            term.draw(|f| a.draw(f)).unwrap();
            assert!(a.areas.toast.bounds.is_empty());
        }
    }

    #[test]
    fn cancelled_preparation_never_reopens_a_dialog() {
        let (_t, mut a) = app();
        let (tx, rx) = mpsc::channel::<Result<Work>>();
        a.launch_job(JobKind::Preparation, "Preparing capture", move || {
            rx.recv().unwrap()
        });
        assert!(a.modal.is_none());
        a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(a.modal.is_none());
        a.modal = Some(Modal::Menu(2));
        tx.send(Ok(Work::Plan(Plan {
            id: "abandoned".into(),
            action: Action::Capture,
            paths: vec![".untracked".into()],
            payload: None,
            fingerprint: String::new(),
            created_at: String::new(),
            preview: String::new(),
        })))
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while a.job.is_some() && Instant::now() < deadline {
            a.pump();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(a.job.is_none());
        assert!(matches!(a.modal, Some(Modal::Menu(2))));
        assert!(!a.config.source.join("dot_untracked").exists());
    }

    #[test]
    fn removing_untracked_entries_shows_a_compact_nothing_to_remove_message() {
        let (_t, mut a) = app();
        initialize(&a.config, &a.config_path, None).unwrap();
        fs::create_dir(a.config.home.join("scratch")).unwrap();
        fs::write(a.config.home.join("scratch/keep.txt"), "keep original").unwrap();
        a.snapshot = backend::snapshot(&a.config).unwrap();
        a.rebuild();
        for (path, key, subject) in [
            ("scratch", KeyCode::Char('r'), "This folder"),
            (".untracked", KeyCode::Char('r'), "This file"),
        ] {
            a.cursor = a.rows.iter().position(|row| row.path == path).unwrap();
            a.key(KeyEvent::new(key, KeyModifiers::NONE));
            if let Some(job) = &mut a.job {
                job.toast_started = Some(Instant::now() - TOAST_MIN_VISIBLE);
            }
            let deadline = Instant::now() + Duration::from_secs(5);
            while a
                .job
                .as_ref()
                .is_some_and(|job| job.kind == JobKind::Preparation)
                && Instant::now() < deadline
            {
                a.pump();
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(
                matches!(a.modal, Some(Modal::Text { ref title, ref text, .. })
                if title == "Nothing to remove" && text.starts_with(subject) && text.contains("no files tracked by dot"))
            );
            let mut term = Terminal::new(ratatui::backend::TestBackend::new(120, 35)).unwrap();
            term.draw(|f| a.draw(f)).unwrap();
            assert!(a.areas.modal.height < 12, "no-op message should be compact");
            assert!(a.areas.modal.width < 100);
            a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
            assert!(a.modal.is_none());
            assert_eq!(a.rows[a.cursor].path, path);
        }
        assert_eq!(
            fs::read_to_string(a.config.home.join("scratch/keep.txt")).unwrap(),
            "keep original"
        );
        assert_eq!(
            fs::read_to_string(a.config.home.join(".untracked")).unwrap(),
            "untracked"
        );

        // An untracked item must not suppress removal of tracked files in the same selection.
        fs::write(a.config.source.join("dot_tracked"), "saved").unwrap();
        fs::write(a.config.home.join(".tracked"), "original").unwrap();
        a.selected = ["scratch".into(), ".tracked".into()].into();
        a.menu_action(3);
        if let Some(job) = &mut a.job {
            job.toast_started = Some(Instant::now() - TOAST_MIN_VISIBLE);
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while a
            .job
            .as_ref()
            .is_some_and(|job| job.kind == JobKind::Preparation)
            && Instant::now() < deadline
        {
            a.pump();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            matches!(a.modal, Some(Modal::Review { ref plan, .. }) if plan.paths == [".tracked"])
        );
        assert_eq!(
            fs::read_to_string(a.config.source.join("dot_tracked")).unwrap(),
            "saved"
        );
    }

    #[test]
    fn background_failures_do_not_replace_an_open_dialog() {
        for kind in [JobKind::Inspection, JobKind::Preview] {
            let (_t, mut a) = app();
            a.modal = Some(Modal::Report {
                title: "Result".into(),
                text: "Completed".into(),
                scroll: 0,
            });
            let (tx, rx) = mpsc::channel::<Result<Work>>();
            a.launch_job(kind, "Refreshing background state", move || {
                rx.recv().unwrap()
            });
            tx.send(Err(anyhow::anyhow!("Fixture background failure")))
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(2);
            while a.job.is_some() && Instant::now() < deadline {
                a.pump();
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(a.job.is_none());
            assert!(matches!(a.modal, Some(Modal::Report { ref text, .. }) if text == "Completed"));
        }
    }

    #[test]
    fn operation_errors_and_source_editor_resize_inside_dialogs() {
        let (_t, mut a) = app();
        let (tx, rx) = mpsc::channel::<Result<Work>>();
        a.launch("Executing operation", move || rx.recv().unwrap());
        let mut term = Terminal::new(ratatui::backend::TestBackend::new(120, 35)).unwrap();
        term.draw(|f| a.draw(f)).unwrap();
        let bounds = a.areas.modal;
        for key in [KeyCode::Esc, KeyCode::Char('y'), KeyCode::Enter] {
            a.key(KeyEvent::new(key, KeyModifiers::NONE));
            assert!(matches!(a.modal, Some(Modal::Busy { .. })));
        }
        tx.send(Err(anyhow::anyhow!("Fixture operation failed")))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while a.job.is_some() && Instant::now() < deadline {
            a.pump();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            matches!(a.modal, Some(Modal::Report { ref text, .. }) if text.contains("Fixture operation failed"))
        );
        term.draw(|f| a.draw(f)).unwrap();
        assert_eq!(a.areas.modal, bounds);
        a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        a.modal = Some(Modal::Input {
            purpose: Purpose::Source(".zshrc".into()),
            text: "héllo\n".repeat(100),
        });
        a.input_cursor = 600;
        term.draw(|f| a.draw(f)).unwrap();
        assert_eq!(a.areas.modal, bounds);
        a.key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE));
        term.draw(|f| a.draw(f)).unwrap();
        assert_eq!(a.areas.modal, Rect::new(0, 0, 120, 35));
        a.key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE));
        term.draw(|f| a.draw(f)).unwrap();
        assert_eq!(a.areas.modal, bounds);
        for (w, h) in [(45, 14), (20, 8)] {
            term.backend_mut().resize(w, h);
            term.resize(Rect::new(0, 0, w, h)).unwrap();
            term.draw(|f| a.draw(f)).unwrap();
            let text = term
                .backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect::<String>();
            assert!(
                text.contains('▏'),
                "editor cursor lost after resize at {w}x{h}"
            );
        }
    }
    #[test]
    fn tab_headers_are_clickable_and_keep_active_tab_visible_when_narrow() {
        let (_t, mut a) = app();
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(120, 35)).unwrap();
        terminal.draw(|f| a.draw(f)).unwrap();
        let (rect, _, _) = a
            .areas
            .tabs
            .iter()
            .find(|(_, pane, i)| *pane == Pane::Git && *i == 1)
            .copied()
            .unwrap();
        assert_eq!(
            terminal.backend().buffer()[(rect.x, rect.y)].symbol(),
            "C",
            "click target must align with the rendered Commits label"
        );
        a.mouse(event::MouseEvent {
            kind: MouseEventKind::Down(event::MouseButton::Left),
            column: rect.x,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(a.pane, Pane::Git);
        assert!(a.history_view);
        assert_eq!(a.detail_tab, 0);
        for key in [' ', 'v', 'd'] {
            a.key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE));
        }
        assert!(a.job.is_none());
        assert!(a.modal.is_none());
        a.pane = Pane::Details;
        a.set_tab(Pane::Details, 3);
        let mut narrow = Terminal::new(ratatui::backend::TestBackend::new(20, 8)).unwrap();
        narrow.draw(|f| a.draw(f)).unwrap();
        let text = narrow
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text.contains("Activity"));
        assert!(
            a.areas
                .tabs
                .iter()
                .any(|(_, pane, index)| *pane == Pane::Details && *index == 3)
        );
    }
    #[test]
    fn restore_is_uppercase_r_and_refresh_uses_control_r() {
        for (key, kind, label) in [
            (
                KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE),
                JobKind::Preparation,
                Action::Restore.label(),
            ),
            (
                KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
                JobKind::Inspection,
                "Inspecting chezmoi and Git",
            ),
        ] {
            let (_t, mut a) = app();
            a.config.chezmoi = "/not/a/command".into();
            a.key(key);
            let job = a
                .job
                .take()
                .expect("shortcut must launch the intended action");
            assert!(job.kind == kind);
            assert_eq!(job.label, label);
            let _ = job.rx.recv_timeout(Duration::from_secs(2)).unwrap();
        }
    }

    #[test]
    fn delete_original_shortcuts_review_trash_and_cancel_without_moving_files() {
        let (_t, mut a) = app();
        initialize(&a.config, &a.config_path, None).unwrap();
        fs::create_dir(a.config.home.join("scratch")).unwrap();
        fs::write(
            a.config.home.join("scratch/original.txt"),
            "keep until confirmed",
        )
        .unwrap();
        a.snapshot = backend::snapshot(&a.config).unwrap();
        a.rebuild();
        a.cursor = a.rows.iter().position(|row| row.path == "scratch").unwrap();
        for key in [KeyCode::Char('d'), KeyCode::Delete] {
            a.key(KeyEvent::new(key, KeyModifiers::NONE));
            a.job.as_mut().unwrap().toast_started = Some(Instant::now() - TOAST_MIN_VISIBLE);
            let deadline = Instant::now() + Duration::from_secs(5);
            while a.job.is_some() && Instant::now() < deadline {
                a.pump();
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(matches!(a.modal, Some(Modal::Review { ref plan, .. })
                if plan.action == Action::Trash && plan.paths == ["scratch"]));
            for (w, h) in [(120, 35), (80, 24), (45, 14), (20, 8)] {
                let mut term = Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
                term.draw(|f| a.draw(f)).unwrap();
                let text = term
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .map(|c| c.symbol())
                    .collect::<String>();
                assert!(
                    text.contains("Enter/y Trash"),
                    "Trash control missing at {w}x{h}"
                );
                assert!(
                    a.areas
                        .operation
                        .controls
                        .iter()
                        .any(|(_, key)| *key == KeyCode::Esc)
                );
                if w >= 80 {
                    assert!(text.contains("Only untracked originals"));
                }
            }
            a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
            assert!(a.modal.is_none());
            assert_eq!(
                fs::read_to_string(a.config.home.join("scratch/original.txt")).unwrap(),
                "keep until confirmed"
            );
        }
    }

    #[test]
    fn focused_panel_has_a_bright_double_border_and_remove_is_discoverable() {
        let (_t, mut a) = app();
        let mut term = Terminal::new(ratatui::backend::TestBackend::new(120, 35)).unwrap();
        term.draw(|f| a.draw(f)).unwrap();
        let files = a.areas.files;
        let git = a.areas.git;
        let edge = |r: Rect| (r.right() - 1, r.y + 1);
        assert_eq!(term.backend().buffer()[edge(files)].fg, FOCUS_BORDER);
        assert_eq!(term.backend().buffer()[edge(files)].symbol(), "║");
        assert_ne!(term.backend().buffer()[edge(git)].fg, FOCUS_BORDER);
        a.mouse(event::MouseEvent {
            kind: MouseEventKind::Down(event::MouseButton::Left),
            column: git.right() - 1,
            row: git.y + 1,
            modifiers: KeyModifiers::NONE,
        });
        term.draw(|f| a.draw(f)).unwrap();
        assert_eq!(term.backend().buffer()[edge(git)].fg, FOCUS_BORDER);
        assert_eq!(term.backend().buffer()[edge(git)].symbol(), "║");
        assert_ne!(term.backend().buffer()[edge(files)].fg, FOCUS_BORDER);
        a.pane = Pane::Files;
        for (w, h) in [(120, 35), (80, 24), (45, 14)] {
            let mut term = Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
            term.draw(|f| a.draw(f)).unwrap();
            assert!(
                a.areas
                    .footer
                    .iter()
                    .any(|(_, key)| *key == KeyCode::Char('r')),
                "Remove must stay visible at {w}x{h}"
            );
        }
    }

    #[test]
    fn remove_saved_copy_requires_review_and_preserves_original_file() {
        let (_t, mut a) = app();
        initialize(&a.config, &a.config_path, None).unwrap();
        let plan =
            operations::plan(&a.config, Action::Capture, &[".untracked".into()], None).unwrap();
        operations::execute(&mut a.config, &a.config_path, &plan, &plan.id).unwrap();
        a.selected.insert(".untracked".into());
        let mut term = Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        term.draw(|f| a.draw(f)).unwrap();
        let button = a
            .areas
            .footer
            .iter()
            .find(|(_, key)| *key == KeyCode::Char('r'))
            .unwrap()
            .0;
        a.mouse(event::MouseEvent {
            kind: MouseEventKind::Down(event::MouseButton::Left),
            column: button.x,
            row: button.y,
            modifiers: KeyModifiers::NONE,
        });
        if let Some(job) = &mut a.job {
            job.toast_started = Some(Instant::now() - TOAST_MIN_VISIBLE);
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while a.job.is_some() && Instant::now() < deadline {
            a.pump();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            matches!(a.modal, Some(Modal::Review { ref plan, .. }) if plan.action == Action::Forget && plan.paths == [".untracked"])
        );
        term.draw(|f| a.draw(f)).unwrap();
        let text = term
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(text.contains("Your original files stay"));
        a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(
            a.config.source.join("dot_untracked").exists(),
            "cancel must preserve the saved copy"
        );
        a.key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
        if let Some(job) = &mut a.job {
            job.toast_started = Some(Instant::now() - TOAST_MIN_VISIBLE);
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while a.job.is_some() && Instant::now() < deadline {
            a.pump();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(matches!(a.modal, Some(Modal::Review { .. })));
        a.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        if let Some(job) = &mut a.job {
            job.toast_started = Some(Instant::now() - TOAST_MIN_VISIBLE);
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while !matches!(a.modal, Some(Modal::Outcome { .. })) && Instant::now() < deadline {
            a.pump();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            matches!(a.modal, Some(Modal::Outcome { ref outcome, .. }) if outcome.failed.is_none() && outcome.action == Action::Forget)
        );
        assert!(!a.config.source.join("dot_untracked").exists());
        assert_eq!(
            fs::read_to_string(a.config.home.join(".untracked")).unwrap(),
            "untracked"
        );
    }

    #[test]
    fn mouse_focus_changes_only_on_left_click() {
        let (_t, mut a) = app();
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(120, 35)).unwrap();
        terminal.draw(|f| a.draw(f)).unwrap();
        for (pane, area) in [
            (Pane::Status, a.areas.status),
            (Pane::Git, a.areas.git),
            (Pane::Machines, a.areas.machines),
            (Pane::Activity, a.areas.activity),
            (Pane::Details, a.areas.details),
            (Pane::Files, a.areas.files),
        ] {
            a.pane = Pane::Files;
            a.detail_owner = Pane::Files;
            let event = event::MouseEvent {
                kind: MouseEventKind::Moved,
                column: area.x + area.width / 2,
                row: area.y + area.height / 2,
                modifiers: KeyModifiers::NONE,
            };
            for kind in [
                MouseEventKind::Moved,
                MouseEventKind::Drag(event::MouseButton::Left),
                MouseEventKind::Up(event::MouseButton::Left),
                MouseEventKind::Down(event::MouseButton::Right),
                MouseEventKind::Down(event::MouseButton::Middle),
                MouseEventKind::ScrollDown,
                MouseEventKind::ScrollUp,
                MouseEventKind::ScrollLeft,
                MouseEventKind::ScrollRight,
            ] {
                a.mouse(event::MouseEvent { kind, ..event });
                assert_eq!(a.pane, Pane::Files, "{kind:?} over {pane:?} stole focus");
                assert_eq!(
                    a.detail_owner,
                    Pane::Files,
                    "{kind:?} changed the detail context"
                );
            }
            a.mouse(event::MouseEvent {
                kind: MouseEventKind::Down(event::MouseButton::Left),
                ..event
            });
            assert_eq!(a.pane, pane, "click must focus {pane:?}");
        }
        a.pane = Pane::Files;
        a.detail_owner = Pane::Files;
        a.scroll = 0;
        a.mouse(event::MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: a.areas.details.x + 2,
            row: a.areas.details.y + 2,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(
            a.scroll, 3,
            "wheel must still scroll the pane under the pointer"
        );
        assert_eq!(a.pane, Pane::Files);
        assert_eq!(a.detail_owner, Pane::Files);
    }

    #[test]
    fn enter_and_y_both_confirm_operation_reviews_and_local_controls() {
        for key in [KeyCode::Char('y'), KeyCode::Enter] {
            let (_t, mut a) = app();
            initialize(&a.config, &a.config_path, None).unwrap();
            let plan =
                operations::plan(&a.config, Action::Capture, &[".untracked".into()], None).unwrap();
            a.modal = Some(Modal::Review {
                plan,
                view: OperationView::default(),
            });
            a.key(KeyEvent::new(key, KeyModifiers::NONE));
            assert!(
                matches!(a.modal, Some(Modal::Busy { .. })),
                "{key:?} must confirm the review"
            );
            let deadline = Instant::now() + Duration::from_secs(5);
            while !matches!(a.modal, Some(Modal::Outcome { .. })) && Instant::now() < deadline {
                a.pump();
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(
                matches!(a.modal, Some(Modal::Outcome { ref outcome, .. }) if outcome.failed.is_none())
            );
            assert_eq!(
                fs::read_to_string(a.config.source.join("dot_untracked")).unwrap(),
                "untracked"
            );
            // Finish background inspection before removing the fixture.
            if let Some(job) = a.job.take() {
                let _ = job.rx.recv_timeout(Duration::from_secs(5)).unwrap();
            }

            let (_t, mut a) = app();
            a.config.chezmoi = "/not/a/command".into();
            a.config.save(&a.config_path).unwrap();
            a.modal = Some(Modal::Confirm {
                command: Control::Policy("trusted".into(), true),
                text: "Enable trusted mode?".into(),
                scroll: 0,
            });
            a.key(KeyEvent::new(key, KeyModifiers::NONE));
            assert!(
                matches!(a.modal, Some(Modal::Busy { .. })),
                "{key:?} must confirm the local action"
            );
            let deadline = Instant::now() + Duration::from_secs(5);
            while !a.config.trusted && Instant::now() < deadline {
                a.pump();
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(Config::load(&a.config_path).unwrap().trusted);
            if let Some(job) = a.job.take() {
                let _ = job.rx.recv_timeout(Duration::from_secs(5)).unwrap();
            }
        }
    }

    #[test]
    fn capture_review_requires_confirmation_and_result_routes_to_git() {
        let (_t, mut a) = app();
        initialize(&a.config, &a.config_path, None).unwrap();
        let plan =
            operations::plan(&a.config, Action::Capture, &[".untracked".into()], None).unwrap();
        a.modal = Some(Modal::Review {
            plan,
            view: OperationView::default(),
        });
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        terminal.draw(|f| a.draw(f)).unwrap();
        assert!(a.job.is_none(), "rendering a review must not execute it");
        assert!(!a.config.source.join("dot_untracked").exists());
        // Clicking a tab switches views without executing anything.
        let changes = a
            .areas
            .operation
            .controls
            .iter()
            .find(|(_, key)| *key == KeyCode::Char('2'))
            .unwrap()
            .0;
        a.mouse(event::MouseEvent {
            kind: MouseEventKind::Down(event::MouseButton::Left),
            column: changes.x,
            row: changes.y,
            modifiers: KeyModifiers::NONE,
        });
        assert!(matches!(a.modal, Some(Modal::Review { ref view, .. }) if view.tab == 1));
        assert!(a.job.is_none());
        terminal.draw(|f| a.draw(f)).unwrap();
        let review_bounds = a.areas.modal;
        let confirm = a
            .areas
            .operation
            .controls
            .iter()
            .find(|(_, key)| *key == KeyCode::Char('y'))
            .unwrap()
            .0;
        a.mouse(event::MouseEvent {
            kind: MouseEventKind::Down(event::MouseButton::Left),
            column: confirm.x,
            row: confirm.y,
            modifiers: KeyModifiers::NONE,
        });
        assert!(matches!(a.modal, Some(Modal::Busy { .. })));
        a.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
        a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        terminal.draw(|f| a.draw(f)).unwrap();
        assert_eq!(a.areas.modal, review_bounds);
        assert!(matches!(a.modal, Some(Modal::Busy { .. })));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !matches!(a.modal, Some(Modal::Outcome { .. })) && Instant::now() < deadline {
            a.pump();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            matches!(a.modal, Some(Modal::Outcome { ref outcome, .. }) if outcome.failed.is_none() && outcome.succeeded == [".untracked"])
        );
        terminal.draw(|f| a.draw(f)).unwrap();
        assert_eq!(a.areas.modal, review_bounds);
        assert_eq!(
            fs::read_to_string(a.config.source.join("dot_untracked")).unwrap(),
            "untracked"
        );
        assert_eq!(
            fs::read_to_string(a.config.home.join(".untracked")).unwrap(),
            "untracked"
        );
        a.key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE));
        assert!(matches!(a.modal, Some(Modal::Outcome { ref view, .. }) if view.tab == 1));
        a.key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
        assert!(a.modal.is_none());
        assert_eq!(a.pane, Pane::Git);
    }

    #[test]
    fn operation_screens_render_long_lists_failures_and_narrow_terminals() {
        let (_t, mut a) = app();
        a.config.source = PathBuf::from("/Users/example/.local/share/chezmoi");
        let paths: Vec<String> = [
            ".agents/skills/dot/SKILL.md",
            ".config/ghostty/config",
            ".config/nvim/init.lua",
            ".config/nvim/lua/keymaps.lua",
            ".config/nvim/lua/options.lua",
            ".config/starship.toml",
            ".gitconfig",
            ".zshrc",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let plan = Plan {
            id: "preview-fixture".into(), action: Action::Capture, paths: paths.clone(),
            payload: None, fingerprint: String::new(), created_at: String::new(),
            preview: "diff --git a/.config/ghostty/config b/.config/ghostty/config\n--- a/.config/ghostty/config\n+++ b/.config/ghostty/config\n@@ -1,3 +1,4 @@\n-font-size = 14\n+font-size = 16\n theme = catppuccin-mocha\n+window-padding-x = 12\n\nNEW source from HOME: .config/starship.toml\nadd_newline = false\n\n[character]\nsuccess_symbol = \"[>](bold green)\"\n".into(),
        };
        let outcome = Outcome {
            id: plan.id.clone(),
            action: Action::Capture,
            succeeded: paths.clone(),
            failed: None,
            not_attempted: vec![],
            message: String::new(),
        };
        let partial = Outcome {
            succeeded: paths[..2].to_vec(),
            failed: Some(paths[2].clone()),
            not_attempted: paths[3..].to_vec(),
            message: "Permission denied while reading init.lua".into(),
            ..outcome.clone()
        };
        for (name, modal) in [
            ("trash-review", Modal::Review {
                plan: Plan { action: Action::Trash, paths: vec!["Downloads/old-file.txt".into(), "scratch".into()],
                    preview: "FILE  /Users/example/Downloads/old-file.txt\nFOLDER  /Users/example/scratch\nFolders include all contents. Recover using system Trash.".into(), ..plan.clone() },
                view: OperationView::default(),
            }),
            ("trash-result", Modal::Outcome {
                outcome: Outcome { action: Action::Trash, succeeded: vec!["Downloads/old-file.txt".into(), "scratch".into()],
                    message: "Originals moved to system Trash. Restore them using your system Trash.".into(), ..outcome.clone() },
                view: OperationView::default(),
            }),
            (
                "capture-files",
                Modal::Review {
                    plan: plan.clone(),
                    view: OperationView::default(),
                },
            ),
            (
                "capture-changes",
                Modal::Review {
                    plan: plan.clone(),
                    view: OperationView { tab: 1, scroll: 0 },
                },
            ),
            (
                "capture-result",
                Modal::Outcome {
                    outcome: outcome.clone(),
                    view: OperationView::default(),
                },
            ),
            (
                "capture-partial",
                Modal::Outcome {
                    outcome: partial,
                    view: OperationView::default(),
                },
            ),
        ] {
            for (w, h) in [(120, 35), (80, 24), (45, 14), (20, 8)] {
                a.modal = Some(modal.clone());
                let mut term = Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
                term.draw(|f| a.draw(f)).unwrap();
                let bounds = a.areas.modal;
                let text = (bounds.y..bounds.bottom())
                    .flat_map(|y| (bounds.x..bounds.right()).map(move |x| (x, y)))
                    .map(|pos| term.backend().buffer()[pos].symbol())
                    .collect::<String>();
                assert!(!text.contains("\"succeeded\""));
                assert!(!text.contains("Space Select"));
                assert!(
                    text.contains("Enter/y Capture") || text.contains("Enter/y Trash") || text.contains("Enter Done"),
                    "primary control must remain visible at {w}x{h}"
                );
                if w >= 80 {
                    assert!(text.contains("Esc Cancel") || text.contains("Enter Done"));
                    if name == "capture-partial" {
                        assert!(text.contains("Permission denied"));
                        assert!(text.contains("Capture stopped"));
                    }
                }
                if let Ok(dir) = std::env::var("DOT_OPERATION_RENDER_DIR") {
                    let rows: Vec<_> = term
                        .backend()
                        .buffer()
                        .content
                        .chunks(w as usize)
                        .map(|row| {
                            row.iter().map(|c| serde_json::json!({
                            "text":c.symbol(), "fg":format!("{:?}",c.fg), "bg":format!("{:?}",c.bg),
                            "bold":c.modifier.contains(Modifier::BOLD),
                        })).collect::<Vec<_>>()
                        })
                        .collect();
                    fs::write(
                        Path::new(&dir).join(format!("{name}-{w}x{h}.json")),
                        serde_json::to_vec(&rows).unwrap(),
                    )
                    .unwrap();
                }
                a.key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
                term.draw(|f| a.draw(f)).unwrap();
                let text = term
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .map(|c| c.symbol())
                    .collect::<String>();
                if name == "capture-files" && w >= 45 {
                    assert!(
                        text.contains("options.lua"),
                        "last target must remain reachable"
                    );
                }
                a.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
                assert!(match &a.modal {
                    Some(Modal::Review { view, .. } | Modal::Outcome { view, .. }) =>
                        view.scroll == 0,
                    _ => false,
                });
                a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
                assert!(a.modal.is_none());
            }
        }
    }

    #[test]
    fn setup_reports_created_existing_and_downloaded_without_applying_settings() {
        let (_t, a) = app();
        fs::remove_dir(&a.config.source).unwrap();
        let created = initialize(&a.config, &a.config_path, None).unwrap();
        assert_eq!(created.kind, SetupKind::Created);
        fs::write(a.config.source.join("dot_untracked"), "saved settings").unwrap();
        let existing = initialize(&a.config, &a.config_path, None).unwrap();
        assert_eq!(existing.kind, SetupKind::Existing);
        assert_eq!(
            fs::read_to_string(a.config.home.join(".untracked")).unwrap(),
            "untracked"
        );
        let mut downloaded = a.config.clone();
        downloaded.source = a.config.source.with_file_name("downloaded");
        let cloned = initialize(&downloaded, &a.config_path, a.config.source.to_str()).unwrap();
        assert_eq!(cloned.kind, SetupKind::Cloned);
        assert_eq!(
            fs::read_to_string(a.config.home.join(".untracked")).unwrap(),
            "untracked"
        );
    }

    #[test]
    fn welcome_guide_is_responsive_scrollable_and_opens_files() {
        let (_t, mut a) = app();
        // Include a long path to exercise wrapping independently of the terminal size.
        a.config.source = PathBuf::from("/Users/example/.local/share/a-long-folder-name/chezmoi");
        let report = SetupReport {
            kind: SetupKind::Created,
            details: "Agent instructions installed.".into(),
        };
        for (w, h) in [(120, 35), (80, 24), (45, 14), (20, 8)] {
            a.modal = Some(Modal::Welcome {
                report: report.clone(),
                scroll: 0,
            });
            let mut term = Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
            term.draw(|f| a.draw(f)).unwrap();
            let text = term
                .backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect::<String>();
            assert!(text.contains("Welcome"));
            assert!(text.contains("Open files"));
            assert!(!text.contains("HOME"));
            assert!(!text.contains("Space Select"));
            if w >= 45 {
                assert!(text.contains("Powered by:"));
            }
            if w >= 80 {
                assert!(text.contains("chezmoi"));
                assert!(text.contains("Git"));
            }
            if let Ok(dir) = std::env::var("DOT_WELCOME_RENDER_DIR") {
                let rows: Vec<_> = term.backend().buffer().content.chunks(w as usize)
                    .map(|row| row.iter().map(|c| serde_json::json!({"text":c.symbol(),"fg":format!("{:?}",c.fg),"bg":format!("{:?}",c.bg)})).collect::<Vec<_>>()).collect();
                fs::write(
                    Path::new(&dir).join(format!("welcome-{w}x{h}.json")),
                    serde_json::to_vec(&rows).unwrap(),
                )
                .unwrap();
            }
            for _ in 0..100 {
                a.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
            }
            term.draw(|f| a.draw(f)).unwrap();
            let text = term
                .backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect::<String>();
            if w >= 45 {
                assert!(text.contains("Enter or y to use them."));
            }
            let button = a.areas.setup[0].0;
            a.mouse(event::MouseEvent {
                kind: MouseEventKind::Down(event::MouseButton::Left),
                column: button.x,
                row: button.y,
                modifiers: KeyModifiers::NONE,
            });
            assert!(a.modal.is_none());
            assert_eq!(a.pane, Pane::Files);
        }
        a.modal = Some(Modal::Welcome {
            report: report.clone(),
            scroll: 0,
        });
        a.key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
        assert!(
            matches!(a.modal, Some(Modal::Text { ref text, .. }) if text.contains("Agent instructions installed."))
        );
        a.modal = Some(Modal::Welcome {
            report: report.clone(),
            scroll: 0,
        });
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(a.modal.is_none());
        a.modal = Some(Modal::Welcome { report, scroll: 0 });
        assert!(a.key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)));
    }

    #[test]
    fn setup_screen_routes_keyboard_and_mouse_to_existing_flows() {
        let (_t, mut a) = app();
        a.modal = Some(Modal::Setup);
        a.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        assert!(matches!(
            a.modal,
            Some(Modal::Input {
                purpose: Purpose::Repo,
                ..
            })
        ));
        a.modal = Some(Modal::Setup);
        a.key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE));
        assert!(matches!(
            a.modal,
            Some(Modal::Input {
                purpose: Purpose::Switch,
                ..
            })
        ));
        a.modal = Some(Modal::Setup);
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
        terminal.draw(|f| a.draw(f)).unwrap();
        let area = a
            .areas
            .setup
            .iter()
            .find(|(_, key)| *key == KeyCode::Char('c'))
            .unwrap()
            .0;
        a.mouse(event::MouseEvent {
            kind: MouseEventKind::Down(event::MouseButton::Left),
            column: area.x,
            row: area.y,
            modifiers: KeyModifiers::NONE,
        });
        assert!(matches!(
            a.modal,
            Some(Modal::Input {
                purpose: Purpose::Repo,
                ..
            })
        ));
        a.modal = Some(Modal::Setup);
        assert!(a.key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)));
    }
    #[test]
    fn startup_files_do_not_require_left_after_inspection_failure() {
        let (_t, fixture) = app();
        fs::remove_dir(&fixture.config.source).unwrap();
        let mut a = App::new(fixture.config, fixture.config_path);
        a.refresh();
        let deadline = Instant::now() + Duration::from_secs(2);
        while a.modal.is_none() && Instant::now() < deadline {
            a.pump();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(a.notice.contains("Source does not exist"));
        a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let initial: Vec<_> = a.rows.iter().map(|r| r.path.clone()).collect();
        a.key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        let after_left: Vec<_> = a.rows.iter().map(|r| r.path.clone()).collect();
        assert!(after_left.contains(&".untracked".to_string()));
        assert_eq!(
            initial, after_left,
            "Left must not be needed to load HOME files"
        );
    }
    #[test]
    fn startup_files_are_available_before_native_inspection() {
        let (_t, fixture) = app();
        let a = App::new(fixture.config, fixture.config_path);
        assert!(a.rows.iter().any(|r| r.path == ".untracked"));
    }
    #[test]
    fn dismissed_missing_source_error_does_not_restart_refresh() {
        let (_t, mut a) = app();
        fs::remove_dir(&a.config.source).unwrap();
        a.preview_key = a.current_key();
        a.last_refresh = Instant::now() - Duration::from_secs(16);
        a.refresh();
        let deadline = Instant::now() + Duration::from_secs(2);
        while a.modal.is_none() && Instant::now() < deadline {
            a.pump();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(a.notice.contains("Source does not exist"));
        assert!(a.modal.is_some());
        a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        a.pump();
        assert!(a.modal.is_none(), "dismissed error reopened");
        assert!(
            a.job.is_none(),
            "dismissal immediately retried the failed refresh"
        );
        assert!(a.key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)));
        a.last_refresh = Instant::now() - Duration::from_secs(30);
        a.pump();
        assert!(a.job.is_none(), "automatic retries must remain paused");
        a.key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        assert!(a.job.is_some(), "manual refresh must remain available");
    }
    #[test]
    fn can_quit_from_error_screen() {
        let (_t, mut a) = app();
        a.modal = Some(Modal::Text {
            title: "Needs attention".into(),
            text: "Source does not exist".into(),
            scroll: 0,
        });
        assert!(a.key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)));
    }
    #[test]
    fn quit_during_inspection_but_not_during_mutation() {
        let (_t, mut a) = app();
        let (_tx, rx) = mpsc::channel();
        a.job = Some(Job {
            rx,
            label: "Inspection".into(),
            kind: JobKind::Inspection,
            discard: false,
            started: Instant::now(),
            toast_started: None,
        });
        assert!(a.key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)));
        a.searching = true;
        assert!(a.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)));
        a.searching = false;
        a.job.as_mut().unwrap().kind = JobKind::Operation;
        assert!(!a.key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)));
        assert!(!a.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)));
    }
    #[test]
    fn search_owns_q_digits_and_shortcuts() {
        let (_t, mut a) = app();
        a.key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        for c in "q2Wtconfig".chars() {
            assert!(!a.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)));
        }
        assert_eq!(a.query, "q2Wtconfig");
        assert_eq!(a.filter, Filter::All);
        assert_eq!(a.pane, Pane::Files);
    }
    #[test]
    fn navigating_panels_never_transfers_file_selection() {
        let (_t, mut a) = app();
        a.key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        let old = a.selected.clone();
        a.key(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE));
        assert_eq!(a.selected, old);
        assert!(a.git_selected.is_empty());
        assert!(a.modal.is_none());
    }
    #[test]
    fn responsive_workspace_renders_and_pointer_actions_match() {
        let (_t, mut a) = app();
        for (w, h) in [(120, 35), (80, 24), (45, 14), (20, 8)] {
            let b = ratatui::backend::TestBackend::new(w, h);
            let mut term = Terminal::new(b).unwrap();
            term.draw(|f| a.draw(f)).unwrap();
            let buf = term.backend().buffer();
            let text = buf.content.iter().map(|c| c.symbol()).collect::<String>();
            assert!(text.contains("Files"));
            if w >= 80 && h >= 22 {
                assert!(text.contains("Status"));
                assert!(text.contains("Machines"));
                assert!(text.contains("Details"));
            }
        }
        let mut term = Terminal::new(ratatui::backend::TestBackend::new(120, 35)).unwrap();
        term.draw(|f| a.draw(f)).unwrap();
        let filter = a
            .areas
            .footer
            .iter()
            .find(|(_, k)| *k == KeyCode::Char('t'))
            .unwrap()
            .0;
        a.mouse(event::MouseEvent {
            kind: MouseEventKind::Down(event::MouseButton::Left),
            column: filter.x,
            row: filter.y,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(a.filter, Filter::Managed);
        assert!(a.rows.iter().all(|row| row.managed));
        assert!(!a.rows.iter().any(|row| row.path == ".untracked"));
        a.key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
        assert_eq!(a.filter, Filter::All);
        assert!(a.rows.iter().any(|row| row.path == ".untracked"));
        a.set_tab(Pane::Files, 2);
        a.key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
        assert_eq!(a.filter, Filter::Managed);
        a.pane = Pane::Git;
        a.key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE));
        assert_eq!(a.filter, Filter::Managed);
    }
    #[test]
    fn details_assignments_support_clicks_forms_review_cancel_and_save() {
        let (_temp, mut a) = app();
        initialize(&a.config, &a.config_path, None).unwrap();
        let capture =
            operations::plan(&a.config, Action::Capture, &[".untracked".into()], None).unwrap();
        operations::execute(&mut a.config, &a.config_path, &capture, &capture.id).unwrap();
        a.snapshot = backend::snapshot(&a.config).unwrap();
        a.rebuild();
        a.cursor = a
            .rows
            .iter()
            .position(|row| row.path == ".untracked")
            .unwrap();
        a.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
        assert_eq!((a.pane, a.detail_tab), (Pane::Details, 2));
        let mut term = Terminal::new(ratatui::backend::TestBackend::new(120, 35)).unwrap();
        term.draw(|f| a.draw(f)).unwrap();
        let os = if a.snapshot.identity.os == "darwin" {
            assignments::Os::Linux
        } else {
            assignments::Os::Darwin
        };
        let button = a
            .areas
            .assignment
            .controls
            .iter()
            .find(|(_, command)| matches!(command, AssignmentCommand::Os(value) if *value == os))
            .unwrap()
            .0;
        a.mouse(event::MouseEvent {
            kind: MouseEventKind::Down(event::MouseButton::Left),
            column: button.x,
            row: button.y,
            modifiers: KeyModifiers::NONE,
        });
        a.key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE));
        assert!(
            a.assignment_scope(".untracked")
                .machines
                .contains(&a.snapshot.identity.hostname)
        );
        a.key(KeyEvent::new(KeyCode::Char('M'), KeyModifiers::NONE));
        assert!(matches!(
            a.modal,
            Some(Modal::Input {
                purpose: Purpose::AssignmentMachines(_),
                ..
            })
        ));
        if let Some(Modal::Input { text, .. }) = &mut a.modal {
            *text = "laptop.local, desktop.company".into();
        }
        a.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(a.modal.is_none());
        assert_eq!(
            a.assignment_scope(".untracked").machines,
            BTreeSet::from(["laptop".into(), "desktop".into()])
        );
        for selected in [true, false] {
            term.draw(|f| a.draw(f)).unwrap();
            let button = a.areas.assignment.controls.iter().find(|(_, command)|
                matches!(command, AssignmentCommand::ToggleMachine(name) if name == &a.snapshot.identity.hostname)).unwrap().0;
            a.mouse(event::MouseEvent {
                kind: MouseEventKind::Down(event::MouseButton::Left),
                column: button.x,
                row: button.y,
                modifiers: KeyModifiers::NONE,
            });
            assert_eq!(
                a.assignment_scope(".untracked")
                    .machines
                    .contains(&a.snapshot.identity.hostname),
                selected
            );
        }
        let mut renders = Vec::new();
        for (width, height) in [(120, 35), (80, 24), (45, 14), (20, 8), (8, 4), (1, 1)] {
            let mut terminal =
                Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            a.scroll = 0;
            terminal.draw(|f| a.draw(f)).unwrap();
            for (rect, _) in &a.areas.assignment.controls {
                assert!(a.areas.details.contains(Position::new(rect.x, rect.y)));
                assert!(rect.right() <= a.areas.details.right());
            }
            if width >= 20 {
                let text = terminal
                    .backend()
                    .buffer()
                    .content
                    .chunks(width as usize)
                    .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                    .collect::<Vec<_>>()
                    .join("\n");
                assert!(text.contains("Rules"));
                renders.push(format!("{width}×{height}\n{text}"));
            }
            a.key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
            terminal.draw(|f| a.draw(f)).unwrap();
            assert_eq!(a.scroll, a.areas.assignment.max_scroll);
        }
        if let Ok(path) = std::env::var("DOT_ASSIGNMENT_RENDER") {
            fs::write(path, renders.join("\n\n")).unwrap();
        }
        let before = fs::read(a.config.source.join(".chezmoiignore")).unwrap();
        for confirm in [false, true] {
            a.key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
            let deadline = Instant::now() + Duration::from_secs(5);
            while !matches!(a.modal, Some(Modal::Review { .. })) && Instant::now() < deadline {
                if let Some(job) = &mut a.job {
                    job.toast_started = Some(Instant::now() - TOAST_MIN_VISIBLE);
                }
                a.pump();
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(
                matches!(a.modal, Some(Modal::Review { ref plan, .. }) if plan.action == Action::Assign && plan.paths == [".untracked"]),
                "review missing: {:?}",
                a.modal
            );
            assert_eq!(
                fs::read(a.config.source.join(".chezmoiignore")).unwrap(),
                before
            );
            a.key(KeyEvent::new(
                if confirm {
                    KeyCode::Enter
                } else {
                    KeyCode::Esc
                },
                KeyModifiers::NONE,
            ));
            if confirm {
                let deadline = Instant::now() + Duration::from_secs(5);
                while !matches!(a.modal, Some(Modal::Outcome { .. })) && Instant::now() < deadline {
                    a.pump();
                    std::thread::sleep(Duration::from_millis(5));
                }
                assert!(
                    matches!(a.modal, Some(Modal::Outcome { ref outcome, .. }) if outcome.failed.is_none())
                );
            } else {
                assert!(a.modal.is_none());
            }
        }
        let saved = assignments::read(&a.config).unwrap();
        assert_eq!(saved[".untracked"].os, os);
        assert_eq!(
            saved[".untracked"].machines,
            BTreeSet::from(["laptop".into(), "desktop".into()])
        );
        assert_eq!(
            fs::read_to_string(a.config.home.join(".untracked")).unwrap(),
            "untracked"
        );
    }

    #[test]
    fn help_topics_render_resize_scroll_and_handle_pointer_controls() {
        let (_t, mut a) = app();
        a.key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
        assert!(matches!(a.modal, Some(Modal::Help(ref view)) if view.tab == 1));
        let mut artifacts = Vec::new();
        for (width, height) in [(120, 40), (80, 24), (60, 20), (40, 12), (20, 8)] {
            let mut term =
                Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            for topic in ['1', '2', '3'] {
                a.key(KeyEvent::new(KeyCode::Char(topic), KeyModifiers::NONE));
                term.draw(|f| a.draw(f)).unwrap();
                let text = term
                    .backend()
                    .buffer()
                    .content
                    .chunks(width as usize)
                    .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
                    .collect::<Vec<_>>()
                    .join("\n");
                assert!(text.contains("Help"));
                assert!(text.contains("Close"));
                if width >= 40 {
                    assert!(text.contains("General"));
                    assert!(text.contains("Files"));
                    assert!(text.contains("Git"));
                }
                artifacts.push(format!("{width}×{height} · topic {topic}\n{text}"));
                a.key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
                term.draw(|f| a.draw(f)).unwrap();
                let text = term
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect::<String>();
                let last_word = match topic {
                    '1' => "supported.",
                    '2' => "hooks.",
                    _ => "published.",
                };
                assert!(
                    text.contains(last_word),
                    "unreachable end of topic {topic} at {width}×{height}"
                );
                let before = a.areas.help.max_scroll;
                assert!(matches!(a.modal, Some(Modal::Help(_))));
                if before > 0 {
                    // Scrolling remains in Help and reaches the final content.
                    let body = a.areas.help.body;
                    a.mouse(event::MouseEvent {
                        kind: MouseEventKind::ScrollUp,
                        column: body.x,
                        row: body.y,
                        modifiers: KeyModifiers::NONE,
                    });
                    assert!(matches!(a.modal, Some(Modal::Help(_))));
                }
            }
        }
        // A resize while scrolled must clamp the viewport and remain navigable.
        let mut term = Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
        term.draw(|f| a.draw(f)).unwrap();
        let tab = a
            .areas
            .modal_controls
            .iter()
            .find(|(_, key)| *key == KeyCode::Char('2'))
            .unwrap()
            .0;
        a.mouse(event::MouseEvent {
            kind: MouseEventKind::Down(event::MouseButton::Left),
            column: tab.x,
            row: tab.y,
            modifiers: KeyModifiers::NONE,
        });
        assert!(matches!(a.modal, Some(Modal::Help(ref view)) if view.tab == 1));
        a.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert!(matches!(a.modal, Some(Modal::Help(ref view)) if view.tab == 2));
        a.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(a.modal.is_none());
        assert_eq!(a.pane, Pane::Files);
        a.pane = Pane::Git;
        a.key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
        assert!(matches!(a.modal, Some(Modal::Help(ref view)) if view.tab == 2));
        for (width, height) in [(8, 4), (1, 1)] {
            let mut tiny =
                Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            tiny.draw(|f| a.draw(f)).unwrap();
        }
        if let Ok(path) = std::env::var("DOT_HELP_RENDER") {
            fs::write(path, artifacts.join("\n\n")).unwrap();
        }
    }

    #[test]
    fn render_review_artifact() {
        let (_t, mut a) = app();
        let mut term = Terminal::new(ratatui::backend::TestBackend::new(120, 35)).unwrap();
        term.draw(|f| a.draw(f)).unwrap();
        if let Ok(path) = std::env::var("DOT_RENDER_JSON") {
            let b = term.backend().buffer();
            let rows:Vec<_>=b.content.chunks(120).map(|row|row.iter().map(|c|serde_json::json!({"text":c.symbol(),"fg":format!("{:?}",c.fg),"bg":format!("{:?}",c.bg)})).collect::<Vec<_>>()).collect();
            fs::write(path, serde_json::to_vec(&rows).unwrap()).unwrap();
        }
    }
    #[test]
    fn files_and_details_hide_ignored_children_after_background_navigation() {
        let (_t, mut a) = app();
        initialize(&a.config, &a.config_path, None).unwrap();
        fs::create_dir_all(a.config.home.join(".agents/skills")).unwrap();
        fs::write(a.config.home.join(".agents/.DS_Store"), "junk").unwrap();
        fs::write(a.config.home.join(".agents/skills/SKILL.md"), "skill").unwrap();
        fs::create_dir_all(a.config.home.join(".Trash")).unwrap();
        fs::write(a.config.home.join(".Trash/.DS_Store"), "junk").unwrap();
        a.snapshot = backend::snapshot(&a.config).unwrap();
        a.rebuild();
        assert!(!a.rows.iter().any(|r| r.path == ".Trash"));
        a.cursor = a.rows.iter().position(|r| r.path == ".agents").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            a.pump();
            if a.detail.contains("skills/") && a.job.is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(a.detail.contains("skills/"), "{}", a.detail);
        assert!(!a.detail.contains(".DS_Store"));
        a.enter_dir();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            a.pump();
            if a.snapshot.directory_entries.contains_key(".agents") && a.job.is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(a.rows.iter().any(|r| r.name == "skills"));
        assert!(!a.rows.iter().any(|r| r.name == ".DS_Store"));
    }
}
