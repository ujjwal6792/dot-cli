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
fn git_branches_create_switch_and_delete_have_previews() {
    let mut tui = open(edited());
    tui.press('4');
    tui.press(']');
    tui.press('n');
    tui.type_text("other");
    tui.send(KeyCode::Enter, KeyModifiers::NONE);
    assert!(matches!(tui.app.dialog, Some(Dialog::GitConfirm { .. })));
    tui.press('y');
    assert_eq!(tui.app.branches.len(), 2);
    tui.press('j');
    tui.send(KeyCode::Enter, KeyModifiers::NONE);
    tui.press('y');
    assert!(
        tui.app
            .branches
            .iter()
            .any(|b| b.name == "other" && b.current)
    );
    assert!(tui.screen().contains("non-default"));
    tui.press('k');
    tui.send(KeyCode::Enter, KeyModifiers::NONE);
    tui.press('y');
    tui.press('j');
    tui.press('d');
    assert!(tui.screen().contains("Delete"));
    tui.press('y');
    assert_eq!(tui.app.branches.len(), 1);
    assert_eq!(fs::read_to_string(tui.t.home.join(".a")).unwrap(), "two\n");
}

#[test]
fn git_stash_confirmation_lists_home_and_round_trips() {
    let mut tui = open(edited());
    tui.press('4');
    tui.press('[');
    tui.press('s');
    let screen = tui.screen();
    assert!(screen.contains("HOME: .a"), "{screen}");
    tui.press('n');
    assert_eq!(fs::read_to_string(tui.t.home.join(".a")).unwrap(), "two\n");
    tui.press('s');
    fs::write(tui.t.home.join(".a"), "changed after preview\n").unwrap();
    tui.press('y');
    assert!(tui.app.stashes.is_empty());
    assert!(
        tui.app
            .message
            .as_deref()
            .unwrap_or("")
            .contains("changed since preview")
    );
    tui.send(KeyCode::Esc, KeyModifiers::NONE);
    fs::write(tui.t.home.join(".a"), "two\n").unwrap();
    tui.press('s');
    tui.press('y');
    assert_eq!(fs::read_to_string(tui.t.home.join(".a")).unwrap(), "one\n");
    assert_eq!(tui.app.stashes.len(), 1);
    tui.send(KeyCode::Enter, KeyModifiers::NONE);
    tui.press('y');
    assert_eq!(fs::read_to_string(tui.t.home.join(".a")).unwrap(), "two\n");
    assert!(tui.app.stashes.is_empty());
}

#[test]
fn h_shows_the_history_of_one_file_and_esc_shows_all_commits() {
    let t = edited();
    write(&t, ".b", "b\n");
    t.dw_ok(&["track", ".b", "--apply"]);
    t.dw_ok(&["commit", "-m", "only b"]);
    let mut tui = open(t);
    tui.press('H');
    assert_eq!(tui.app.focus, Panel::Git);
    let subjects: Vec<_> = tui.app.commits.iter().map(|c| c.subject.clone()).collect();
    assert_eq!(subjects, ["base"]);
    let screen = tui.screen();
    assert!(screen.contains("History of ~/.a"), "{screen}");
    assert!(
        screen.contains("one"),
        "the file's diff is shown:\n{screen}"
    );
    tui.send(KeyCode::Esc, KeyModifiers::NONE);
    assert!(tui.app.commits.iter().any(|c| c.subject == "only b"));
    assert!(tui.app.commit_path.is_none());
    assert!(!tui.screen().contains("History of"));
}

