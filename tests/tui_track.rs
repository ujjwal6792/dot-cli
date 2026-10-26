//! Tracking from the TUI: Space, range select, markers and the Pending pane,
//! driven with real key events against the real bundled chezmoi and git.

mod common;

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::process::Command;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use crossterm::event::{
    Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use serde_json::Value;

use common::{TestEnv, libexec};
use dotwarden::hide::HideRules;
use dotwarden::store::Store;
use dotwarden::toolchain::Toolchain;
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

fn ch(c: char) -> Event {
    key(KeyCode::Char(c))
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

fn cli(t: &TestEnv, args: &[&str]) -> (i32, String) {
    cli_env(t, args, &[])
}

fn cli_env(t: &TestEnv, args: &[&str], extra: &[(&str, &str)]) -> (i32, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_dotwarden"))
        .args(args)
        .env_clear()
        .envs(&t.env)
        .envs(extra.iter().copied())
        .current_dir(&t.home)
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    (
        o.status.code().unwrap(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        ),
    )
}

fn pending(t: &TestEnv) -> Vec<Value> {
    let (code, out) = cli(t, &["pending", "list", "--json"]);
    assert_eq!(code, 0, "{out}");
    serde_json::from_str::<Value>(&out).unwrap()["items"]
        .as_array()
        .unwrap()
        .clone()
}

fn summaries(t: &TestEnv) -> Vec<String> {
    pending(t)
        .iter()
        .map(|i| i["summary"].as_str().unwrap().to_string())
        .collect()
}

fn git_status(t: &TestEnv) -> String {
    t.git(
        &t.dirs().repo(),
        &["status", "--porcelain", "--untracked-files=all"],
    )
}

fn open_with(t: TestEnv, store: Store) -> Tui {
    let store = Arc::new(store);
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

fn open(t: TestEnv) -> Tui {
    let store = t.store();
    open_with(t, store)
}

fn initialised() -> TestEnv {
    let t = TestEnv::new();
    t.dw_ok(&["init", "--name", "testbox"]);
    t
}

impl Tui {
    /// Handle events until no job or action is running or queued.
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
        self.app.handle_event(ch(c));
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

    fn enter(&mut self, name: &str) {
        self.select(name);
        self.press('l');
    }

    fn screen(&mut self) -> String {
        self.screen_at(100, 30)
    }

    fn screen_at(&mut self, w: u16, h: u16) -> String {
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

// ---- Space --------------------------------------------------------------------

#[test]
fn space_adds_a_pending_item_with_a_marker_and_a_second_space_drops_it() {
    let t = initialised();
    write(&t, ".zshrc", "export A=1\n");
    write(&t, ".aliases", "alias l=ls\n");
    let mut tui = open(t);
    tui.select(".zshrc");
    tui.press(' ');

    let items = pending(&tui.t);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["kind"], "track");
    assert_eq!(items[0]["path"], ".zshrc");
    assert_eq!(items[0]["requester"], "human");
    assert_eq!(
        git_status(&tui.t),
        "",
        "Space only adds a pending item, no dialog and no repository change"
    );
    let screen = tui.screen();
    assert!(screen.contains("+ .zshrc"), "pending marker:\n{screen}");
    assert!(screen.contains("[x] #1 human: Track ~/.zshrc"), "{screen}");
    assert!(screen.contains("Added #1"), "feedback:\n{screen}");
    assert!(
        screen.contains("[untracked] pending: track"),
        "main view says so:\n{screen}"
    );
    assert!(
        screen.contains("pending: 1"),
        "status panel count:\n{screen}"
    );

    tui.press(' ');
    assert!(pending(&tui.t).is_empty(), "second Space drops the item");
    let screen = tui.screen();
    assert!(!screen.contains("+ .zshrc"), "{screen}");
    assert!(screen.contains("Dropped #1"), "{screen}");
}

#[test]
fn space_on_a_tracked_file_adds_an_untrack_item() {
    let t = initialised();
    write(&t, ".zshrc", "x");
    t.dw_ok(&["track", ".zshrc", "--apply"]);
    let mut tui = open(t);
    tui.select(".zshrc");
    assert!(tui.screen().contains("✓  .zshrc"));
    tui.press(' ');
    assert_eq!(summaries(&tui.t), ["Untrack ~/.zshrc"]);
    let screen = tui.screen();
    assert!(
        screen.contains("✓- .zshrc"),
        "tracked + pending untrack:\n{screen}"
    );
    tui.press(' ');
    assert!(pending(&tui.t).is_empty());
}

#[test]
fn space_on_a_folder_tracks_the_folder_and_the_markers_show_every_state() {
    let t = initialised();
    write(&t, ".config/app/a.conf", "a");
    write(&t, "proj/one", "1");
    write(&t, "proj/two", "2");
    write(&t, ".zshrc", "z");
    write(&t, ".untouched", "u");
    symlink(".zshrc", t.home.join("link")).unwrap();
    t.dw_ok(&[
        "track",
        ".config/app",
        "proj/one",
        ".zshrc",
        "link",
        "--apply",
    ]);
    write(&t, ".config/app/new.txt", "n");
    let mut tui = open(t);

    let screen = tui.screen();
    for (needle, what) in [
        ("✓  .zshrc", "tracked file"),
        ("✓  link@", "tracked symlink"),
        ("◇  .config/", "folder holding tracked files"),
        ("◇  proj/", "folder holding tracked files"),
        ("   .untouched", "untracked"),
    ] {
        assert!(screen.contains(needle), "{what}: {needle}\n{screen}");
    }
    tui.enter(".config");
    assert!(tui.screen().contains("▣  app/"), "{}", tui.screen());
    tui.enter("app");
    let screen = tui.screen();
    assert!(screen.contains("✓  a.conf"), "{screen}");
    assert!(
        screen.contains("?  new.txt"),
        "new file in a tracked folder:\n{screen}"
    );
    // Space on the New file tracks it; the pending marker appears.
    tui.select("new.txt");
    tui.press(' ');
    assert_eq!(summaries(&tui.t), ["Track ~/.config/app/new.txt"]);
    assert!(tui.screen().contains("?+ new.txt"), "{}", tui.screen());
    tui.press(' ');

    // A folder holding tracked files offers to untrack them (review fix 6) ...
    tui.press('h');
    tui.press('h');
    tui.select("proj");
    tui.press(' ');
    assert_eq!(summaries(&tui.t), ["Untrack ~/proj (1 file)"]);
    tui.press(' ');
    assert!(pending(&tui.t).is_empty());
}

#[test]
fn space_on_a_folder_without_tracked_files_tracks_it() {
    let t = initialised();
    write(&t, "fresh/f", "f");
    let mut tui = open(t);
    tui.select("fresh");
    tui.press(' ');
    assert_eq!(summaries(&tui.t), ["Track folder ~/fresh (1 file)"]);
}

#[test]
fn hidden_entries_are_refused_with_the_reason_in_the_footer() {
    let t = initialised();
    write(&t, ".cache/x", "c");
    let mut tui = open(t);
    tui.press('.');
    tui.select(".cache");
    tui.press(' ');
    assert!(pending(&tui.t).is_empty());
    let screen = tui.screen();
    assert!(
        screen.contains("hidden") && screen.contains(".cache/"),
        "{screen}"
    );
}

// ---- range select ---------------------------------------------------------------------

#[test]
fn v_selects_a_range_and_space_flips_every_entry_in_it() {
    let t = initialised();
    for f in ["d/a", "d/b", "d/c", "d/e"] {
        write(&t, f, f);
    }
    t.dw_ok(&["track", "d/b", "--apply"]);
    let mut tui = open(t);
    tui.enter("d");
    tui.select("a");
    tui.press('v');
    tui.press('j');
    tui.press('j');
    assert!(tui.app.range_rows().is_some());
    tui.press(' ');
    assert!(tui.app.range_rows().is_none(), "the range ends after Space");
    // a and c get tracked, the already tracked b gets untracked, e is untouched.
    let mut s = summaries(&tui.t);
    s.sort();
    assert_eq!(s, ["Track ~/d/a", "Track ~/d/c", "Untrack ~/d/b"]);
    assert!(
        tui.app
            .message
            .as_deref()
            .unwrap_or("")
            .contains("3 pending changes updated"),
        "{:?}",
        tui.app.message
    );
}

#[test]
fn escape_cancels_a_range_and_moving_backwards_selects_upwards() {
    let t = initialised();
    for f in ["d/a", "d/b", "d/c"] {
        write(&t, f, f);
    }
    let mut tui = open(t);
    tui.enter("d");
    tui.select("b");
    tui.press('v');
    tui.press_key(KeyCode::Esc);
    assert!(tui.app.range_rows().is_none());
    tui.press(' ');
    assert_eq!(summaries(&tui.t), ["Track ~/d/b"], "only the cursor entry");
    tui.press(' ');

    tui.select("c");
    tui.press('v');
    tui.press('k');
    tui.press('k');
    assert_eq!(tui.app.range_rows(), Some((0, 2)));
    assert!(tui.screen().contains("c"), "range renders");
    tui.press(' ');
    assert_eq!(pending(&tui.t).len(), 3);
}

// ---- the Pending pane ----------------------------------------------------------------------

#[test]
fn the_pending_pane_shows_requesters_previews_and_drops_items() {
    let t = initialised();
    write(&t, ".zshrc", "z");
    write(&t, ".b", "b");
    cli_env(&t, &["track", ".zshrc"], &[("DOTWARDEN_AGENT", "claude")]);
    t.dw_ok(&["track", ".b"]);
    let mut tui = open(t);
    tui.press('0');
    assert_eq!(tui.app.focus, Panel::Pending);
    let screen = tui.screen();
    assert!(screen.contains("[0] Pending (2)"), "{screen}");
    assert!(screen.contains("[x] #1 claude: Track ~/.zshrc"), "{screen}");
    assert!(screen.contains("[x] #2 human: Track ~/.b"), "{screen}");
    // The first item's exact preview is in the main view.
    for needle in [
        "requested by claude",
        "applies to: all machines",
        "+ home/dot_zshrc",
    ] {
        assert!(screen.contains(needle), "{needle}:\n{screen}");
    }
    tui.press('j');
    let screen = tui.screen();
    assert!(
        screen.contains("requested by human") && screen.contains("+ home/dot_b"),
        "{screen}"
    );

    tui.press('d');
    assert_eq!(summaries(&tui.t), ["Track ~/.zshrc"]);
    assert_eq!(tui.app.pending.items.len(), 1);
    assert!(tui.screen().contains("Dropped #2"));
    // Tab reaches the pane too: Machines -> Pending.
    tui.press('5');
    tui.press_key(KeyCode::Tab);
    assert_eq!(tui.app.focus, Panel::Pending);
}

#[test]
fn enter_opens_one_review_of_the_selected_items_and_confirming_applies_them() {
    let t = initialised();
    for f in [".a", ".b", ".c"] {
        write(&t, f, f);
    }
    t.dw_ok(&["track", ".a", ".b", ".c"]);
    let mut tui = open(t);
    tui.press('0');
    tui.press('j');
    tui.press(' '); // uncheck #2
    assert!(tui.screen().contains("[ ] #2"), "{}", tui.screen());

    tui.press_key(KeyCode::Enter);
    assert!(matches!(tui.app.dialog, Some(Dialog::Review(_))));
    let screen = tui.screen();
    assert!(screen.contains("Apply 2 pending item(s)?"), "{screen}");
    assert!(
        screen.contains("Track ~/.a") && screen.contains("Track ~/.c"),
        "{screen}"
    );
    assert!(screen.contains("home/dot_a"), "files are listed:\n{screen}");
    assert!(
        !screen.contains("Track ~/.b  ") || !screen.contains("— Track ~/.b"),
        "{screen}"
    );

    // Escape cancels: nothing happened.
    tui.press_key(KeyCode::Esc);
    assert!(tui.app.dialog.is_none());
    assert_eq!(git_status(&tui.t), "");
    assert_eq!(pending(&tui.t).len(), 3);

    // Enter, then confirm.
    tui.press_key(KeyCode::Enter);
    tui.press_key(KeyCode::Enter);
    let screen = tui.screen();
    assert!(
        matches!(tui.app.dialog, Some(Dialog::Result(_))),
        "{screen}"
    );
    assert!(
        screen.contains("applied") && screen.contains("Track ~/.a"),
        "{screen}"
    );
    assert_eq!(
        git_status(&tui.t).lines().collect::<BTreeSet<_>>(),
        ["A  home/dot_a", "A  home/dot_c"].into()
    );
    assert_eq!(
        summaries(&tui.t),
        ["Track ~/.b"],
        "the unchecked item stays"
    );
    tui.press('x'); // any key closes the result
    assert!(tui.app.dialog.is_none());
    let screen = tui.screen();
    assert!(
        screen.contains("✓  .a") || tui.app.tracked.is_tracked(".a"),
        "{screen}"
    );
    assert!(screen.contains("pending: 1"), "{screen}");
}

#[test]
fn a_stale_item_shows_in_the_partial_result_and_stays_pending() {
    let t = initialised();
    write(&t, ".fresh", "f");
    write(&t, ".moves", "before");
    t.dw_ok(&["track", ".fresh", ".moves"]);
    write(&t, ".moves", "different length after the item was added");
    let mut tui = open(t);
    tui.press('0');
    tui.press_key(KeyCode::Enter);
    tui.press_key(KeyCode::Enter);
    let screen = tui.screen();
    assert!(
        screen.contains("stale") && screen.contains("applied"),
        "{screen}"
    );
    assert!(
        screen.contains("Stale and failed items are kept"),
        "{screen}"
    );
    assert_eq!(git_status(&tui.t), "A  home/dot_fresh");
    assert_eq!(summaries(&tui.t), ["Track ~/.moves"]);
}

#[test]
fn enter_with_nothing_selected_says_so() {
    let t = initialised();
    write(&t, ".a", "a");
    t.dw_ok(&["track", ".a"]);
    let mut tui = open(t);
    tui.press('0');
    tui.press(' ');
    tui.press_key(KeyCode::Enter);
    assert!(tui.app.dialog.is_none());
    assert!(tui.screen().contains("no pending items selected"));
}

#[test]
fn the_pending_pane_is_usable_in_a_single_narrow_panel() {
    let t = initialised();
    write(&t, ".a", "a");
    t.dw_ok(&["track", ".a"]);
    let mut tui = open(t);
    tui.press('0');
    let screen = tui.screen_at(45, 14);
    assert!(
        screen.contains("[0] Pending (1)") && screen.contains("Track ~/.a"),
        "{screen}"
    );
    assert!(!screen.contains("[2] Files"), "{screen}");
}

#[test]
fn changes_made_by_the_cli_appear_after_refresh() {
    let t = initialised();
    write(&t, ".a", "a");
    let mut tui = open(t);
    assert!(pending(&tui.t).is_empty());
    tui.t.dw_ok(&["track", ".a", "--apply"]);
    tui.press('r');
    let screen = tui.screen();
    assert!(screen.contains("✓  .a"), "{screen}");
    cli(&tui.t, &["untrack", ".a"]);
    tui.press('r');
    assert!(tui.screen().contains("[x] #1 human: Untrack ~/.a"));
}

#[test]
fn help_lists_the_new_keys() {
    let t = initialised();
    let mut tui = open(t);
    tui.press('?');
    let screen = tui.screen_at(100, 40);
    for needle in [
        "Space",
        "range selection",
        "hide (or un-hide) the path",
        "drop the selected pending item",
        "review and apply",
    ] {
        assert!(screen.contains(needle), "{needle}:\n{screen}");
    }
}

// ---- off the event loop, and alongside the CLI -------------------------------------------------

#[test]
fn space_returns_at_once_while_a_slow_chezmoi_works_and_keys_keep_flowing() {
    let t = initialised();
    write(&t, ".a", "a");
    write(&t, ".b", "b");
    // A chezmoi that is slow only for the preview of `add`; git is the real one.
    let fake = t.root.path().join("slow-libexec");
    fs::create_dir_all(&fake).unwrap();
    symlink(libexec().join("git"), fake.join("git")).unwrap();
    let script = format!(
        "#!/bin/sh\nsleep 2\nexec {} \"$@\"\n",
        libexec().join("chezmoi").display()
    );
    fs::write(fake.join("chezmoi"), script).unwrap();
    fs::set_permissions(fake.join("chezmoi"), fs::Permissions::from_mode(0o755)).unwrap();
    let store = Store::new(t.dirs(), Toolchain::at(&fake), t.env.clone());
    let mut tui = open_with(t, store);

    tui.select(".a");
    let start = Instant::now();
    tui.app.handle_event(ch(' '));
    assert!(
        start.elapsed() < Duration::from_millis(150),
        "Space blocked the loop for {:?}",
        start.elapsed()
    );
    assert!(tui.app.actions_busy());
    // The loop keeps handling input while the action runs, and queues more.
    let start = Instant::now();
    tui.app.handle_event(ch('j'));
    tui.app.handle_event(ch(' '));
    tui.app.handle_event(ch('?'));
    tui.app.handle_event(key(KeyCode::Esc));
    assert!(start.elapsed() < Duration::from_millis(150));
    assert!(tui.app.files.selected >= 1);
    assert!(!tui.app.show_help);
    assert!(tui.screen().contains("working"), "{}", tui.screen());
    // Both actions complete, in order.
    tui.settle();
    assert_eq!(summaries(&tui.t), ["Track ~/.a", "Track ~/.b"]);
}

#[test]
fn the_tui_and_concurrent_cli_commands_share_one_consistent_pending_list() {
    let t = initialised();
    for i in 0..4 {
        write(&t, &format!(".t{i}"), "t");
        write(&t, &format!(".c{i}"), "c");
    }
    let mut tui = open(t);
    let handles: Vec<_> = (0..4)
        .map(|i| {
            let env = tui.t.env.clone();
            let home = tui.t.home.clone();
            std::thread::spawn(move || {
                Command::new(env!("CARGO_BIN_EXE_dotwarden"))
                    .args(["track", &format!(".c{i}")])
                    .env_clear()
                    .envs(&env)
                    .current_dir(&home)
                    .output()
                    .unwrap()
            })
        })
        .collect();
    for i in 0..4 {
        tui.select(&format!(".t{i}"));
        tui.app.handle_event(ch(' '));
    }
    for h in handles {
        assert!(h.join().unwrap().status.success());
    }
    tui.settle();
    let items = pending(&tui.t);
    assert_eq!(items.len(), 8);
    let ids: BTreeSet<u64> = items.iter().map(|i| i["id"].as_u64().unwrap()).collect();
    assert_eq!(ids, (1..=8).collect());
    tui.press('r');
    assert_eq!(tui.app.pending.items.len(), 8);
}

// ---- review fixes ------------------------------------------------------------------------

#[test]
fn space_never_creates_an_item_that_overlaps_a_pending_folder_item() {
    let t = initialised();
    for f in ["d/a", "d/b", "e/x"] {
        write(&t, f, f);
    }
    t.dw_ok(&["track", "d"]);
    t.dw_ok(&["track", "e/x"]);
    let mut tui = open(t);
    // A file inside the pending folder item.
    tui.enter("d");
    tui.select("a");
    tui.press(' ');
    let msg = tui.app.message.clone().unwrap_or_default();
    assert!(msg.contains("overlaps pending item #1"), "{msg}");
    assert_eq!(pending(&tui.t).len(), 2, "no overlapping item was added");
    // The folder above a pending file item.
    tui.press('h');
    tui.select("e");
    tui.press(' ');
    let msg = tui.app.message.clone().unwrap_or_default();
    assert!(msg.contains("overlaps pending item #2"), "{msg}");
    assert_eq!(pending(&tui.t).len(), 2);
    // The folder item itself still toggles off.
    tui.select("d");
    tui.press(' ');
    assert_eq!(summaries(&tui.t), ["Track ~/e/x"]);
}

#[test]
fn a_lock_timeout_shows_busy_try_again_and_changes_nothing() {
    let mut t = initialised();
    t.set("DOTWARDEN_LOCK_WAIT_MS", "200");
    write(&t, ".a", "a");
    let mut tui = open(t);
    tui.select(".a");
    let held = tui.t.store().lock().unwrap();
    tui.press(' ');
    let screen = tui.screen();
    assert!(
        screen.contains("Busy") && screen.contains("try again"),
        "{screen}"
    );
    assert!(
        !screen.contains("another dotwarden command has held"),
        "no hard failure text:\n{screen}"
    );
    drop(held);
    assert!(pending(&tui.t).is_empty());
    tui.press(' ');
    assert_eq!(summaries(&tui.t), ["Track ~/.a"]);
}

#[test]
fn i_hides_a_path_through_a_pending_item_and_the_list_updates_after_it_is_applied() {
    let t = initialised();
    write(&t, "Projects/x", "x");
    write(&t, ".zshrc", "z");
    let mut tui = open(t);
    tui.select("Projects");
    tui.press('i');
    assert_eq!(summaries(&tui.t), ["Add hide rule `/Projects/`"]);
    assert!(tui.screen().contains("Added #1"));
    tui.press('i');
    assert!(pending(&tui.t).is_empty(), "i again drops the item");
    tui.press('i');
    tui.press('0');
    tui.press_key(KeyCode::Enter);
    tui.press_key(KeyCode::Enter);
    let hide = fs::read_to_string(tui.t.dirs().repo().join("hide")).unwrap();
    assert!(hide.lines().any(|l| l == "/Projects/"), "{hide}");
    assert_eq!(git_status(&tui.t), "M  hide");
    tui.press_key(KeyCode::Char('x')); // close the result
    tui.press('2');
    let screen = tui.screen();
    assert!(
        !screen.contains("Projects/"),
        "the browser dropped the hidden folder:\n{screen}"
    );
    assert!(screen.contains(".zshrc"));
    // Revealed with `.`, `i` offers to remove exactly that rule.
    tui.press('.');
    tui.select("Projects");
    tui.press('i');
    assert_eq!(summaries(&tui.t), ["Remove hide rule `/Projects/`"]);
}

#[test]
fn i_on_a_path_hidden_by_another_rule_points_at_dw_hide() {
    let t = initialised();
    write(&t, ".cache/x", "c");
    let mut tui = open(t);
    tui.press('.');
    tui.select(".cache");
    tui.press('i');
    let screen = tui.screen();
    assert!(pending(&tui.t).is_empty());
    assert!(
        screen.contains("edit the rules with `dw hide`") && screen.contains(".cache/"),
        "{screen}"
    );
}

#[test]
fn hide_rules_changed_from_the_cli_are_reloaded_without_a_restart() {
    let t = initialised();
    write(&t, "Zed/settings", "s");
    write(&t, ".zshrc", "z");
    let mut tui = open(t);
    assert!(tui.screen().contains("Zed/"));
    tui.t.dw_ok(&["hide", "add", "/Zed/", "--apply"]);
    // The periodic tick notices the changed file.
    tui.app.handle_event(Event::Tick);
    tui.settle();
    let screen = tui.screen();
    assert!(!screen.contains("Zed/"), "{screen}");
    assert!(screen.contains(".zshrc"));
    // And back again via `r`.
    tui.t.dw_ok(&["hide", "remove", "/Zed/", "--apply"]);
    tui.press('r');
    assert!(tui.screen().contains("Zed/"));
}

#[test]
fn the_tick_also_picks_up_pending_items_added_by_the_cli() {
    let t = initialised();
    write(&t, ".a", "a");
    let mut tui = open(t);
    tui.t.dw_ok(&["track", ".a"]);
    tui.app.handle_event(Event::Tick);
    tui.settle();
    assert!(tui.screen().contains("[x] #1 human: Track ~/.a"));
}
