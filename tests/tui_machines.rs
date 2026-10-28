//! The Machines panel, the assignment editor (`m`) and the assignment badge,
//! driven with real key events against the real bundled chezmoi and git.

mod common;

use std::fs;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use crossterm::event::{
    Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

use common::TestEnv;
use dotwarden::hide::HideRules;
use dotwarden::tui::app::{App, Dialog, Panel};
use dotwarden::tui::event::Event;
use dotwarden::tui::ui;

fn key(c: KeyCode) -> Event {
    Event::Input(TermEvent::Key(KeyEvent {
        code: c,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }))
}

struct Tui {
    t: TestEnv,
    app: App,
    rx: Receiver<Event>,
    _tx: Sender<Event>,
}

impl Tui {
    fn open(t: TestEnv) -> Self {
        let store = Arc::new(t.store());
        let rules = HideRules::for_store(&store).unwrap();
        let (tx, rx) = channel();
        let mut app = App::new(store, rules, tx.clone());
        app.start();
        let mut tui = Tui {
            t,
            app,
            rx,
            _tx: tx,
        };
        tui.settle();
        tui
    }

    fn settle(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match self.rx.recv_timeout(Duration::from_millis(300)) {
                Ok(ev) => self.app.handle_event(ev),
                Err(_) => {
                    if self.app.in_flight_jobs() == 0 && !self.app.actions_busy() {
                        return;
                    }
                }
            }
            assert!(Instant::now() < deadline, "the TUI never settled");
        }
    }

    fn press(&mut self, c: char) {
        self.app.handle_event(key(KeyCode::Char(c)));
        self.settle();
    }

    fn press_key(&mut self, k: KeyCode) {
        self.app.handle_event(key(k));
        self.settle();
    }

    fn select(&mut self, name: &str) {
        let i = self
            .app
            .files
            .visible()
            .iter()
            .position(|e| e.display() == name)
            .unwrap_or_else(|| panic!("{name} is not listed"));
        self.app.files.selected = i;
    }

    fn screen(&mut self) -> String {
        let (w, h) = (100, 30);
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| ui::draw(f, &mut self.app)).unwrap();
        let buf = term.backend().buffer().clone();
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Two tracked files: `.a` includes darwin, `.b` has no assignment. srv is a linux machine.
fn setup() -> TestEnv {
    let t = TestEnv::new();
    t.dw_ok(&["init", "--name", "alpha"]);
    t.dw_ok(&[
        "machine",
        "add",
        "srv",
        "--platform",
        "linux",
        "--distribution",
        "ubuntu",
        "--family",
        "debian",
        "--apply",
    ]);
    fs::write(t.home.join(".a"), "a\n").unwrap();
    fs::write(t.home.join(".b"), "b\n").unwrap();
    fs::write(t.home.join(".loose"), "l\n").unwrap();
    t.dw_ok(&["track", ".a", ".b", "--apply"]);
    t.dw_ok(&["assign", ".a", "--include", "platform:darwin", "--apply"]);
    t
}

fn pending_changes(t: &TestEnv) -> Vec<(String, Vec<String>, Vec<String>)> {
    let v = t.dw_json(&["pending", "list"]);
    let strings = |v: &serde_json::Value| -> Vec<String> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap().to_string())
            .collect()
    };
    v["items"][0]["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["path"].as_str().unwrap().to_string(),
                strings(&c["include"]),
                strings(&c["exclude"]),
            )
        })
        .collect()
}

#[test]
fn the_editor_is_tri_state_on_a_mixed_selection_and_leaves_mixed_targets_alone() {
    let mut tui = Tui::open(setup());
    tui.select(".a");
    tui.press('v');
    tui.select(".b");
    tui.press('m');
    assert!(matches!(tui.app.dialog, Some(Dialog::Assign(_))));
    let screen = tui.screen();
    assert!(
        screen.contains("Platforms") && screen.contains("Machines"),
        "{screen}"
    );
    assert!(
        screen.contains("[~] mixed") && screen.contains("darwin"),
        "darwin differs: {screen}"
    );

    // Enter without a change adds nothing.
    tui.press_key(KeyCode::Enter);
    assert!(matches!(tui.app.dialog, Some(Dialog::Assign(_))));
    assert!(pending_free(&tui.t));

    // Cursor 0 is darwin (mixed, left alone); linux gets Include for both files.
    tui.press('j');
    tui.press(' ');
    assert!(tui.screen().contains("[+] incl"));
    tui.press_key(KeyCode::Enter);
    assert!(tui.app.dialog.is_none());
    assert_eq!(
        pending_changes(&tui.t),
        [
            (
                ".a".to_string(),
                vec!["platform:darwin".to_string(), "platform:linux".to_string()],
                vec![]
            ),
            (".b".to_string(), vec!["platform:linux".to_string()], vec![]),
        ]
    );
}

fn pending_free(t: &TestEnv) -> bool {
    t.dw_json(&["pending", "list"])["items"]
        .as_array()
        .unwrap()
        .is_empty()
}

#[test]
fn exclude_from_the_editor_in_changes_and_untracked_paths_are_refused() {
    let t = setup();
    fs::write(t.home.join(".b"), "b changed\n").unwrap();
    let mut tui = Tui::open(t);
    tui.select(".loose");
    tui.press('m');
    assert!(tui.app.dialog.is_none());
    assert!(
        tui.app
            .message
            .as_deref()
            .unwrap_or("")
            .contains("not tracked")
    );

    tui.press('3');
    tui.press('m');
    let Some(Dialog::Assign(e)) = &tui.app.dialog else {
        panic!("the editor opens for the selected change");
    };
    assert_eq!(e.paths, [".b"]);
    // Move to the machine group's srv row and exclude it.
    let at = e
        .rows
        .iter()
        .position(|r| r.target == "machine:srv")
        .unwrap();
    for _ in 0..at {
        tui.press('j');
    }
    tui.press('x');
    tui.press_key(KeyCode::Enter);
    assert_eq!(
        pending_changes(&tui.t),
        [(".b".to_string(), vec![], vec!["machine:srv".to_string()])]
    );
}

#[test]
fn files_show_assignment_badges_and_the_machines_panel_previews_a_machine() {
    let t = setup();
    t.dw_ok(&["assign", ".b", "--exclude", "machine:srv", "--apply"]);
    let mut tui = Tui::open(t);
    let screen = tui.screen();
    let line = |needle: &str| {
        screen
            .lines()
            .find(|l| l.contains(needle))
            .unwrap_or("")
            .to_string()
    };
    assert!(line(".a").contains("[darwin]"), "{screen}");
    assert!(line(".b").contains("[!srv]"), "{screen}");
    assert!(!line(".loose").contains('['), "{screen}");

    tui.press('5');
    assert_eq!(tui.app.focus, Panel::Machines);
    let screen = tui.screen();
    assert!(
        screen.contains("* alpha darwin/macos") && screen.contains("srv linux/ubuntu"),
        "{screen}"
    );
    tui.press('j');
    tui.press_key(KeyCode::Enter);
    let screen = tui.screen();
    assert!(screen.contains("srv would receive 0 file(s)"), "{screen}");
}
