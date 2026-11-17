//! `dw import` (T18) with real git and chezmoi: a bare repository over an
//! isolated HOME, path lists from a file and from stdin.

mod common;

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use common::TestEnv;

struct Run {
    code: i32,
    out: String,
    err: String,
}

fn run(t: &TestEnv, agent: bool, args: &[&str], stdin: Option<&str>) -> Run {
    let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_dotwarden"));
    c.args(args)
        .env_clear()
        .envs(&t.env)
        .current_dir(&t.home)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if agent {
        c.env("DOTWARDEN_AGENT", "claude");
    }
    let mut child = c.spawn().unwrap();
    if let Some(input) = stdin {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
    }
    let o = child.wait_with_output().unwrap();
    Run {
        code: o.status.code().unwrap(),
        out: String::from_utf8_lossy(&o.stdout).into_owned(),
        err: String::from_utf8_lossy(&o.stderr).into_owned(),
    }
}

fn dw(t: &TestEnv, args: &[&str]) -> Run {
    run(t, false, args, None)
}

fn write(t: &TestEnv, rel: &str, content: &str) {
    let p = t.home.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

const TOKEN: &str = "gho_16C7e42F292c6912E7710c838347Ae178B4a";

/// HOME with an old bare repository `~/.cfg` listing an nvim folder fully, a
/// fish folder partly (its `functions/` fully), a `.cache` path, a gh folder
/// with a token, `.zshrc` and a file since deleted from HOME.
fn old_setup() -> (TestEnv, PathBuf, String) {
    let t = TestEnv::new();
    t.dw_ok(&["init", "--name", "alpha"]);
    let listed = [
        (".zshrc", "export EDITOR=nvim\n".to_string()),
        (".config/nvim/init.lua", "require('plugins')\n".into()),
        (".config/nvim/lua/plugins.lua", "return {}\n".into()),
        (".config/fish/config.fish", "set -x A 1\n".into()),
        (
            ".config/fish/functions/hi.fish",
            "function hi; end\n".into(),
        ),
        (".cache/zsh/compdump", "dump\n".into()),
        (
            ".config/gh/hosts.yml",
            format!("github.com:\n    oauth_token: {TOKEN}\n    user: someone\n"),
        ),
        (".config/gh/config.yml", "editor: nvim\n".into()),
        (".gone", "old\n".into()),
    ];
    for (rel, body) in &listed {
        write(&t, rel, body);
    }
    write(&t, ".config/fish/fish_variables", "SETUVAR x\n");
    let bare = t.home.join(".cfg");
    t.git(
        &t.home,
        &["init", "--bare", "--quiet", bare.to_str().unwrap()],
    );
    let git_dir = format!("--git-dir={}", bare.display());
    let work_tree = format!("--work-tree={}", t.home.display());
    let mut add = vec![git_dir.as_str(), work_tree.as_str(), "add"];
    add.extend(listed.iter().map(|(r, _)| *r));
    t.git(&t.home, &add);
    t.git(
        &t.home,
        &[&git_dir, &work_tree, "commit", "--quiet", "-m", "old"],
    );
    let head = t.git(&t.home, &[&git_dir, "rev-parse", "--short", "HEAD"]);
    fs::remove_file(t.home.join(".gone")).unwrap();
    (t, bare, head)
}

/// Every file under `dir`, recursively (for the token check).
fn all_bytes(dir: &Path) -> String {
    common::snapshot(dir).unwrap().join("\n")
}

#[test]
fn import_from_a_bare_repo_groups_skips_and_leaves_it_untouched() {
    let (t, bare, head) = old_setup();
    let before = common::snapshot(&bare);
    let mut outputs = String::new();

    let r = dw(&t, &["import", "--bare", "~/.cfg", "--dry-run"]);
    outputs += &(r.out.clone() + &r.err);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    for line in [
        &format!("Import from ~/.cfg at {head}: 9 listed path(s)") as &str,
        "Tracked folders:\n  ~/.config/fish/functions (1 file(s))\n  ~/.config/gh (2 file(s))\n  ~/.config/nvim (2 file(s))\n",
        "Single files:\n  ~/.config/fish/config.fish\n  ~/.zshrc\n",
        "~/.cache/zsh/compdump: ~/.cache/zsh/compdump is hidden",
        "~/.gone: ~/.gone does not exist",
        "~/.config/gh/hosts.yml: possible secret (line 2",
        "Dry run: nothing was changed",
    ] {
        assert!(r.out.contains(line), "missing {line:?} in\n{}", r.out);
    }
    assert!(
        t.dw_json(&["pending", "list"])["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    // Agents may import: it only adds pending items.
    let r = run(&t, true, &["import", "--bare", "~/.cfg", "--json"], None);
    outputs += &(r.out.clone() + &r.err);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    let v: serde_json::Value = serde_json::from_str(&r.out).unwrap();
    assert_eq!(v["added"].as_array().unwrap().len(), 5, "{v}");
    assert_eq!(v["skipped"].as_array().unwrap().len(), 3, "{v}");
    let items = t.dw_json(&["pending", "list"]);
    for i in items["items"].as_array().unwrap() {
        assert_eq!(i["import"]["from"], "~/.cfg", "{i}");
        assert_eq!(i["import"]["head"], head.as_str(), "{i}");
        assert_eq!(i["requester"], "claude");
    }
    outputs += &t.dw_ok(&["pending", "apply", "--all"]);
    let pending_json = fs::read_to_string(t.dirs().pending_file()).unwrap_or_default();

    let remote = t.root.path().join("remote.git");
    t.git(
        &t.home,
        &["init", "--bare", "--quiet", remote.to_str().unwrap()],
    );
    t.dw_ok(&["remote", "add", "origin", remote.to_str().unwrap(), "--yes"]);
    let r = dw(&t, &["sync", "--yes"]);
    outputs += &(r.out.clone() + &r.err);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    let repo = t.dirs().repo();
    let message = t.git(&repo, &["log", "-1", "--format=%B"]);
    assert!(
        message.starts_with(&format!("Import from ~/.cfg at {head}\n\nsync from alpha:")),
        "{message}"
    );
    assert!(!t.dirs().import_file().exists(), "cleared by the commit");

    assert_eq!(
        common::snapshot(&bare),
        before,
        "the old repository is untouched"
    );
    let history = t.git(&repo, &["log", "-p", "--all"]);
    for (what, text) in [
        ("history", &history),
        ("outputs", &outputs),
        ("pending.json", &pending_json),
        ("repository", &all_bytes(&repo)),
    ] {
        assert!(!text.contains(TOKEN), "the token leaked into the {what}");
    }
    assert!(
        history.contains("editor: nvim"),
        "gh's clean file was committed"
    );
}

#[test]
fn import_reads_a_path_list_from_stdin() {
    let t = TestEnv::new();
    t.dw_ok(&["init", "--name", "alpha"]);
    write(&t, ".zshrc", "z\n");
    write(&t, ".vim/colors/dark.vim", "hi\n");
    write(&t, ".vim/autoload/plug.vim", "fn\n");
    let list = "# my files\n.zshrc\n\n.vim/colors/dark.vim\n.vim/autoload/plug.vim\n../outside\n/etc/hosts\n";
    let r = run(
        &t,
        false,
        &["import", "--from-file", "-", "--dry-run"],
        Some(list),
    );
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert!(
        r.out.contains("Import from stdin: 5 listed path(s)"),
        "{}",
        r.out
    );
    // `.vim` holds only folders, so they are tracked instead of it.
    assert!(
        r.out.contains(
            "Tracked folders:\n  ~/.vim/autoload (1 file(s))\n  ~/.vim/colors (1 file(s))\n"
        ),
        "{}",
        r.out
    );
    assert!(r.out.contains("Single files:\n  ~/.zshrc\n"), "{}", r.out);
    for skip in ["  ../outside: ", "  /etc/hosts: "] {
        let line = r.out.lines().find(|l| l.starts_with(skip)).unwrap_or("");
        assert!(line.contains("is outside HOME"), "{skip}: {}", r.out);
    }
}

#[test]
fn nothing_to_import_exits_1() {
    let t = TestEnv::new();
    t.dw_ok(&["init", "--name", "alpha"]);
    write(&t, "list.txt", ".missing\n");
    let r = dw(&t, &["import", "--from-file", "list.txt"]);
    assert_eq!(r.code, 1, "{}{}", r.out, r.err);
    assert!(
        r.out.contains("~/.missing: ~/.missing does not exist"),
        "{}",
        r.out
    );
    fs::create_dir_all(t.home.join("plain")).unwrap();
    let r = dw(&t, &["import", "--bare", "plain"]);
    assert_eq!(r.code, 2, "{}{}", r.out, r.err);
    assert!(
        r.err.contains("~/plain is not a git repository"),
        "{}",
        r.err
    );
}

#[test]
fn the_tui_commit_box_starts_with_the_import_line() {
    use crossterm::event::{Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, KeyEventState};
    use dotwarden::tui::app::{App, Dialog};
    use dotwarden::tui::event::Event;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    let t = TestEnv::new();
    t.dw_ok(&["init", "--name", "alpha"]);
    write(&t, ".zshrc", "z\n");
    write(&t, "list.txt", ".zshrc\n");
    t.dw_ok(&["import", "--from-file", "list.txt", "--apply"]);

    let store = Arc::new(t.store());
    let rules = dotwarden::hide::HideRules::for_store(&store).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = App::new(store, rules, tx.clone());
    app.start();
    let press = |app: &mut App, c: char| {
        app.handle_event(Event::Input(TermEvent::Key(KeyEvent {
            code: KeyCode::Char(c),
            modifiers: crossterm::event::KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })));
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match rx.recv_timeout(Duration::from_millis(300)) {
                Ok(ev) => app.handle_event(ev),
                Err(_) if app.in_flight_jobs() == 0 && !app.actions_busy() => break,
                Err(_) => {}
            }
            assert!(Instant::now() < deadline, "the TUI never settled");
        }
    };
    press(&mut app, '3');
    press(&mut app, 'c');
    match &app.dialog {
        Some(Dialog::Commit(b)) => assert_eq!(b.text, "Import from ~/list.txt"),
        other => panic!("no commit box: {other:?}"),
    }
}
