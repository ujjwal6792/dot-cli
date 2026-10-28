//! The Changes panel: tabs, staging, commit box, discard prompt and the styled
//! diff, driven with real key events against the real bundled chezmoi and git.

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
use dotwarden::tui::app::{App, Dialog, Load, Panel};
use dotwarden::tui::event::Event;
use dotwarden::tui::ui;

fn key_with(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Input(TermEvent::Key(KeyEvent {
        code,
        modifiers,
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

fn write(t: &TestEnv, rel: &str, content: &str) {
    let p = t.home.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

/// A machine with `.a` committed, then `.a` edited.
fn edited() -> TestEnv {
    let t = TestEnv::new();
    t.dw_ok(&["init", "--name", "testbox"]);
    write(&t, ".a", "one\n");
    t.dw_ok(&["track", ".a", "--apply"]);
    t.dw_ok(&["commit", "-m", "base"]);
    write(&t, ".a", "two\n");
    t
}

fn open(t: TestEnv) -> Tui {
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
    tui.press('3');
    tui
}

impl Tui {
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

    fn send(&mut self, code: KeyCode, m: KeyModifiers) {
        self.app.handle_event(key_with(code, m));
        self.settle();
    }

    fn press(&mut self, c: char) {
        self.send(KeyCode::Char(c), KeyModifiers::NONE);
    }

    fn type_text(&mut self, s: &str) {
        s.chars().for_each(|c| self.press(c));
    }

    fn screen(&mut self) -> String {
        let (w, h) = (110, 30);
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| ui::draw(f, &mut self.app)).unwrap();
        let buf = term.backend().buffer().clone();
        (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn changes(&self) -> Vec<String> {
        let v = self.t.dw_json(&["status"]);
        v["changes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| format!("{}:{}", c["path"], c["state"]).replace('"', ""))
            .collect()
    }
}

#[test]
fn space_stages_on_the_local_tab_and_unstages_on_the_staged_tab() {
    let mut tui = open(edited());
    assert_eq!(tui.app.focus, Panel::Changes);
    let screen = tui.screen();
    assert!(
        screen.contains("Local 1") && screen.contains("~/.a"),
        "{screen}"
    );
    assert!(
        screen.contains("local 1 · staged 0"),
        "status counts:\n{screen}"
    );

    tui.press(' ');
    assert_eq!(tui.changes(), [".a:staged"]);
    assert!(tui.screen().contains("local 0 · staged 1"));

    tui.press(']');
    assert!(tui.screen().contains("~/.a"));
    tui.press(' ');
    assert_eq!(tui.changes(), [".a:local"]);

    tui.press('[');
    tui.press('a');
    assert_eq!(tui.changes(), [".a:staged"], "a stages everything on Local");
    tui.press(']');
    tui.press('a');
    assert_eq!(
        tui.changes(),
        [".a:local"],
        "a unstages everything on Staged"
    );
}

#[test]
fn c_opens_a_box_that_commits_a_multi_line_message_and_a_amends_it() {
    let mut tui = open(edited());
    tui.press('c');
    assert!(tui.app.message.as_deref() == Some("nothing is staged"));
    tui.press('a');
    tui.press('c');
    assert!(matches!(tui.app.dialog, Some(Dialog::Commit(_))));
    tui.send(KeyCode::Enter, KeyModifiers::NONE);
    assert!(
        matches!(tui.app.dialog, Some(Dialog::Commit(_))),
        "an empty message does not commit"
    );
    tui.type_text("Subject");
    tui.send(KeyCode::Char('j'), KeyModifiers::CONTROL);
    tui.send(KeyCode::Enter, KeyModifiers::ALT);
    tui.type_text("Body line");
    tui.send(KeyCode::Enter, KeyModifiers::NONE);
    assert!(tui.app.dialog.is_none());
    let repo = tui.t.dirs().repo();
    assert_eq!(
        tui.t.git(&repo, &["log", "-1", "--format=%B"]),
        "Subject\n\nBody line"
    );
    assert!(tui.changes().is_empty());

    tui.press('A');
    match &tui.app.dialog {
        Some(Dialog::Commit(b)) => assert!(b.amend && b.text == "Subject\n\nBody line"),
        other => panic!("expected the amend box, got {other:?}"),
    }
    tui.send(KeyCode::Esc, KeyModifiers::NONE);
    assert!(tui.app.dialog.is_none());
}

#[test]
fn d_asks_before_discarding_and_a_snapshot_keeps_the_edit() {
    let mut tui = open(edited());
    tui.press('d');
    assert!(matches!(tui.app.dialog, Some(Dialog::ConfirmDiscard(_))));
    assert!(tui.screen().contains("Overwrite ~/.a"));
    tui.press('n');
    assert_eq!(fs::read_to_string(tui.t.home.join(".a")).unwrap(), "two\n");

    tui.press('d');
    tui.press('y');
    assert_eq!(fs::read_to_string(tui.t.home.join(".a")).unwrap(), "one\n");
    let snaps = tui.t.dw_json(&["snapshot", "list"]);
    assert_eq!(snaps[0]["reason"], "discard");
    assert!(tui.changes().is_empty());
}

#[test]
fn the_main_view_shows_the_selected_diff_styled_by_delta() {
    let mut tui = open(edited());
    let Load::Ready(text) = &tui.app.diff else {
        panic!("diff not ready: {:?}", tui.app.diff);
    };
    let plain: String = text
        .lines
        .iter()
        .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
        .collect();
    assert!(plain.contains("one") && plain.contains("two"), "{plain}");
    assert!(!plain.contains('\u{1b}'), "escape codes must be converted");
    assert!(
        text.lines
            .iter()
            .flat_map(|l| &l.spans)
            .any(|s| s.style.fg.is_some() || s.style.bg.is_some()),
        "delta's colours become styled spans"
    );
    // Moving to another tab replaces it.
    tui.press(']');
    tui.press('[');
    assert!(tui.screen().contains("two"));
}

#[test]
fn incoming_files_are_applied_after_a_y_n_prompt_with_a_snapshot() {
    let t = edited();
    t.dw_ok(&["discard", ".a", "--yes"]);
    let repo = t.dirs().repo();
    fs::write(repo.join("home/dot_a"), "remote\n").unwrap();
    t.git(&repo, &["commit", "-qam", "from elsewhere"]);
    let mut tui = open(t);
    tui.press(']');
    tui.press(']');
    let screen = tui.screen();
    assert!(
        screen.contains("Incoming 1") && screen.contains("~/.a"),
        "{screen}"
    );
    tui.press(' ');
    assert!(matches!(tui.app.dialog, Some(Dialog::ConfirmApply(_))));
    tui.press('n');
    assert_eq!(tui.changes(), [".a:incoming"]);
    assert_eq!(fs::read_to_string(tui.t.home.join(".a")).unwrap(), "one\n");

    tui.press('a');
    tui.press('y');
    assert_eq!(
        fs::read_to_string(tui.t.home.join(".a")).unwrap(),
        "remote\n"
    );
    assert!(tui.changes().is_empty());
    assert_eq!(
        tui.t.dw_json(&["snapshot", "list", ".a"])[0]["reason"],
        "apply"
    );
}

#[test]
fn p_f_and_push_work_from_the_keyboard_and_push_asks_first() {
    let t = edited();
    t.dw_ok(&["discard", ".a", "--yes"]);
    let remote = t.root.path().join("r.git");
    t.git(
        &t.home,
        &[
            "init",
            "--bare",
            "--quiet",
            "--initial-branch=main",
            remote.to_str().unwrap(),
        ],
    );
    t.dw_ok(&["remote", "add", "origin", remote.to_str().unwrap(), "--yes"]);
    let mut tui = open(t);
    tui.press('P');
    assert!(matches!(tui.app.dialog, Some(Dialog::ConfirmPush)));
    tui.press('n');
    assert!(tui.t.git(&remote, &["branch", "--list"]).is_empty());
    tui.press('P');
    tui.press('y');
    let head = tui.t.git(&tui.t.dirs().repo(), &["rev-parse", "HEAD"]);
    assert_eq!(tui.t.git(&remote, &["rev-parse", "main"]), head);
    tui.press('f');
    assert!(
        tui.app
            .message
            .as_deref()
            .unwrap()
            .contains("ahead 0, behind 0")
    );
    tui.press('p');
    assert!(tui.app.message.as_deref().unwrap().contains("Up to date"));
}
