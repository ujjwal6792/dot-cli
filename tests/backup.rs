//! Backup workflow (T16) with real git and chezmoi, local bare remotes and no network.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use common::TestEnv;

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

fn repo(t: &TestEnv) -> PathBuf {
    t.dirs().repo()
}

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

fn pending_paths(t: &TestEnv) -> Vec<String> {
    t.dw_json(&["pending", "list"])["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["summary"].as_str().unwrap().to_string())
        .collect()
}

/// Machine A with `.a` tracked and pushed to a new bare remote.
fn machine_a() -> (TestEnv, PathBuf) {
    let a = TestEnv::new();
    a.dw_ok(&["init", "--name", "alpha"]);
    write(&a, ".zshrc", "from a\n");
    a.dw_ok(&["track", ".zshrc", "--apply"]);
    a.dw_ok(&["commit", "-m", "base"]);
    let remote = bare(&a);
    a.dw_ok(&["remote", "add", "origin", remote.to_str().unwrap(), "--yes"]);
    a.dw_ok(&["push", "--yes"]);
    (a, remote)
}

#[test]
fn track_adds_the_valid_paths_and_lists_the_skipped_ones() {
    let t = TestEnv::new();
    t.dw_ok(&["init", "--name", "alpha"]);
    write(&t, ".zshrc", "z\n");
    write(&t, ".config/kitty/kitty.conf", "k\n");
    write(&t, ".cache/x/y", "cache\n");
    write(&t, ".tmux.conf", "t\n");

    let batch = [
        ".zshrc",
        ".config/kitty/kitty.conf",
        ".cache/x/y",
        ".tmux.conf",
    ];
    let mut dry = vec!["track", "--dry-run", "--json"];
    dry.extend(batch);
    let v: serde_json::Value = serde_json::from_str(&dw(&t, &dry).out).unwrap();
    assert_eq!(v["items"].as_array().unwrap().len(), 3);
    assert_eq!(v["skipped"][0]["path"], ".cache/x/y");
    assert!(pending_paths(&t).is_empty(), "dry run changed nothing");

    let mut args = vec!["track"];
    args.extend(batch);
    let r = dw(&t, &args);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert!(
        r.out.contains("Skipped:") && r.out.contains(".cache/x/y: "),
        "{}",
        r.out
    );
    assert!(r.out.contains("hidden"), "{}", r.out);
    assert_eq!(pending_paths(&t).len(), 3);

    // --apply follows the same rule; nothing plannable is still exit 1.
    write(&t, ".vimrc", "v\n");
    let r = dw(&t, &["track", "--apply", ".vimrc", ".cache/x/y", ".nope"]);
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert!(
        r.out.contains("Skipped:") && r.out.contains(".nope"),
        "{}",
        r.out
    );
    let r = dw(&t, &["track", ".cache/x/y", ".nope"]);
    assert_eq!(r.code, 1);
    // A single refused path keeps its plain error.
    let r = dw(&t, &["track", ".cache/x/y"]);
    assert_eq!(r.code, 1);
    assert!(
        r.err.contains("is hidden") && !r.out.contains("Skipped"),
        "{}{}",
        r.out,
        r.err
    );
}

#[test]
fn clone_shares_the_registration_unless_an_agent_runs_it() {
    let (a, remote) = machine_a();
    let b = TestEnv::new();
    let out = b.dw_ok(&["clone", remote.to_str().unwrap(), "--name", "beta"]);
    assert!(out.contains("Pushed the machine registration"), "{out}");
    assert!(!out.contains("run dw push"), "{out}");
    a.dw_ok(&["pull"]);
    let names: Vec<String> = dotwarden::repo::read_registry(&repo(&a))
        .unwrap()
        .into_iter()
        .map(|m| m.name)
        .collect();
    assert_eq!(
        names,
        ["alpha", "beta"],
        "beta reached the remote without a push"
    );
    assert!(b.dw_ok(&["status"]).contains("ahead 0"));

    let c = TestEnv::new();
    let r = run_with(
        &c,
        true,
        &["clone", remote.to_str().unwrap(), "--name", "gamma"],
    );
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert!(
        r.out
            .contains("run dw push to share this machine with the others"),
        "{}",
        r.out
    );
    let tip = c.git(&repo(&c), &["rev-parse", "HEAD"]);
    assert_ne!(
        c.git(&remote, &["rev-parse", "main"]),
        tip,
        "the agent's commit stays local"
    );
}

#[test]
fn a_successful_push_removes_the_pending_push_it_satisfied() {
    let (a, remote) = machine_a();
    write(&a, ".zshrc", "two\n");
    a.dw_ok(&["stage", ".zshrc"]);
    a.dw_ok(&["commit", "-m", "second"]);
    let r = dw(&a, &["push"]);
    assert_eq!(r.code, 3, "{}{}", r.out, r.err);
    assert_eq!(pending_paths(&a).len(), 1);

    let out = a.dw_ok(&["push", "--yes"]);
    assert!(out.contains("removed pending #1"), "{out}");
    assert!(pending_paths(&a).is_empty());
    assert_eq!(
        a.git(&remote, &["rev-parse", "main"]),
        a.git(&repo(&a), &["rev-parse", "HEAD"])
    );
}

#[test]
fn apply_lists_the_existing_files_it_replaced_and_how_to_undo() {
    let (_a, remote) = machine_a();
    let b = TestEnv::new();
    write(&b, ".zshrc", "mine\n");
    b.dw_ok(&["clone", remote.to_str().unwrap(), "--name", "beta"]);

    let dry = b.dw_ok(&["apply", "--all", "--dry-run"]);
    assert!(
        dry.contains("replaced existing files") && dry.contains("~/.zshrc"),
        "{dry}"
    );
    assert_eq!(fs::read_to_string(b.home.join(".zshrc")).unwrap(), "mine\n");

    let out = b.dw_ok(&["apply", "--all"]);
    assert!(
        out.contains("replaced existing files") && out.contains("~/.zshrc"),
        "{out}"
    );
    assert!(out.contains("dw snapshot restore"), "{out}");
    assert_eq!(
        fs::read_to_string(b.home.join(".zshrc")).unwrap(),
        "from a\n"
    );

    // Written by dotwarden now: a later change is not a "replaced existing" file.
    let v = b.dw_json(&["apply", "--all"]);
    assert!(v["replaced_existing"].as_array().unwrap().is_empty());
}

#[test]
fn remote_add_warns_on_https_to_a_known_host_but_still_adds_it() {
    let mut t = TestEnv::new();
    t.dw_ok(&["init", "--name", "alpha"]);
    let probe = t.root.path().join("probe.sh");
    fs::write(&probe, "#!/bin/sh\nexit 1\n").unwrap();
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o755)).unwrap();
    t.set("DOTWARDEN_VISIBILITY_PROBE", probe);
    let r = dw(
        &t,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/me/dots.git",
            "--yes",
        ],
    );
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    assert!(r.out.contains("git@github.com:me/dots.git"), "{}", r.out);
    assert_eq!(
        t.dw_json(&["remote", "list"])["remotes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