#[test]
fn remotes_tab_adds_an_unverifiable_host_on_confirmation_and_removes() {
    let mut tui = open(edited());
    tui.press('4');
    tui.press(']');
    tui.press(']');
    tui.press('n');
    let url = "ssh://git.example.test/dots.git";
    tui.type_text(&format!("origin {url}"));
    tui.send(KeyCode::Enter, KeyModifiers::NONE);
    let screen = tui.screen();
    assert!(screen.contains("cannot be checked"), "{screen}");
    tui.press('n');
    assert!(tui.app.remotes.is_empty(), "n adds nothing");
    tui.press('n');
    tui.type_text(&format!("origin {url}"));
    tui.send(KeyCode::Enter, KeyModifiers::NONE);
    tui.press('y');
    let list = tui.t.dw_json(&["remote", "list"]);
    assert_eq!(list["remotes"][0]["url"], url);
    assert_eq!(list["remotes"][0]["visibility"], "confirmed-private");
    assert_eq!(tui.app.remotes.len(), 1);
    tui.press('d');
    assert!(tui.screen().contains("Remove remote origin"));
    tui.press('y');
    assert!(tui.app.remotes.is_empty());
    assert!(
        tui.t.dw_json(&["remote", "list"])["remotes"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn git_history_restore_is_reviewed_as_incoming_before_apply() {
    let t = edited();
    t.dw_ok(&["stage", ".a"]);
    t.dw_ok(&["commit", "-m", "second"]);
    let head = t.git(&t.dirs().repo(), &["rev-parse", "HEAD"]);
    let mut tui = open(t);
    tui.press('4');
    tui.press('j');
    tui.press('R');
    assert!(tui.screen().contains("Review restore"));
    tui.press('y');
    assert_eq!(fs::read_to_string(tui.t.home.join(".a")).unwrap(), "two\n");
    tui.press('3');
    tui.press(']');
    tui.press(']');
    assert!(tui.app.change_rows().iter().any(|c| c.kind == "restore"));
    tui.send(KeyCode::Enter, KeyModifiers::NONE);
    tui.press('y');
    assert_eq!(fs::read_to_string(tui.t.home.join(".a")).unwrap(), "one\n");
    assert_eq!(
        tui.t.git(&tui.t.dirs().repo(), &["rev-parse", "HEAD"]),
        head
    );
}

#[test]
fn commits_cursor_diff_paging_and_revert_confirmation() {
    let t = edited();
    t.dw_ok(&["stage", ".a"]);
    t.dw_ok(&["commit", "-m", "second"]);
    // Real history beyond the initial page. Empty commits keep this inexpensive.
    for i in 0..51 {
        t.git(
            &t.dirs().repo(),
            &["commit", "--allow-empty", "-m", &format!("padding {i}")],
        );
    }
    let mut tui = open(t);
    tui.press('4');
    assert_eq!(tui.app.commits.len(), 50);
    for _ in 0..51 {
        tui.press('j');
    }
    assert!(tui.app.commits.len() > 50);
    assert_eq!(tui.app.commits[tui.app.commit_selected].subject, "second");
    let screen = tui.screen();
    assert!(screen.contains("two"), "{screen}");
    tui.press('r');
    assert!(matches!(tui.app.dialog, Some(Dialog::ConfirmRevert(_))));
    let before = tui.t.git(&tui.t.dirs().repo(), &["rev-parse", "HEAD"]);
    tui.press('n');
    assert_eq!(
        tui.t.git(&tui.t.dirs().repo(), &["rev-parse", "HEAD"]),
        before
    );
    tui.press('r');
    tui.press('y');
    assert_ne!(
        tui.t.git(&tui.t.dirs().repo(), &["rev-parse", "HEAD"]),
        before
    );
    assert_eq!(
        fs::read_to_string(tui.t.dirs().repo().join("home/dot_a")).unwrap(),
        "one\n"
    );
    assert_eq!(fs::read_to_string(tui.t.home.join(".a")).unwrap(), "two\n");
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
fn z_undoes_the_last_local_action() {
    let mut tui = open(edited());
    let before = tui.changes();
    tui.press(' ');
    assert_ne!(tui.changes(), before);
    tui.press('z');
    assert_eq!(tui.changes(), before);
    tui.press('d');
    tui.press('y');
    assert!(tui.changes().is_empty());
    tui.press('z');
    assert_eq!(fs::read_to_string(tui.t.home.join(".a")).unwrap(), "two\n");
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

const TWO_HUNKS_BASE: &str = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\nl11\nl12\n";

#[test]
fn enter_opens_the_staging_view_and_space_stages_one_line_a_stages_a_hunk() {
    let t = TestEnv::new();
    t.dw_ok(&["init", "--name", "testbox"]);
    write(&t, ".a", TWO_HUNKS_BASE);
    t.dw_ok(&["track", ".a", "--apply"]);
    t.dw_ok(&["commit", "-m", "base"]);
    let edit = "l1\nL2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\nL11\nl12\n";
    write(&t, ".a", edit);
    let mut tui = open(t);
    tui.send(KeyCode::Enter, KeyModifiers::NONE);
    let screen = tui.screen();
    assert!(
        screen.contains("-l2") && screen.contains("+L11"),
        "{screen}"
    );
    // Header, -l2: Space stages only the removal of l2.
    tui.press('j');
    tui.press('j');
    tui.press(' ');
    let repo_file = tui.t.dirs().repo().join("home/dot_a");
    let src = fs::read_to_string(&repo_file).unwrap();
    assert!(!src.contains("\nl2\n"), "{src}");
    assert!(src.contains("l11\n") && !src.contains("L2"), "{src}");
    assert!(
        tui.app.staging.is_some(),
        "the view stays open for the rest"
    );
    // Next hunk, then `a` stages all of it.
    tui.press(']');
    tui.press('a');
    let src = fs::read_to_string(&repo_file).unwrap();
    assert!(src.contains("L11") && !src.contains("\nl11\n"), "{src}");
    assert_eq!(fs::read_to_string(tui.t.home.join(".a")).unwrap(), edit);
    tui.send(KeyCode::Esc, KeyModifiers::NONE);
    assert!(tui.app.staging.is_none());
}

#[test]
fn staging_view_of_a_secret_file_is_read_only_and_masked() {
    let t = TestEnv::new();
    t.dw_ok(&["init", "--name", "testbox"]);
    write(&t, ".env", "host = h\n");
    t.dw_ok(&["track", ".env", "--apply"]);
    t.dw_ok(&["commit", "-m", "env"]);
    let secret = "Zq8x7Lm2Pq9wRt5vYb3nKd6HfJs1AaGc";
    write(&t, ".env", &format!("host = h\napi_token = {secret}\n"));
    let mut tui = open(t);
    tui.send(KeyCode::Enter, KeyModifiers::NONE);
    tui.press('j');
    tui.press('j');
    tui.press(' ');
    let screen = tui.screen();
    assert!(
        screen.contains("read-only") && !screen.contains(secret),
        "{screen}"
    );
    assert!(tui.changes().contains(&".env:local".to_string()));
}
