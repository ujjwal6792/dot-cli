//! TUI state and input handling. Nothing here performs I/O: anything that
//! touches the disk or runs a tool is a job whose result arrives as an event.

use std::sync::Arc;
use std::sync::mpsc::Sender;

use crossterm::event::{Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::event::Event;
use super::files::{self, FilesState};
use super::jobs::{Done, Jobs, KEY_LISTING, KEY_PREVIEW, KEY_STATUS, Payload};
use crate::hide::HideRules;
use crate::status::{self, Status};
use crate::store::Store;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Status,
    Files,
    Changes,
    Git,
    Machines,
}

impl Panel {
    pub const ALL: [Panel; 5] = [
        Panel::Status,
        Panel::Files,
        Panel::Changes,
        Panel::Git,
        Panel::Machines,
    ];

    pub fn number(self) -> usize {
        Self::ALL.iter().position(|p| *p == self).unwrap() + 1
    }

    pub fn title(self) -> &'static str {
        match self {
            Panel::Status => "Status",
            Panel::Files => "Files",
            Panel::Changes => "Changes",
            Panel::Git => "Git",
            Panel::Machines => "Machines",
        }
    }

    fn shifted(self, by: isize) -> Panel {
        let n = Self::ALL.len() as isize;
        let i = (self.number() as isize - 1 + by).rem_euclid(n);
        Self::ALL[i as usize]
    }
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
        }
    }

    pub fn in_flight_jobs(&self) -> usize {
        self.jobs.in_flight()
    }

    /// Kick off the initial loads. Returns immediately.
    pub fn start(&mut self) {
        self.refresh_status();
        self.reload_listing(None);
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

    pub fn handle_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
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
            KeyCode::Char('r') => {
                self.refresh_status();
                return self.reload_listing(None);
            }
            _ => {}
        }
        match self.focus {
            Panel::Files => self.files_key(key),
            _ => match key.code {
                KeyCode::Char('h') | KeyCode::Left => self.focus = self.focus.shifted(-1),
                KeyCode::Char('l') | KeyCode::Right => self.focus = self.focus.shifted(1),
                _ => {}
            },
        }
    }

    fn files_key(&mut self, key: KeyEvent) {
        let before = self.files.selected_path();
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.files.move_by(1),
            KeyCode::Char('k') | KeyCode::Up => self.files.move_by(-1),
            KeyCode::PageDown => self.files.move_by(10),
            KeyCode::PageUp => self.files.move_by(-10),
            KeyCode::Char('g') | KeyCode::Home => self.files.select_first(),
            KeyCode::Char('G') | KeyCode::End => self.files.select_last(),
            KeyCode::Char('.') => self.files.toggle_hidden(),
            KeyCode::Char('l') | KeyCode::Right | KeyCode::Enter => {
                if self.files.enter() {
                    return self.reload_listing(None);
                }
            }
            KeyCode::Char('h') | KeyCode::Left | KeyCode::Backspace => {
                let left = self.files.cwd.file_name().map(std::ffi::OsString::from);
                if self.files.leave() {
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
        assert_eq!(a.focus, Panel::Status);
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
