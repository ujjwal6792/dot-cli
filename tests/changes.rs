//! The change model: classification, stage, unstage, commit and diff against the
//! real bundled chezmoi and git in isolated temporary HOME/XDG directories.

mod common;

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::PathBuf;

use common::TestEnv;

struct Run {
    code: i32,
    out: String,
    err: String,
}

fn dw(t: &TestEnv, args: &[&str]) -> Run {
    let o = t.dw(args);
    Run {
        code: o.status.code().unwrap(),
        out: String::from_utf8_lossy(&o.stdout).into_owned(),
        err: String::from_utf8_lossy(&o.stderr).into_owned(),
    }
}

fn write(t: &TestEnv, rel: &str, content: &str) {
    let p = t.home.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

fn repo(t: &TestEnv) -> PathBuf {
    t.dirs().repo()
}

/// A machine with `.a` and `.b` tracked and committed.
fn setup() -> TestEnv {
    let t = TestEnv::new();
    t.dw_ok(&["init", "--name", "testbox"]);
    write(&t, ".a", "one\n");
    write(&t, ".b", "bee\n");
    t.dw_ok(&["track", ".a", ".b", "--apply"]);
    t.dw_ok(&["commit", "-m", "base"]);
    t
}

/// `path:state:kind` for every change.
fn changes(t: &TestEnv) -> Vec<String> {
    let v = t.dw_json(&["status"]);
    v["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| format!("{}:{}:{}", c["path"], c["state"], c["kind"]).replace('"', ""))
        .collect()
}

fn log_body(t: &TestEnv) -> String {
    t.git(&repo(t), &["log", "-1", "--format=%B"])
}

#[test]
fn a_committed_untouched_machine_has_no_changes() {
    let t = setup();
    assert!(changes(&t).is_empty());
}

#[test]
fn edits_deletions_mode_and_symlink_changes_are_local() {
    let t = setup();
    write(&t, ".c", "c\n");
    symlink(".a", t.home.join(".l")).unwrap();
    t.dw_ok(&["track", ".c", ".l", "--apply"]);
    t.dw_ok(&["commit", "-m", "more"]);
    write(&t, ".a", "two\n");
    fs::remove_file(t.home.join(".b")).unwrap();
    fs::set_permissions(t.home.join(".c"), fs::Permissions::from_mode(0o755)).unwrap();
    fs::remove_file(t.home.join(".l")).unwrap();
    symlink(".b", t.home.join(".l")).unwrap();
    assert_eq!(
        changes(&t),
        [
            ".a:local:modified",
            ".b:local:deleted",
            ".c:local:modified",
            ".l:local:modified"
        ]
    );
}

#[test]
fn unhidden_untracked_files_in_a_tracked_folder_are_new() {
    let t = setup();
    write(&t, ".cfg/x", "x\n");
    t.dw_ok(&["track", ".cfg", "--apply"]);
    t.dw_ok(&["commit", "-m", "folder"]);
    write(&t, ".cfg/y", "y\n");
    write(&t, ".cfg/.cache/z", "hidden\n");
    write(&t, "outside", "not in a tracked folder\n");
    assert_eq!(changes(&t), [".cfg/y:new:new"]);
}

#[test]
fn staging_and_committing_leaves_no_change_and_the_repo_equals_its_index() {
    let t = setup();
    write(&t, ".a", "two\n");
    t.dw_ok(&["stage", ".a"]);
    assert_eq!(changes(&t), [".a:staged:modified"]);
    // No unstaged column anywhere: worktree == index (ADR 0003).
    for line in t
        .git(
            &repo(&t),
            &["status", "--porcelain", "--untracked-files=all"],
        )
        .lines()
    {
        assert_eq!(&line[1..2], " ", "{line}");
    }
    t.dw_ok(&["commit", "-m", "a"]);
    assert!(changes(&t).is_empty());
    // And a later edit is a plain local change again, not a conflict.
    write(&t, ".a", "three\n");
    assert_eq!(changes(&t), [".a:local:modified"]);
}

#[test]
fn an_edit_after_staging_is_both_staged_and_local() {
    let t = setup();
    write(&t, ".a", "two\n");
    t.dw_ok(&["stage", ".a"]);
    write(&t, ".a", "three\n");
    assert_eq!(changes(&t), [".a:local:modified", ".a:staged:modified"]);
}

#[test]
fn unstaging_returns_the_file_to_local_and_the_repo_to_head() {
    let t = setup();
    write(&t, ".a", "two\n");
    fs::set_permissions(t.home.join(".b"), fs::Permissions::from_mode(0o755)).unwrap();
    t.dw_ok(&["stage", "--all"]);
    t.dw_ok(&["unstage", ".a", ".b"]);
    assert_eq!(changes(&t), [".a:local:modified", ".b:local:modified"]);
    assert_eq!(t.git(&repo(&t), &["status", "--porcelain"]), "");
    assert_eq!(fs::read_to_string(t.home.join(".a")).unwrap(), "two\n");
}

#[test]
fn staging_a_new_file_tracks_it_and_unstaging_forgets_it_again() {
    let t = setup();
    write(&t, ".cfg/x", "x\n");
    t.dw_ok(&["track", ".cfg", "--apply"]);
    t.dw_ok(&["commit", "-m", "folder"]);
    write(&t, ".cfg/y", "y\n");
    t.dw_ok(&["stage", ".cfg/y"]);
    assert_eq!(changes(&t), [".cfg/y:staged:new"]);
    t.dw_ok(&["unstage", ".cfg/y"]);
    assert_eq!(changes(&t), [".cfg/y:new:new"]);
    assert_eq!(t.git(&repo(&t), &["status", "--porcelain", "-uall"]), "");
    t.dw_ok(&["stage", ".cfg"]);
    t.dw_ok(&["commit", "-m", "y"]);
    assert!(changes(&t).is_empty());
    let ls = t.dw_json(&["ls", ".cfg"]);
    assert!(ls.to_string().contains("tracked"), "{ls}");
}

#[test]
fn files_assigned_to_other_machines_are_never_changes() {
    let t = setup();
    dotwarden::assign::set_assignment(&repo(&t), ".b", &["platform:linux".into()], &[]).unwrap();
    t.git(&repo(&t), &["add", "-A"]);
    t.dw_ok(&["commit", "-m", "assign"]);
    write(
        &t,
        ".b",
        "edited on a machine that is not meant to have it\n",
    );
    fs::remove_file(t.home.join(".b")).unwrap();
    assert!(changes(&t).is_empty());
}

/// Another machine's commit arrives in the repository (as after a pull).
fn incoming(t: &TestEnv, source: &str, content: &str) {
    fs::write(repo(t).join(source), content).unwrap();
    t.git(&repo(t), &["commit", "-qam", "from elsewhere"]);
}

#[test]
fn incoming_and_conflict_come_from_the_last_written_state() {
    let t = setup();
    incoming(&t, "home/dot_a", "remote\n");
    assert_eq!(changes(&t), [".a:incoming:modified"]);
    write(&t, ".a", "mine\n");
    assert_eq!(changes(&t), [".a:conflict:modified"]);
    // A different file edited only here stays Local beside it.
    write(&t, ".b", "mine too\n");
    assert_eq!(changes(&t), [".b:local:modified", ".a:conflict:modified"]);
}

#[test]
fn incoming_and_conflicts_cannot_be_staged_but_do_not_block_stage_all() {
    let t = setup();
    incoming(&t, "home/dot_a", "remote\n");
    let r = dw(&t, &["stage", ".a"]);
    assert_eq!(r.code, 1, "{}", r.err);
    write(&t, ".a", "mine\n");
    let r = dw(&t, &["stage", ".a"]);
    assert_eq!(r.code, 5, "{}", r.err);
    write(&t, ".b", "mine too\n");
    let r = dw(&t, &["stage", "--all", "--json"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(
        r.out.contains("conflict") && r.out.contains("~/.b"),
        "{}",
        r.out
    );
    assert_eq!(changes(&t), [".b:staged:modified", ".a:conflict:modified"]);
}

#[test]
fn stage_and_unstage_dry_runs_change_nothing_and_templates_are_refused() {
    let t = setup();
    write(&t, ".a", "two\n");
    let r = dw(&t, &["stage", ".a", "--dry-run"]);
    assert!(r.out.contains("would stage ~/.a"), "{}", r.out);
    assert_eq!(changes(&t), [".a:local:modified"]);
    assert_eq!(t.git(&repo(&t), &["status", "--porcelain"]), "");

    fs::write(repo(&t).join("home/dot_t.tmpl"), "t {{ 1 }}\n").unwrap();
    t.git(&repo(&t), &["add", "-A"]);
    t.dw_ok(&["commit", "-m", "template"]);
    t.store()
        .chezmoi(&["apply", "--force", "--include=files"])
        .unwrap();
    write(&t, ".t", "edited\n");
    let r = dw(&t, &["stage", ".t"]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("template"), "{}", r.err);
    assert_eq!(t.git(&repo(&t), &["status", "--porcelain"]), "");
}

#[test]
fn commit_takes_repeated_messages_and_an_agent_trailer() {
    let mut t = setup();
    write(&t, ".a", "two\n");
    t.dw_ok(&["stage", ".a"]);
    t.set("DOTWARDEN_AGENT", "claude");
    t.dw_ok(&["commit", "-m", "subject", "-m", "second paragraph"]);
    let body = log_body(&t);
    assert!(
        body.starts_with("subject\n\nsecond paragraph\n\nAgent: claude"),
        "{body}"
    );
}

#[test]
fn commit_refuses_nothing_staged_and_empty_or_missing_messages() {
    let t = setup();
    let r = dw(&t, &["commit", "-m", "x"]);
    assert_eq!(r.code, 1, "{}", r.err);
    assert!(r.err.contains("nothing is staged"), "{}", r.err);
    write(&t, ".a", "two\n");
    t.dw_ok(&["stage", ".a"]);
    assert_eq!(dw(&t, &["commit", "-m", "  "]).code, 2);
    // No terminal here, so no editor either.
    assert_eq!(dw(&t, &["commit"]).code, 2);
    assert_eq!(changes(&t), [".a:staged:modified"]);
}

#[test]
fn amend_rewrites_a_local_commit_but_not_one_already_on_a_remote_branch() {
    let t = setup();
    write(&t, ".a", "two\n");
    t.dw_ok(&["stage", ".a"]);
    t.dw_ok(&["commit", "-m", "first"]);
    let before = t.git(&repo(&t), &["rev-list", "--count", "HEAD"]);
    write(&t, ".b", "bee two\n");
    t.dw_ok(&["stage", ".b"]);
    t.dw_ok(&["commit", "--amend", "-m", "first, better"]);
    assert_eq!(log_body(&t), "first, better");
    assert_eq!(t.git(&repo(&t), &["rev-list", "--count", "HEAD"]), before);

    let remote = t.root.path().join("remote.git");
    t.git(
        t.root.path(),
        &["init", "-q", "--bare", remote.to_str().unwrap()],
    );
    t.git(
        &repo(&t),
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    t.git(&repo(&t), &["push", "-q", "-u", "origin", "HEAD"]);
    let r = dw(&t, &["commit", "--amend", "-m", "rewrite"]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("already on"), "{}", r.err);
    assert_eq!(log_body(&t), "first, better");
}

#[test]
fn diff_gives_plain_unified_diff_for_local_new_and_staged_changes() {
    let t = setup();
    write(&t, ".a", "two\n");
    write(&t, ".b", "bee\nsecond\n");
    t.dw_ok(&["stage", ".b"]);
    let out = t.dw_ok(&["diff", "--no-pager"]);
    assert!(out.contains("--- a/.a\n+++ b/.a\n"), "{out}");
    assert!(out.contains("-one\n+two\n"), "{out}");
    assert!(out.contains("+second"), "{out}");
    assert!(!out.contains('\u{1b}'), "no colour codes");
    let only_b = t.dw_ok(&["diff", ".b"]);
    assert!(
        only_b.contains("+second") && !only_b.contains("+two"),
        "{only_b}"
    );
    let v = t.dw_json(&["diff"]);
    let first = &v[0];
    assert_eq!(
        (first["path"].as_str(), first["state"].as_str()),
        (Some("~/.a"), Some("local"))
    );
}
