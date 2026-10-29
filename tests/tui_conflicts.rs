//! The Conflicts screen: hunk picking, masked secrets and the editor session,
//! driven with real key events against two machines over a local bare remote.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
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
use dotwarden::tui::app::App;
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
    for _ in 0..3 {
        tui.press(']');
    }
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

fn read(t: &TestEnv, rel: &str) -> String {
    fs::read_to_string(t.home.join(rel)).unwrap()
}

const PASS: &str = "correct horse battery";

fn bare(t: &TestEnv) -> PathBuf {
    let p = t.root.path().join("remote.git");
    t.git(
        &t.home,
        &[
            "init",
            "--bare",
            "--quiet",
            "--initial-branch=main",
            p.to_str().unwrap(),
        ],
    );
    p
}

fn dw_with(t: &TestEnv, extra: &[(&str, &str)], args: &[&str]) -> (i32, String) {
    let o = std::process::Command::new(env!("CARGO_BIN_EXE_dotwarden"))
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

fn vault_env() -> TestEnv {
    let mut t = TestEnv::new();
    let rt = t.root.path().canonicalize().unwrap().join("rt");
    fs::create_dir_all(&rt).unwrap();
    t.set("XDG_RUNTIME_DIR", rt);
    t.set("DOTWARDEN_SESSION_BACKEND", "file");
    t.set("DOTWARDEN_TEST_WORK_FACTOR", "4");
    t
}

fn sync(from: &TestEnv, to: &TestEnv) {
    from.dw_ok(&["push", "--yes"]);
    to.dw_ok(&["pull"]);
}

const BASE: &str = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n";

/// alpha and beta share `.a` (BASE); beta has applied it.
fn pair() -> (TestEnv, TestEnv) {
    let a = TestEnv::new();
    a.dw_ok(&["init", "--name", "alpha"]);
    write(&a, ".a", BASE);
    a.dw_ok(&["track", ".a", "--apply"]);
    a.dw_ok(&["commit", "-m", "base"]);
    let remote = bare(&a);
    a.dw_ok(&["remote", "add", "origin", remote.to_str().unwrap(), "--yes"]);
    a.dw_ok(&["push", "--yes"]);
    let b = TestEnv::new();
    b.dw_ok(&["clone", remote.to_str().unwrap(), "--name", "beta"]);
    b.dw_ok(&["push", "--yes"]);
    b.dw_ok(&["apply", "--all"]);
    a.dw_ok(&["pull"]);
    (a, b)
}

#[test]
fn hunks_are_picked_with_l_and_i_while_other_changes_merge_automatically() {
    let (a, b) = pair();
    write(&b, ".a", "l1\nL2\nl3\nl4\nL5\nl6\nl7\nl8\nL9\nl10\n");
    write(&a, ".a", "l1\nI2\nl3\nl4\nl5\nl6\nI7\nl8\nI9\nl10\n");
    a.dw_ok(&["stage", ".a"]);
    a.dw_ok(&["commit", "-m", "alpha"]);
    sync(&a, &b);
    let mut tui = open(b);
    assert_eq!(tui.changes(), [".a:conflict"]);
    tui.send(KeyCode::Enter, KeyModifiers::NONE);
    let screen = tui.screen();
    assert!(
        screen.contains("hunk 1/2") && screen.contains("undecided"),
        "{screen}"
    );
    assert!(
        !screen.contains("I7") && !screen.contains("L5"),
        "auto-merged lines are not hunks"
    );

    // Writing before every hunk is decided is refused.
    tui.press('w');
    assert!(tui.screen().contains("choose l / i / b"));
    tui.press('l');
    tui.press('i');
    let screen = tui.screen();
    assert!(
        screen.contains("→ local") && screen.contains("→ incoming"),
        "{screen}"
    );
    // n at the prompt writes nothing; the screen stays open.
    tui.press('w');
    assert!(tui.screen().contains("stage it? y / n"));
    tui.press('n');
    assert_eq!(
        read(&tui.t, ".a"),
        "l1\nL2\nl3\nl4\nL5\nl6\nl7\nl8\nL9\nl10\n"
    );
    tui.press('w');
    tui.press('y');
    assert_eq!(
        read(&tui.t, ".a"),
        "l1\nL2\nl3\nl4\nL5\nl6\nI7\nl8\nI9\nl10\n"
    );
    assert_eq!(tui.changes(), [".a:staged"]);
    assert!(tui.app.conflict.is_none());
}

#[test]
fn esc_leaves_the_conflict_untouched() {
    let (a, b) = pair();
    write(&b, ".a", "l1\nL2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n");
    write(&a, ".a", "l1\nI2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n");
    a.dw_ok(&["stage", ".a"]);
    a.dw_ok(&["commit", "-m", "alpha"]);
    sync(&a, &b);
    let mut tui = open(b);
    tui.send(KeyCode::Enter, KeyModifiers::NONE);
    tui.press('i');
    tui.send(KeyCode::Esc, KeyModifiers::NONE);
    assert!(tui.app.conflict.is_none());
    assert_eq!(tui.changes(), [".a:conflict"]);
    assert!(read(&tui.t, ".a").contains("L2"));
}

const SRV_V: &str = "Srv-Only-Val-1f2e3d4c";
const DEFAULT_V: &str = "Default-Val-9a8b7c6d";

#[test]
fn a_conflict_around_a_secret_never_shows_or_prints_its_value() {
    let a = vault_env();
    a.dw_ok(&["init", "--name", "mac"]);
    dw_with(
        &a,
        &[("DOTWARDEN_TEST_NEW_PASSPHRASE", PASS)],
        &["vault", "init"],
    );
    dw_with(
        &a,
        &[("DOTWARDEN_TEST_PASSPHRASE", PASS)],
        &["vault", "unlock"],
    );
    a.dw_ok(&["commit", "-m", "vault"]);
    let remote = bare(&a);
    a.dw_ok(&["remote", "add", "origin", remote.to_str().unwrap(), "--yes"]);
    a.dw_ok(&["push", "--yes"]);
    let b = vault_env();
    b.dw_ok(&["clone", remote.to_str().unwrap(), "--name", "srv"]);
    b.dw_ok(&["push", "--yes"]);
    a.dw_ok(&["pull"]);
    write(&a, ".conf", &format!("key = {DEFAULT_V}\nname = x\n"));
    a.dw_ok(&["track", ".conf", "--apply"]);
    for (target, value) in [(None, DEFAULT_V), (Some("machine:srv"), SRV_V)] {
        let mut args = vec!["values", "add-secret", "API"];
        if let Some(t) = target {
            args.extend(["--target", t]);
        }
        let (code, out) = dw_with(&a, &[("DOTWARDEN_TEST_SECRET_VALUE", value)], &args);
        assert_eq!(code, 0, "{out}");
        if target.is_none() {
            a.dw_ok(&["values", "wire", ".conf", "API", "--apply"]);
        }
    }
    a.dw_ok(&["commit", "-m", "api"]);
    sync(&a, &b);
    dw_with(
        &b,
        &[("DOTWARDEN_TEST_PASSPHRASE", PASS)],
        &["vault", "unlock"],
    );
    b.dw_ok(&["apply", "--all"]);
    assert_eq!(read(&b, ".conf"), format!("key = {SRV_V}\nname = x\n"));

    // srv annotates the secret's line; mac changes the next line.
    write(&b, ".conf", &format!("key = {SRV_V} # note\nname = x\n"));
    write(&a, ".conf", &format!("key = {DEFAULT_V}\nname = y\n"));
    a.dw_ok(&["stage", ".conf"]);
    a.dw_ok(&["commit", "-m", "name"]);
    sync(&a, &b);
    assert_eq!(b.dw_json(&["status"])["changes"][0]["state"], "conflict");

    for args in [
        &["resolve", ".conf", "--merge"][..],
        &["resolve", ".conf", "--merge", "--json"][..],
    ] {
        let (code, out) = dw_with(&b, &[], args);
        assert_eq!(code, 5, "{out}");
        assert!(!out.contains(SRV_V) && !out.contains(DEFAULT_V), "{out}");
    }
    let mut tui = open(b);
    tui.send(KeyCode::Enter, KeyModifiers::NONE);
    let screen = tui.screen();
    assert!(screen.contains("«secret:API»"), "{screen}");
    assert!(
        !screen.contains(SRV_V) && !screen.contains(DEFAULT_V),
        "{screen}"
    );
    tui.press('l');
    tui.press('w');
    let screen = tui.screen();
    assert!(!screen.contains(SRV_V), "{screen}");
    tui.press('y');
    // The reference stays in the repository; the value never reaches it.
    let stored = fs::read_to_string(tui.t.dirs().repo().join("home/dot_conf.tmpl")).unwrap();
    assert!(
        stored.contains("\"API\"") && stored.contains("# note"),
        "{stored}"
    );
    assert!(!stored.contains(SRV_V));
    assert_eq!(
        read(&tui.t, ".conf"),
        format!("key = {SRV_V} # note\nname = x\n")
    );
    assert_eq!(tui.t.dw_json(&["status"])["changes"][0]["state"], "staged");
}

#[test]
fn the_editor_session_gives_nvim_a_diff_and_returns_the_edited_merge() {
    let (a, b) = pair();
    write(&b, ".a", "l1\nL2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n");
    write(&a, ".a", "l1\nI2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n");
    a.dw_ok(&["stage", ".a"]);
    a.dw_ok(&["commit", "-m", "alpha"]);
    sync(&a, &b);
    let script = b.root.path().join("ed.sh");
    fs::write(&script, "#!/bin/sh\ngrep -q '<<<<<<< local' \"$1\" || exit 9\nprintf 'l1\\nboth\\nl3\\nl4\\nl5\\nl6\\nl7\\nl8\\nl9\\nl10\\n' > \"$1\"\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    let mut b = b;
    b.set("EDITOR", script.to_str().unwrap());
    let store = b.store();
    let loaded = dotwarden::conflicts::load(&store, ".a").unwrap();
    let s = dotwarden::conflicts::EditSession::new(&store, &loaded, &[None]).unwrap();
    let dir = s.merge.parent().unwrap().to_path_buf();
    assert_eq!(
        fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(&s.merge).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(s.command(&store).status().unwrap().success());
    let text = s.read_result().unwrap();
    assert!(text.contains("both"));
    drop(s);
    assert!(!dir.exists(), "the private merge directory is removed");

    // Markers left in the file are refused.
    let s = dotwarden::conflicts::EditSession::new(&store, &loaded, &[None]).unwrap();
    assert!(s.read_result().is_err());
    // nvim opens merge, local and incoming side by side in diff mode.
    let mut b = b;
    b.set("EDITOR", "nvim");
    let cmd = s.command(&b.store());
    let args: Vec<_> = cmd
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert!(args[1].starts_with("nvim -d "), "{args:?}");
    assert_eq!(args.len(), 6, "{args:?}");
}
