use crate::{
    backend::{self, Filter, Row, Snapshot},
    config::{Config, atomic_write},
    git, migration,
    operations::{self, Action, Outcome, Plan},
    sync,
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
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

const UNTRACKED_TEXT: Color = Color::Rgb(160, 168, 180);
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
    Setup,
    Confirm {
        command: Control,
        text: String,
    },
    Menu(usize),
    Input {
        purpose: Purpose,
        text: String,
    },
    Review {
        plan: Plan,
        scroll: u16,
    },
    Text {
        title: String,
        text: String,
        scroll: u16,
    },
}
enum Work {
    ConfigMessage(Config, String),
    Snapshot(Snapshot),
    Preview { key: String, text: String },
    Plan(Plan),
    Applied(Config, Outcome),
    Message(String),
}
struct Job {
    rx: Receiver<Result<Work>>,
    label: String,
    kind: JobKind,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum JobKind {
    Operation,
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
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        self.job = Some(Job {
            rx,
            label: label.into(),
            kind,
        });
    }
    fn refresh(&mut self) {
        let c = self.config.clone();
        self.launch_job(
            JobKind::Inspection,
            "Inspecting chezmoi and Git",
            move || Ok(Work::Snapshot(backend::snapshot(&c)?)),
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
        let received = self.job.as_ref().and_then(|j| match j.rx.try_recv() {
            Ok(r) => Some(r),
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err(anyhow::anyhow!("Worker stopped unexpectedly")))
            }
            Err(_) => None,
        });
        if let Some(result) = received {
            let kind = self.job.take().unwrap().kind;
            if kind == JobKind::Inspection {
                // Failed inspection must not turn dismissal into an immediate retry.
                // Keep the workspace available; R or setup explicitly retries.
                self.last_refresh = Instant::now();
                self.refresh_failed = result.is_err();
            }
            match result {
                Ok(Work::Snapshot(s)) => {
                    self.snapshot = s;
                    self.rebuild();
                    self.preview_key.clear();
                    self.last_refresh = Instant::now();
                    self.notice = "Ready · HOME, source and Git state are separate".into();
                }
                Ok(Work::Preview { key, text }) => {
                    if key == self.current_key() {
                        self.detail = text;
                        self.scroll = 0;
                    }
                }
                Ok(Work::Plan(p)) => self.modal = Some(Modal::Review { plan: p, scroll: 0 }),
                Ok(Work::Applied(c, r)) => {
                    self.config = c;
                    self.notice = format!(
                        "{} updated · {} failed · {} not attempted. {}",
                        r.succeeded.len(),
                        usize::from(r.failed.is_some()),
                        r.not_attempted.len(),
                        r.message
                    );
                    self.modal = Some(Modal::Text {
                        title: "Operation result".into(),
                        text: serde_json::to_string_pretty(&r).unwrap_or_default(),
                        scroll: 0,
                    });
                    self.selected.clear();
                    self.git_selected.clear();
                    self.refresh();
                }
                Ok(Work::ConfigMessage(c, t)) => {
                    self.config = c;
                    self.directory.clear();
                    self.notice = t.clone();
                    self.modal = Some(Modal::Text {
                        title: "Result".into(),
                        text: t,
                        scroll: 0,
                    });
                    self.refresh();
                }
                Ok(Work::Message(t)) => {
                    self.modal = Some(Modal::Text {
                        title: "Result".into(),
                        text: t,
                        scroll: 0,
                    });
                    self.refresh();
                }
                Err(e) => {
                    self.notice = e.to_string();
                    self.modal = Some(
                        if kind == JobKind::Inspection && !self.config.source.exists() {
                            Modal::Setup
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
        if self.job.is_none()
            && self.modal.is_none()
            && !self.refresh_failed
            && self.last_refresh.elapsed() > Duration::from_secs(15)
        {
            self.refresh();
        }
        if self.job.is_none() && self.modal.is_none() {
            let key = self.current_key();
            if key != self.preview_key {
                self.preview_key = key.clone();
                let c = self.config.clone();
                let s = self.snapshot.clone();
                let pane = self.context();
                let tab = self.detail_tab;
                let selected = self.rows.get(self.cursor).cloned();
                let gf = self.snapshot.git.files.get(self.git_cursor).cloned();
                let hist = self.history_view;
                self.launch_job(JobKind::Preview,"Loading selected details",move||{
                    let text=if pane==Pane::Machines{format!("LAST REPORTED · not live status\n\n{}\n\nLocal automatic scope: {} files\nTrust: {}\nReports are summary metadata; detailed activity stays local.",serde_json::to_string_pretty(&sync::cached_reports(&c))?,c.enabled.len(),if c.trusted{"trusted repository"}else{"strict"})}
                    else if pane==Pane::Status{format!("Machine: {}\nHOME: {}\nSource: {}\nObserved: {}\n\nGit: {}\nUpstream: {}\n↑{} outgoing · ↓{} incoming (last fetched)\n{}\n\nHOME differences: {}\nSource changes: {}\nAutomatic sync: {}\nReports: {}",c.machine,c.home.display(),c.source.display(),s.observed_at,s.git.branch,s.git.upstream,s.git.ahead,s.git.behind,s.git.error.clone().unwrap_or_default(),s.changes.len(),s.git.files.len(),c.sync_enabled,c.reporting)}
                    else if pane==Pane::Activity{serde_json::to_string_pretty(&operations::history(&c)?)?}
                    else if pane==Pane::Git{if hist{s.git.commits.join("\n")}else if let Some(f)=gf{format!("Source ↔ Git · {}\n\nUNSTAGED\n{}\nSTAGED\n{}",f.path,git::diff(&c,std::slice::from_ref(&f.path),false)?,git::diff(&c,std::slice::from_ref(&f.path),true)?)}else{"No source changes. Commit history: press h in Git.".into()}}
                    else if let Some(row)=selected{if row.directory{let rows=backend::rows(&c,&s,&row.path,Filter::All,"")?;format!("~/{} · {} managed descendants\n\n{}",row.path,row.children,rows.iter().map(|r|format!("{} {}{}",r.status,r.name,if r.directory{"/"}else{""})).collect::<Vec<_>>().join("\n"))}
                        else if let Some(m)=s.managed.get(&row.path){match tab{1=>format!("SOURCE {}\n\n{}",m.source_relative,fs::read_to_string(&m.source_absolute).unwrap_or_else(|_|"Binary or unavailable source".into())),2=>format!("HOME: {}\nSource: {}\nAutomatic updates here: {}\nNative chezmoi source semantics apply.\n\nUse Edit source for templates and shared/OS/machine settings.",m.absolute,m.source_relative,c.enabled.contains(&row.path)),3=>serde_json::to_string_pretty(&operations::history(&c)?.into_iter().filter(|e|e.paths.contains(&row.path)).collect::<Vec<_>>())?,_=>{let d=backend::preview(&c,std::slice::from_ref(&row.path),false)?;format!("SOURCE → HOME · {}\n\n{}",row.path,if d.is_empty(){"HOME matches rendered source. Check Git for unpublished changes.".into()}else{d})}}}
                        else{format!("UNTRACKED ~/{}\n\nSelect with Space, then Capture to add through chezmoi.\n\n{}",row.path,fs::read_to_string(c.home.join(&row.path)).unwrap_or_else(|_|"Binary or unavailable file".into()))}}
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
        if action.files() {
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
            Action::Commit | Action::Fetch | Action::Pull | Action::Push | Action::Setting
        ) {
            vec![]
        } else {
            self.paths(action)
        };
        self.modal = None;
        self.launch(action.label(), move || {
            Ok(Work::Plan(operations::plan(&c, action, &paths, payload)?))
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
        if self.pane == Pane::Git {
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
        } else if self.pane == Pane::Files {
            self.cursor = self
                .cursor
                .saturating_add_signed(delta)
                .min(self.rows.len().saturating_sub(1));
            self.file_state.select(Some(self.cursor));
        } else {
            let scroll = match self.pane {
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
        if self.modal.is_some() {
            if key.code == KeyCode::Char('q')
                && matches!(self.modal, Some(Modal::Text { .. } | Modal::Setup))
            {
                return self.request_quit();
            }
            self.modal_key(key);
            return false;
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
            KeyCode::Esc => {
                self.expanded = false;
                self.pane = Pane::Files;
            }
            KeyCode::Char('?') => {
                self.modal = Some(Modal::Text {
                    title: "Keyboard and pointer help".into(),
                    text: HELP.into(),
                    scroll: 0,
                })
            }
            KeyCode::Char(':') => self.modal = Some(Modal::Menu(0)),
            KeyCode::Char('R') => self.refresh(),
            KeyCode::Char('/') => {
                self.pane = Pane::Files;
                self.searching = true;
            }
            KeyCode::Char('f') if self.pane == Pane::Files => {
                self.cycle_tab(1);
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
            KeyCode::Char('a') if self.pane == Pane::Files => self.action(Action::Apply, None),
            KeyCode::Char('r') if self.pane == Pane::Files => self.action(Action::Restore, None),
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
        let modal = self.modal.take().unwrap();
        match modal {
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
            Modal::Confirm { command, text } => {
                if key.code == KeyCode::Char('y') {
                    self.confirm_control(command);
                } else if !matches!(key.code, KeyCode::Esc | KeyCode::Char('n')) {
                    self.modal = Some(Modal::Confirm { command, text });
                }
            }
            Modal::Input { purpose, mut text } => {
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
                                Ok(Work::Message(initialize(&c, &p, Some(&text))?))
                            });
                        }
                        Purpose::Commit => self.action(Action::Commit, Some(text)),
                        Purpose::Source(_) => self.action(Action::EditSource, Some(text)),
                        Purpose::Switch => {
                            self.modal = Some(Modal::Confirm {
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
            Modal::Review { plan, mut scroll } => match key.code {
                KeyCode::Esc | KeyCode::Char('n') => {}
                KeyCode::Char('y') => {
                    let mut c = self.config.clone();
                    let path = self.config_path.clone();
                    self.launch("Executing reviewed operation", move || {
                        let result = operations::execute(&mut c, &path, &plan, &plan.id)?;
                        Ok(Work::Applied(c, result))
                    });
                }
                _ => {
                    if matches!(key.code, KeyCode::Down | KeyCode::Char('j')) {
                        scroll = scroll.saturating_add(1);
                    }
                    if matches!(key.code, KeyCode::Up | KeyCode::Char('k')) {
                        scroll = scroll.saturating_sub(1);
                    }
                    self.modal = Some(Modal::Review { plan, scroll });
                }
            },
            Modal::Text {
                title,
                text,
                mut scroll,
            } => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q')) {
                    return;
                }
                if matches!(key.code, KeyCode::Down | KeyCode::Char('j')) {
                    scroll = scroll.saturating_add(1);
                }
                if matches!(key.code, KeyCode::Up | KeyCode::Char('k')) {
                    scroll = scroll.saturating_sub(1);
                }
                self.modal = Some(Modal::Text {
                    title,
                    text,
                    scroll,
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
            23=>{self.modal=Some(Modal::Confirm{command:Control::Service,text:format!("Install and start a local user service for {}? Sync must already be enabled. It runs while the TUI is closed, checking every {} seconds.",self.config.machine,self.config.interval_seconds)});},
            24=>self.modal=Some(Modal::Confirm{command:Control::Packages,text:"Install missing Git and chezmoi using Homebrew (macOS) or pacman (Linux)? The terminal will return to normal mode for package-manager authentication.".into()}),
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
                    Ok(Work::Message(initialize(&c, &p, None)?))
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
                    let msg = switch_source(&mut c, &p, &target)?;
                    Ok(Work::ConfigMessage(c, msg))
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
        if self.modal.is_some() {
            match mouse.kind {
                MouseEventKind::ScrollDown => {
                    self.modal_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
                }
                MouseEventKind::ScrollUp => {
                    self.modal_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE))
                }
                MouseEventKind::Down(event::MouseButton::Left) => {
                    if matches!(self.modal, Some(Modal::Setup)) {
                        if let Some((_, code)) =
                            self.areas.setup.iter().find(|(r, _)| r.contains(pos))
                        {
                            self.key(KeyEvent::new(*code, KeyModifiers::NONE));
                        }
                    } else if let Some(Modal::Menu(i)) = &mut self.modal {
                        let index = self.menu_state.offset()
                            + mouse.row.saturating_sub(self.areas.modal.y + 1) as usize;
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
                    } else if matches!(
                        self.modal,
                        Some(Modal::Review { .. } | Modal::Confirm { .. })
                    ) && mouse.row == self.areas.modal.bottom().saturating_sub(1)
                    {
                        self.modal_key(KeyEvent::new(
                            if mouse.column < self.areas.modal.x + 20 {
                                KeyCode::Char('y')
                            } else {
                                KeyCode::Esc
                            },
                            KeyModifiers::NONE,
                        ));
                    } else if matches!(self.modal, Some(Modal::Text { .. }))
                        && mouse.row == self.areas.modal.bottom().saturating_sub(1)
                    {
                        self.modal = None;
                    }
                }
                _ => {}
            }
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
            self.pane = pane;
            if pane != Pane::Details {
                self.detail_owner = pane;
            }
            match mouse.kind {
                MouseEventKind::ScrollDown => self.move_cursor(3),
                MouseEventKind::ScrollUp => self.move_cursor(-3),
                MouseEventKind::Down(event::MouseButton::Left) => {
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
        let outer = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);
        self.areas = Areas::default();
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
                ("c Capture", KeyCode::Char('c')),
                ("a Apply", KeyCode::Char('a')),
                ("r Restore", KeyCode::Char('r')),
                ("f Filter", KeyCode::Char('f')),
                (": Actions", KeyCode::Char(':')),
                ("? Help", KeyCode::Char('?')),
            ]
        } else if self.pane == Pane::Git && !self.history_view {
            vec![
                ("Space Stage", KeyCode::Char(' ')),
                ("c Commit", KeyCode::Char('c')),
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
        bindings.splice(
            0..0,
            [
                ("[ Prev", KeyCode::Char('[')),
                ("] Next", KeyCode::Char(']')),
            ],
        );
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
        if let Some(modal) = self.modal.clone() {
            self.areas.modal = outer[0];
            f.render_widget(Clear, outer[0]);
            match modal {
                Modal::Setup => {
                    let block = Block::bordered()
                        .title("Connect your dotfiles")
                        .title_bottom("[Enter / Esc Back · q / Ctrl+C Quit]");
                    let inner = block.inner(outer[0]);
                    f.render_widget(block, outer[0]);
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
                        "Source does not exist: {}\nChoose a setup action below. HOME configs are not applied.\nInitialization also installs the dot agent skill.",
                        self.config.source.display()
                    )).wrap(Wrap { trim: false }), areas[0]);
                    self.areas.setup.clear();
                    for (index, (label, code)) in [
                        ("[i] Initialize a new local source", KeyCode::Char('i')),
                        ("[c] Clone a repository", KeyCode::Char('c')),
                        ("[o] Connect an existing checkout", KeyCode::Char('o')),
                        ("[Esc] Browse HOME without setup", KeyCode::Esc),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        f.render_widget(
                            Paragraph::new(label).style(Style::default().fg(Color::LightCyan)),
                            areas[index + 1],
                        );
                        self.areas.setup.push((areas[index + 1], code));
                    }
                }
                Modal::Confirm { text, .. } => {
                    f.render_widget(
                        Paragraph::new(text).wrap(Wrap { trim: false }).block(
                            Block::bordered()
                                .title("Review local action · y confirm · Esc cancel")
                                .title_bottom("[y Execute]         [Esc Cancel]"),
                        ),
                        outer[0],
                    );
                }
                Modal::Menu(i) => {
                    self.menu_state.select(Some(i));
                    let list = List::new(MENU.iter().map(|s| ListItem::new(*s)))
                        .block(Block::bordered().title("Actions · Enter activate · Esc cancel"))
                        .highlight_style(selection_style())
                        .highlight_symbol("› ");
                    f.render_stateful_widget(list, outer[0], &mut self.menu_state);
                }
                Modal::Review { plan, scroll } => {
                    let text = format!(
                        "{}\n\nExact targets: {}\n{}\n\n{}\n\nApproval: {}\n[y] Execute this reviewed operation · [Esc] Cancel",
                        plan.action.label(),
                        plan.paths.len(),
                        plan.paths.join("\n"),
                        plan.preview,
                        plan.id
                    );
                    f.render_widget(
                        Paragraph::new(diff_text(&text))
                            .block(
                                Block::bordered()
                                    .title("Review · y executes · Esc cancels")
                                    .title_bottom("[y Execute]         [Esc Cancel]"),
                            )
                            .scroll((scroll, 0)),
                        outer[0],
                    );
                }
                Modal::Text {
                    title,
                    text,
                    scroll,
                } => {
                    f.render_widget(
                        Paragraph::new(text)
                            .block(
                                Block::bordered()
                                    .title(format!("{title} · Enter/Esc back"))
                                    .title_bottom("[Enter / Esc Back · q / Ctrl+C Quit]"),
                            )
                            .wrap(Wrap { trim: false })
                            .scroll((scroll, 0)),
                        outer[0],
                    );
                }
                Modal::Input { purpose, text } => {
                    let title = match purpose {
                        Purpose::Setting(_) => "Setting key=value · Enter review · Esc cancel",
                        Purpose::Repo => {
                            "Repository URL · Enter clone to configured source · Esc cancel"
                        }
                        Purpose::Commit => "Commit message · Enter review",
                        Purpose::Source(ref p) => {
                            let _ = p;
                            "Edit source · Ctrl+S review · arrows move · Esc cancel"
                        }
                        Purpose::Switch => {
                            "New source path · Enter archives and switches · Esc cancel"
                        }
                        Purpose::Machine => "Machine label · Enter save",
                        Purpose::Recover => {
                            "Recovery id · Enter restores if unchanged · Esc cancel"
                        }
                    };
                    let mut display = text;
                    let byte = display
                        .char_indices()
                        .nth(self.input_cursor)
                        .map(|(i, _)| i)
                        .unwrap_or(display.len());
                    let before = &display[..byte];
                    let width = outer[0].width.saturating_sub(2).max(1) as usize;
                    let cursor_row: usize = before
                        .split('\n')
                        .map(|line| 1 + line.chars().count() / width)
                        .sum::<usize>()
                        .saturating_sub(1);
                    let offset = cursor_row
                        .saturating_sub(outer[0].height.saturating_sub(4) as usize)
                        .min(u16::MAX as usize) as u16;
                    display.insert(byte, '▏');
                    f.render_widget(
                        Paragraph::new(display)
                            .scroll((offset, 0))
                            .block(Block::bordered().title(title))
                            .wrap(Wrap { trim: false }),
                        outer[0],
                    );
                }
            }
        }
    }
    fn block(&self, title: String, pane: Pane) -> Block<'static> {
        let block = Block::bordered().border_style(Style::default().fg(if self.pane == pane {
            Color::Rgb(108, 118, 137)
        } else {
            Color::Rgb(73, 80, 94)
        }));
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
                .fg(Color::Rgb(196, 167, 231))
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
                        Color::LightCyan
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
                .style(Style::default().fg(if row.changed {
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
        f.render_widget(
            Paragraph::new(diff_text(&self.detail))
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
fn diff_text(text: &str) -> Text<'static> {
    Text::from(
        text.lines()
            .map(|l| {
                Line::styled(
                    l.to_string(),
                    Style::default().fg(if l.starts_with('+') {
                        Color::Green
                    } else if l.starts_with('-') {
                        Color::Red
                    } else if l.starts_with("@@") {
                        Color::Cyan
                    } else {
                        Color::White
                    }),
                )
            })
            .collect::<Vec<_>>(),
    )
}
const MENU: &[&str] = &[
    "Capture selected HOME changes → source",
    "Apply selected source → HOME",
    "Restore selected HOME from source",
    "Stop tracking selected (shared source)",
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
];
const HELP: &str = "1 Status · 2 Files · 3 Git · 4 Machines · 0 Details\nTab focus next pane · z expand focused pane · Esc return\n↑/↓ or j/k move · PgUp/PgDn scroll\nFiles: →/l enter folder · ←/h parent · Space select · / filter · f All/Managed/Changed\nFiles: c capture HOME → source · a apply · r restore · e edit source\nGit: Space stage/unstage · v multi-select · c commit · d discard source → HEAD\nGit: h changes/history · F fetch · L pull · P push\n[/] previous/next tab in focused section · click tab labels · Enter focus details\n: all actions, Settings, Machines, Activity, Recovery, Packages\nR refresh · ? help · q quit when idle\nMouse: click panes/rows/footer hints · wheel scroll pane\nText fields receive all character keys before global commands. Paste supported.\nNative chezmoi previews may evaluate repository templates/hooks.\nHOME changes, source changes, staged changes and unpublished commits are distinct.";

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
                    if app.searching {
                        app.query.push_str(&s);
                        app.rebuild();
                    } else if let Some(Modal::Input { text, .. }) = &mut app.modal {
                        let mut chars: Vec<_> = text.chars().collect();
                        let i = app.input_cursor.min(chars.len());
                        let add: Vec<_> = s.chars().collect();
                        app.input_cursor = i + add.len();
                        chars.splice(i..i, add);
                        *text = chars.into_iter().collect();
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

pub fn initialize(c: &Config, path: &Path, repo: Option<&str>) -> Result<String> {
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
    c.save(path)?;
    let skill = install_skill(c)?;
    Ok(format!(
        "Connected {}. HOME has not been applied.\n{skill}",
        c.source.display()
    ))
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
        a.key(KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE));
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
        for c in "q2Wconfig".chars() {
            assert!(!a.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)));
        }
        assert_eq!(a.query, "q2Wconfig");
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
            .find(|(_, k)| *k == KeyCode::Char('f'))
            .unwrap()
            .0;
        a.mouse(event::MouseEvent {
            kind: MouseEventKind::Down(event::MouseButton::Left),
            column: filter.x,
            row: filter.y,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(a.filter, Filter::Managed);
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
}
