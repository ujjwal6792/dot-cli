//! `dw sync` and the backup state (T17) with real git and chezmoi, local bare
//! remotes and no network.

mod common;

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

use common::TestEnv;
use crossterm::event::{Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, KeyEventState};

struct Run {
    code: i32,
    out: String,
    err: String,
}

fn run_with(t: &TestEnv, agent: bool, args: &[&str]) -> Run {
    let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_dotwarden"));
    c.args(args)
        .env_clear()
        .envs(&t.env)
        .current_dir(&t.home)
        .stdin(std::process::Stdio::null());
    if agent {
        c.env("DOTWARDEN_AGENT", "claude");
    }
    let o = c.output().unwrap();
    Run {
        code: o.status.code().unwrap(),
        out: String::from_utf8_lossy(&o.stdout).into_owned(),
        err: String::from_utf8_lossy(&o.stderr).into_owned(),
    }
}

fn dw(t: &TestEnv, args: &[&str]) -> Run {
    run_with(t, false, args)
}

fn write(t: &TestEnv, rel: &str, content: &str) {
    let p = t.home.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

fn read(t: &TestEnv, rel: &str) -> String {
    fs::read_to_string(t.home.join(rel)).unwrap()
}

fn repo(t: &TestEnv) -> PathBuf {
    t.dirs().repo()
}

fn head(t: &TestEnv) -> String {
    t.git(&repo(t), &["rev-parse", "HEAD"])
}

fn nothing_staged(t: &TestEnv) -> bool {
    let mut c = t.store().git_command(&repo(t));
    c.args(["diff", "--cached", "--quiet"]);
    c.status().unwrap().success()
}

/// Machine A tracking `.zshrc` and `.vimrc`, pushed to a new bare remote.
fn machine_a() -> (TestEnv, PathBuf) {
    let a = TestEnv::new();
    a.dw_ok(&["init", "--name", "alpha"]);
    write(&a, ".zshrc", "z1\n");
    write(&a, ".vimrc", "v1\n");
    a.dw_ok(&["track", ".zshrc", ".vimrc", "--apply"]);
    a.dw_ok(&["commit", "-m", "base"]);
    let remote = a.root.path().join("remote.git");
    a.git(
        &a.home,
        &[
            "init",
            "--bare",
            "--quiet",
            "--initial-branch=main",
            remote.to_str().unwrap(),
        ],
    );
    a.dw_ok(&["remote", "add", "origin", remote.to_str().unwrap(), "--yes"]);
    a.dw_ok(&["push", "--yes"]);
    (a, remote)
}

/// Machine B cloned from A's remote, with A's files applied.
fn machine_b(remote: &std::path::Path) -> TestEnv {
    let b = TestEnv::new();
    b.dw_ok(&["clone", remote.to_str().unwrap(), "--name", "beta"]);
    b.dw_ok(&["apply", "--all"]);
    b
}

#[test]
fn two_machines_receive_each_others_edits() {
    let (a, remote) = machine_a();
    let b = machine_b(&remote);

    write(&a, ".zshrc", "z from a\n");
    a.dw_ok(&["sync", "--yes"]);

    write(&b, ".vimrc", "v from b\n");
    let r = dw(&b, &["sync", "--yes"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert_eq!(read(&b, ".zshrc"), "z from a\n", "B received A's edit");
    assert!(r.out.contains("apply   ~/.zshrc"), "{}", r.out);
    assert!(
        r.out
            .contains("commit  sync from beta: 1 changed, 0 new (~/.vimrc)"),
        "{}",
        r.out
    );

    let r = dw(&a, &["sync", "--yes"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert_eq!(read(&a, ".vimrc"), "v from b\n", "A received B's edit");
    assert_eq!(head(&a), b.git(&remote, &["rev-parse", "main"]));
    let s = a.dw_json(&["status"]);
    assert_eq!(s["backup"]["unbacked"], 0, "{s}");
    assert!(
        s["backup"]["summary"].is_null(),
        "nothing shown when pushed"
    );
    assert!(s["backup"]["last_pushed"].is_string());
}

#[test]
fn without_a_remote_sync_commits_locally_and_names_remote_add() {
    let t = TestEnv::new();
    t.dw_ok(&["init", "--name", "alpha"]);
    write(&t, ".zshrc", "z1\n");
    t.dw_ok(&["track", ".zshrc", "--apply"]);
    let before = head(&t);

    let r = dw(&t, &["sync", "--yes"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert_ne!(head(&t), before, "the staged track was committed");
    assert!(nothing_staged(&t));
    assert!(r.out.contains("dw remote add origin URL"), "{}", r.out);
}

#[test]
fn a_conflict_stops_sync_before_anything_is_staged() {
    let (a, remote) = machine_a();
    let b = machine_b(&remote);
    write(&a, ".zshrc", "z from a\n");
    a.dw_ok(&["sync", "--yes"]);

    write(&b, ".zshrc", "z from b\n");
    write(&b, ".vimrc", "v from b\n");
    let r = dw(&b, &["sync", "--yes"]);
    assert_eq!(r.code, 5, "{}{}", r.out, r.err);
    assert!(r.err.contains("dw resolve"), "{}", r.err);
    assert!(nothing_staged(&b), "nothing was staged");
    assert_eq!(read(&b, ".zshrc"), "z from b\n", "HOME side kept");
    assert_eq!(
        head(&b),
        b.git(&remote, &["rev-parse", "main"]),
        "the pull still ran"
    );
}

#[test]
fn a_secret_in_a_tracked_folder_is_skipped_and_never_committed() {
    let (a, _remote) = machine_a();
    write(&a, ".config/app/settings", "theme = dark\n");
    a.dw_ok(&["track", ".config/app", "--apply"]);
    a.dw_ok(&["commit", "-m", "app"]);
    let token = "ghp_Abcdefghijklmnopqrstuvwxyz0123456789";
    write(&a, ".config/app/token", &format!("token = {token}\n"));
    write(&a, ".config/app/extra", "plain\n");

    let r = dw(&a, &["sync", "--yes"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert!(
        r.out
            .contains("skip    ~/.config/app/token: holds a secret"),
        "{}",
        r.out
    );
    assert!(
        r.out.contains("stage   ~/.config/app/extra (new)"),
        "{}",
        r.out
    );
    assert!(!r.out.contains(token) && !r.err.contains(token));
    let history = a.git(&repo(&a), &["log", "-p", "--all"]);
    assert!(
        history.contains("plain"),
        "the other new file was committed"
    );
    assert!(!history.contains(token), "the secret never reached git");
    let s = a.dw_json(&["status"]);
    assert_eq!(s["backup"]["unbacked"], 1, "the token stays unbacked");
}

#[test]
fn an_agent_sync_commits_locally_leaves_a_pending_push_and_undo_reverts_it() {
    let (a, remote) = machine_a();
    let pushed = head(&a);
    write(&a, ".zshrc", "agent edit\n");

    let r = run_with(&a, true, &["sync", "--yes"]);
    assert_eq!(r.code, 3, "{}{}", r.out, r.err);
    let msg = a.git(&repo(&a), &["log", "-1", "--format=%B"]);
    assert!(
        msg.starts_with("sync from alpha: 1 changed") && msg.contains("Agent: claude"),
        "{msg}"
    );
    assert_eq!(a.git(&remote, &["rev-parse", "main"]), pushed, "not pushed");
    let items = a.dw_json(&["pending", "list"])["items"].clone();
    assert!(
        items[0]["summary"]
            .as_str()
            .unwrap()
            .starts_with("Push main to origin"),
        "{items}"
    );

    let out = a.dw_ok(&["undo"]);
    assert!(out.contains("commit"), "{out}");
    assert_eq!(head(&a), pushed, "the sync commit is gone");
    assert!(!nothing_staged(&a), "its changes stay staged");
}

#[test]
fn dry_run_shows_the_plan_and_writes_nothing() {
    let (a, _remote) = machine_a();
    write(&a, ".zshrc", "edited\n");
    write(&a, ".config/x", "untracked\n");
    let before = (
        head(&a),
        a.git(&repo(&a), &["ls-files", "-s"]),
        fs::read(a.dirs().undo_file()).ok(),
    );
    let v = a.dw_json(&["sync", "--dry-run"]);
    assert_eq!(v["plan"]["stage"][0], "~/.zshrc", "{v}");
    assert_eq!(v["plan"]["push_commits"], 1, "{v}");
    assert!(
        v["plan"]["message"]
            .as_str()
            .unwrap()
            .starts_with("sync from alpha: 1 changed, 0 new")
    );
    let after = (
        head(&a),
        a.git(&repo(&a), &["ls-files", "-s"]),
        fs::read(a.dirs().undo_file()).ok(),
    );
    assert_eq!(before, after);
    assert!(nothing_staged(&a));
    assert!(
        a.dw_json(&["pending", "list"])["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(read(&a, ".zshrc"), "edited\n");
}

#[test]
fn status_reports_backup_state_and_warns_past_the_preference() {
    let (a, _remote) = machine_a();
    let s = a.dw_json(&["status"]);
    assert!(s["backup"]["summary"].is_null(), "{s}");

    write(&a, ".zshrc", "edited\n");
    let s = a.dw_json(&["status"]);
    assert_eq!(s["backup"]["unbacked"], 1);
    assert_eq!(s["backup"]["warning"], false);
    assert_eq!(s["backup"]["summary"], "last pushed today · 1 unbacked");

    let ten_days_ago = dotwarden::snapshot::now_unix() - 10 * 86_400;
    fs::write(
        a.dirs().last_push_file(),
        format!(r#"{{"at": {ten_days_ago}, "remote": "origin", "branch": "main"}}"#),
    )
    .unwrap();
    let s = a.dw_json(&["status"]);
    assert_eq!(s["backup"]["warning"], true, "{s}");
    assert!(
        a.dw_ok(&["status"])
            .contains("last pushed 10 days ago · 1 unbacked — run dw sync")
    );

    write(
        &a,
        ".config/dotwarden/config.toml",
        "backup_warn_days = 30\n",
    );
    let s = a.dw_json(&["status"]);
    assert_eq!(s["backup"]["warning"], false, "{s}");
}

#[test]
fn tui_s_shows_the_plan_and_y_syncs() {
    use dotwarden::tui::app::{App, Dialog};
    use dotwarden::tui::event::Event;
    let (a, remote) = machine_a();
    write(&a, ".zshrc", "from the tui\n");

    let store = Arc::new(a.store());
    let rules = dotwarden::hide::HideRules::for_store(&store).unwrap();
    let (tx, rx) = channel();
    let mut app = App::new(store, rules, tx.clone());
    app.start();
    let settle = |app: &mut App| {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match rx.recv_timeout(Duration::from_millis(300)) {
                Ok(ev) => app.handle_event(ev),
                Err(_) if app.in_flight_jobs() == 0 && !app.actions_busy() => return,
                Err(_) => {}
            }
            assert!(Instant::now() < deadline, "the TUI never settled");
        }
    };
    let press = |app: &mut App, c: char| {
        app.handle_event(Event::Input(TermEvent::Key(KeyEvent {
            code: KeyCode::Char(c),
            modifiers: crossterm::event::KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })))
    };
    settle(&mut app);
    press(&mut app, 'S');
    settle(&mut app);
    let Some(Dialog::Sync(plan)) = &app.dialog else {
        panic!("no sync dialog: {:?}", app.message);
    };
    assert_eq!(plan.stage, ["~/.zshrc"]);
    assert_eq!(plan.push_commits, 1);
    press(&mut app, 'y');
    settle(&mut app);
    assert!(app.dialog.is_none(), "{:?}", app.dialog);
    assert_eq!(head(&a), a.git(&remote, &["rev-parse", "main"]), "pushed");
    assert!(
        a.git(&repo(&a), &["log", "-1", "--format=%s"])
            .starts_with("sync from alpha")
    );
}
