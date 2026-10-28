//! Remotes, fetch, pull, apply and push with real git and chezmoi, local bare
//! remotes and a fake visibility probe (no network).

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
    fs::write(t.home.join(rel), content).unwrap();
}

fn read(t: &TestEnv, rel: &str) -> String {
    fs::read_to_string(t.home.join(rel)).unwrap()
}

fn repo(t: &TestEnv) -> PathBuf {
    t.dirs().repo()
}

fn bare(t: &TestEnv, name: &str) -> PathBuf {
    let p = t.root.path().join(name);
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

/// A probe script that logs the URL it was asked about and exits with `code`.
fn fake_probe(t: &mut TestEnv, code: i32) -> PathBuf {
    let log = t.root.path().join("probe.log");
    let script = t.root.path().join("probe.sh");
    fs::write(
        &script,
        format!("#!/bin/sh\necho \"$1\" >> {}\nexit {code}\n", log.display()),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    t.set("DOTWARDEN_VISIBILITY_PROBE", script);
    log
}

/// Machine A with `.a` and `.b` tracked, committed and pushed to a new bare remote.
fn machine_a() -> (TestEnv, PathBuf) {
    let a = TestEnv::new();
    a.dw_ok(&["init", "--name", "alpha"]);
    write(&a, ".a", "one\n");
    write(&a, ".b", "bee\n");
    a.dw_ok(&["track", ".a", ".b", "--apply"]);
    a.dw_ok(&["commit", "-m", "base"]);
    let remote = bare(&a, "remote.git");
    a.dw_ok(&["remote", "add", "origin", remote.to_str().unwrap(), "--yes"]);
    a.dw_ok(&["push", "--yes"]);
    (a, remote)
}

/// Machine B cloned from `remote`, registered, pushed (so histories stay linear)
/// and applied once.
fn machine_b(remote: &std::path::Path) -> TestEnv {
    let b = TestEnv::new();
    b.dw_ok(&["clone", remote.to_str().unwrap(), "--name", "beta"]);
    b.dw_ok(&["push", "--yes"]);
    b.dw_ok(&["apply", "--all"]);
    b
}

fn incoming(t: &TestEnv) -> Vec<String> {
    states(t, "incoming")
}

fn states(t: &TestEnv, state: &str) -> Vec<String> {
    t.dw_json(&["status"])["changes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["state"] == state)
        .map(|c| c["path"].as_str().unwrap().to_string())
        .collect()
}

fn edit_commit_push(t: &TestEnv, rel: &str, content: &str) {
    t.dw_ok(&["pull"]);
    write(t, rel, content);
    t.dw_ok(&["stage", rel]);
    t.dw_ok(&["commit", "-m", "edit"]);
    t.dw_ok(&["push", "--yes"]);
}

fn remotes(t: &TestEnv) -> Vec<String> {
    t.dw_json(&["remote", "list"])["remotes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn remotes_are_added_listed_and_removed() {
    let t = TestEnv::new();
    t.dw_ok(&["init", "--name", "alpha"]);
    let b = bare(&t, "r.git");
    t.dw_ok(&["remote", "add", "origin", b.to_str().unwrap(), "--yes"]);
    let v = t.dw_json(&["remote", "list"]);
    assert_eq!(v["remotes"][0]["visibility"], "local");
    let again = dw(&t, &["remote", "add", "origin", "/elsewhere", "--yes"]);
    assert_eq!(again.code, 1, "{}", again.err);
    t.dw_ok(&["remote", "remove", "origin", "--yes"]);
    assert!(remotes(&t).is_empty());
}

#[test]
fn a_public_remote_is_refused_and_the_probe_gets_the_https_url() {
    let mut t = TestEnv::new();
    t.dw_ok(&["init", "--name", "alpha"]);
    let log = fake_probe(&mut t, 0);
    let r = dw(
        &t,
        &[
            "remote",
            "add",
            "origin",
            "git@github.com:me/dots.git",
            "--yes",
        ],
    );
    assert_eq!(r.code, 6, "{}{}", r.out, r.err);
    assert!(r.err.contains("public"), "{}", r.err);
    assert!(remotes(&t).is_empty());
    assert_eq!(
        fs::read_to_string(log).unwrap().trim(),
        "https://github.com/me/dots.git"
    );
}

#[test]
fn a_private_known_host_is_added_and_an_unreachable_one_only_warns() {
    let mut t = TestEnv::new();
    t.dw_ok(&["init", "--name", "alpha"]);
    fake_probe(&mut t, 1);
    t.dw_ok(&["remote", "add", "one", "git@gitlab.com:me/a.git", "--yes"]);
    fake_probe(&mut t, 2);
    let out = t.dw_ok(&["remote", "add", "two", "git@gitlab.com:me/b.git", "--yes"]);
    assert!(out.contains("warning") && out.contains("skipped"), "{out}");
    assert_eq!(remotes(&t), ["one", "two"]);
}

#[test]
fn an_unverifiable_host_needs_a_human_confirmation_stored_per_url() {
    let t = TestEnv::new();
    t.dw_ok(&["init", "--name", "alpha"]);
    let url = "git@git.example.org:me/dots.git";
    let r = dw(&t, &["remote", "add", "origin", url, "--yes"]);
    assert_eq!(r.code, 3, "{}{}", r.out, r.err);
    assert!(remotes(&t).is_empty());
    assert_eq!(
        t.dw_json(&["pending", "list"])["items"][0]["kind"],
        "remote_add"
    );

    dotwarden::remote::confirm_private(&t.store(), url).unwrap();
    t.dw_ok(&["remote", "add", "origin", url, "--yes"]);
    assert_eq!(remotes(&t), ["origin"]);
    // Another repository on the same host is not covered, and the machine name survived.
    let other = dw(
        &t,
        &[
            "remote",
            "add",
            "two",
            "git@git.example.org:me/other.git",
            "--yes",
        ],
    );
    assert_eq!(other.code, 3);
    assert_eq!(
        t.store().local_machine_name().unwrap().as_deref(),
        Some("alpha")
    );
}

#[test]
fn an_agent_cannot_add_remove_or_push_but_a_human_can_approve_the_push() {
    let (a, remote) = machine_a();
    let head = a.git(&repo(&a), &["rev-parse", "HEAD"]);
    write(&a, ".a", "two\n");
    a.dw_ok(&["stage", ".a"]);
    a.dw_ok(&["commit", "-m", "second"]);

    assert_eq!(
        run_with(&a, true, &["remote", "add", "x", "/tmp/x", "--yes"]).code,
        3
    );
    assert_eq!(
        run_with(&a, true, &["remote", "remove", "origin", "--yes"]).code,
        3
    );
    assert_eq!(run_with(&a, true, &["push", "--yes"]).code, 3);
    assert_eq!(remotes(&a), ["origin"]);
    assert_eq!(
        a.git(&remote, &["rev-parse", "main"]),
        head,
        "nothing was pushed"
    );

    let items = a.dw_json(&["pending", "list"]);
    let id = items["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["kind"] == "push")
        .unwrap()["id"]
        .to_string();
    let r = run_with(&a, true, &["pending", "apply", &id, "--yes"]);
    assert_eq!(r.code, 3, "an agent cannot approve its own push");
    a.dw_ok(&["pending", "apply", &id, "--yes"]);
    assert_eq!(
        a.git(&remote, &["rev-parse", "main"]),
        a.git(&repo(&a), &["rev-parse", "HEAD"])
    );
}

#[test]
fn push_refuses_a_public_remote_before_anything_is_sent() {
    let (mut a, remote) = machine_a();
    let spare = bare(&a, "spare.git");
    // A GitHub-looking remote that really lands in `spare`, so a push that got
    // through would be visible there. The probe says it is public.
    let repo_dir = repo(&a);
    a.git(
        &repo_dir,
        &["remote", "add", "pub", "git@github.com:me/dots.git"],
    );
    a.git(
        &repo_dir,
        &[
            "config",
            &format!("url.{}.insteadOf", spare.display()),
            "git@github.com:me/dots.git",
        ],
    );
    fake_probe(&mut a, 0);
    write(&a, ".a", "two\n");
    a.dw_ok(&["stage", ".a"]);
    a.dw_ok(&["commit", "-m", "second"]);
    let before = a.git(&remote, &["rev-parse", "main"]);

    let r = dw(&a, &["push", "--all", "--yes"]);
    assert_eq!(r.code, 6, "{}{}", r.out, r.err);
    assert_eq!(
        a.git(&remote, &["rev-parse", "main"]),
        before,
        "the private remote was spared too"
    );
    assert!(
        std::process::Command::new("ls")
            .arg(spare.join("refs/heads/main"))
            .output()
            .unwrap()
            .status
            .code()
            != Some(0),
        "nothing reached the public remote"
    );
}

#[test]
fn push_with_an_unreachable_probe_warns_and_goes_ahead() {
    let (mut a, remote) = machine_a();
    let repo_dir = repo(&a);
    a.git(
        &repo_dir,
        &["remote", "add", "gh", "git@github.com:me/dots.git"],
    );
    a.git(
        &repo_dir,
        &[
            "config",
            &format!("url.{}.insteadOf", remote.display()),
            "git@github.com:me/dots.git",
        ],
    );
    fake_probe(&mut a, 2);
    write(&a, ".a", "two\n");
    a.dw_ok(&["stage", ".a"]);
    a.dw_ok(&["commit", "-m", "second"]);
    let out = a.dw_ok(&["push", "gh", "--yes"]);
    assert!(out.contains("warning"), "{out}");
    assert_eq!(
        a.git(&remote, &["rev-parse", "main"]),
        a.git(&repo_dir, &["rev-parse", "HEAD"])
    );
}

#[test]
fn fetch_reports_behind_but_changes_neither_home_nor_the_repository() {
    let (a, remote) = machine_a();
    let b = machine_b(&remote);
    edit_commit_push(&a, ".a", "from a\n");

    let head = b.git(&repo(&b), &["rev-parse", "HEAD"]);
    b.dw_ok(&["fetch"]);
    let s = b.dw_json(&["status"]);
    assert_eq!(s["repo"]["behind"], 1);
    assert_eq!(s["repo"]["ahead"], 0);
    assert!(
        incoming(&b).is_empty(),
        "fetch alone brings nothing incoming"
    );
    assert_eq!(b.git(&repo(&b), &["rev-parse", "HEAD"]), head);
    assert_eq!(read(&b, ".a"), "one\n");
}

#[test]
fn a_pull_makes_incoming_and_apply_writes_only_those_files_after_a_snapshot() {
    let (a, remote) = machine_a();
    let b = machine_b(&remote);
    assert!(
        incoming(&b).is_empty(),
        "nothing incoming right after the first apply"
    );
    edit_commit_push(&a, ".a", "from a\n");
    // B has its own unrelated local change.
    write(&b, ".b", "mine\n");

    let p = b.dw_ok(&["pull"]);
    assert!(p.contains("1 incoming"), "{p}");
    assert_eq!(incoming(&b), [".a"]);
    assert_eq!(read(&b, ".a"), "one\n", "pull never writes HOME");

    let dry = b.dw_ok(&["apply", "--all", "--dry-run"]);
    assert!(dry.contains("would apply ~/.a"), "{dry}");
    assert_eq!(read(&b, ".a"), "one\n");

    b.dw_ok(&["apply", "--all"]);
    assert_eq!(read(&b, ".a"), "from a\n");
    assert_eq!(read(&b, ".b"), "mine\n", "only incoming files are written");
    assert!(incoming(&b).is_empty());
    assert_eq!(states(&b, "local"), [".b"]);
    let snaps = b.dw_json(&["snapshot", "list", ".a"]);
    assert_eq!(snaps[0]["reason"], "apply");

    let none = dw(&b, &["apply", ".a"]);
    assert_eq!(none.code, 1);
    assert!(none.err.contains("nothing incoming"), "{}", none.err);
}

#[test]
fn a_file_changed_on_both_machines_is_a_conflict_that_apply_refuses() {
    let (a, remote) = machine_a();
    let b = machine_b(&remote);
    write(&b, ".a", "b edit\n");
    edit_commit_push(&a, ".a", "a edit\n");
    edit_commit_push(&a, ".b", "a bee\n");

    b.dw_ok(&["pull"]);
    assert_eq!(states(&b, "conflict"), [".a"]);
    assert_eq!(incoming(&b), [".b"]);

    let r = dw(&b, &["apply", ".a"]);
    assert_eq!(r.code, 5, "{}{}", r.out, r.err);
    assert_eq!(read(&b, ".a"), "b edit\n");
    let agent = run_with(&b, true, &["apply", ".a"]);
    assert_eq!(agent.code, 5);

    let all = b.dw_ok(&["apply", "--all"]);
    assert!(all.contains("skipped ~/.a: conflict"), "{all}");
    assert_eq!(read(&b, ".a"), "b edit\n");
    assert_eq!(read(&b, ".b"), "a bee\n");
}

#[test]
fn pull_refuses_diverged_history_and_leaves_it_alone() {
    let (a, remote) = machine_a();
    let b = machine_b(&remote);
    write(&b, ".b", "b side\n");
    b.dw_ok(&["stage", ".b"]);
    b.dw_ok(&["commit", "-m", "b local"]);
    edit_commit_push(&a, ".a", "a side\n");
    let head = b.git(&repo(&b), &["rev-parse", "HEAD"]);

    let r = dw(&b, &["pull"]);
    assert_eq!(r.code, 1, "{}{}", r.out, r.err);
    assert!(
        r.err.contains("diverged") && r.err.contains("lazygit"),
        "{}",
        r.err
    );
    assert_eq!(b.git(&repo(&b), &["rev-parse", "HEAD"]), head);
    let s = b.dw_json(&["status"]);
    assert_eq!(
        (s["repo"]["ahead"].as_u64(), s["repo"]["behind"].as_u64()),
        (Some(1), Some(1))
    );
}
