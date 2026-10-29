//! Conflict resolution with real git and chezmoi: two machines over a local bare
//! remote, one file edited on both.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::TestEnv;

struct Run {
    code: i32,
    out: String,
    err: String,
}

impl Run {
    fn all(&self) -> String {
        format!("{}{}", self.out, self.err)
    }
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

fn states(t: &TestEnv, state: &str) -> Vec<String> {
    t.dw_json(&["status"])["changes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["state"] == state)
        .map(|c| c["path"].as_str().unwrap().to_string())
        .collect()
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

const BASE: &str = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\n";

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

/// alpha commits and pushes `content` for `.a`.
fn push_edit(a: &TestEnv, content: &str) {
    a.dw_ok(&["pull"]);
    write(a, ".a", content);
    a.dw_ok(&["stage", ".a"]);
    a.dw_ok(&["commit", "-m", "alpha edit"]);
    a.dw_ok(&["push", "--yes"]);
}

/// beta edits `.a` locally, alpha pushes its edit, beta pulls: a conflict.
fn conflicted(local: &str, incoming: &str) -> (TestEnv, TestEnv) {
    let (a, b) = pair();
    write(&b, ".a", local);
    push_edit(&a, incoming);
    b.dw_ok(&["pull"]);
    assert_eq!(states(&b, "conflict"), [".a"]);
    (a, b)
}

const OVERLAP_LOCAL: &str = "l1\nBETA\nl3\nl4\nl5\nl6\nl7\nl8\n";
const OVERLAP_INCOMING: &str = "l1\nALPHA\nl3\nl4\nl5\nl6\nl7\nl8\n";

fn sync_back(b: &TestEnv, a: &TestEnv) {
    b.dw_ok(&["commit", "-m", "resolved"]);
    b.dw_ok(&["push", "--yes"]);
    a.dw_ok(&["pull"]);
    a.dw_ok(&["apply", "--all"]);
}

#[test]
fn take_local_stages_the_local_file_and_clears_the_conflict() {
    let (a, b) = conflicted(OVERLAP_LOCAL, OVERLAP_INCOMING);
    let r = dw(&b, &["resolve", "notes", "--take", "local"]);
    assert_eq!(
        r.code,
        1,
        "a file that is not a conflict is refused: {}",
        r.all()
    );
    assert_eq!(dw(&b, &["resolve", ".a"]).code, 1, "a choice is required");

    let r = dw(&b, &["resolve", ".a", "--take", "local"]);
    assert_eq!(r.code, 0, "{}", r.all());
    assert_eq!(read(&b, ".a"), OVERLAP_LOCAL);
    assert!(states(&b, "conflict").is_empty());
    assert_eq!(states(&b, "staged"), [".a"]);
    sync_back(&b, &a);
    assert_eq!(read(&a, ".a"), OVERLAP_LOCAL);
    assert!(
        a.dw_json(&["status"])["changes"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn take_incoming_overwrites_home_after_a_snapshot_and_leaves_nothing_to_do() {
    let (_a, b) = conflicted(OVERLAP_LOCAL, OVERLAP_INCOMING);
    let r = dw(&b, &["resolve", ".a", "--take", "incoming"]);
    assert_eq!(r.code, 0, "{}", r.all());
    assert_eq!(read(&b, ".a"), OVERLAP_INCOMING);
    assert!(
        b.dw_json(&["status"])["changes"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let snaps = b.dw_ok(&["snapshot", "list"]);
    assert!(
        snaps.contains("resolve") && snaps.contains("~/.a"),
        "{snaps}"
    );
    // The snapshot holds the local version.
    let id = snaps.split_whitespace().next().unwrap();
    b.dw_ok(&["snapshot", "restore", id, ".a"]);
    assert_eq!(read(&b, ".a"), OVERLAP_LOCAL);
}

#[test]
fn an_agent_taking_the_incoming_side_goes_to_pending_but_may_stage_local() {
    let (a, b) = conflicted(OVERLAP_LOCAL, OVERLAP_INCOMING);
    let r = run_with(&b, true, &["resolve", ".a", "--take", "incoming"]);
    assert_eq!(r.code, 3, "{}", r.all());
    assert_eq!(read(&b, ".a"), OVERLAP_LOCAL);
    assert_eq!(states(&b, "conflict"), [".a"]);
    let pending = b.dw_ok(&["pending", "list"]);
    assert!(pending.contains("Resolve the conflict"), "{pending}");

    // A human approves it.
    b.dw_ok(&["pending", "apply", "--all", "--yes"]);
    assert_eq!(read(&b, ".a"), OVERLAP_INCOMING);
    assert!(states(&b, "conflict").is_empty());
    assert!(
        b.dw_json(&["status"])["changes"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    // Taking local needs no approval.
    let (_a2, b2) = conflicted(OVERLAP_LOCAL, OVERLAP_INCOMING);
    let r = run_with(&b2, true, &["resolve", ".a", "--take", "local"]);
    assert_eq!(r.code, 0, "{}", r.all());
    assert_eq!(states(&b2, "staged"), [".a"]);
    drop(a);
}

#[test]
fn edits_that_do_not_overlap_merge_cleanly_and_are_staged() {
    let local = "L1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\n";
    let incoming = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nA8\n";
    let merged = "L1\nl2\nl3\nl4\nl5\nl6\nl7\nA8\n";
    let (a, b) = conflicted(local, incoming);
    let r = dw(&b, &["resolve", ".a", "--merge", "--dry-run"]);
    assert_eq!(r.code, 0, "{}", r.all());
    assert!(r.out.contains("clean"), "{}", r.out);
    assert_eq!(read(&b, ".a"), local, "a dry run changes nothing");

    let r = dw(&b, &["resolve", ".a", "--merge"]);
    assert_eq!(r.code, 0, "{}", r.all());
    assert_eq!(read(&b, ".a"), merged);
    assert!(states(&b, "conflict").is_empty());
    assert_eq!(states(&b, "staged"), [".a"]);
    sync_back(&b, &a);
    assert_eq!(read(&a, ".a"), merged);
}

fn files_under(dir: &Path, found: &mut Vec<PathBuf>) {
    for e in fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            files_under(&p, found);
        } else {
            found.push(p);
        }
    }
}

#[test]
fn overlapping_edits_exit_5_and_no_conflict_marker_reaches_the_live_file() {
    let (_a, b) = conflicted(OVERLAP_LOCAL, OVERLAP_INCOMING);
    for agent in [false, true] {
        let r = run_with(&b, agent, &["resolve", ".a", "--merge"]);
        assert_eq!(r.code, 5, "agent={agent}: {}", r.all());
        assert!(r.all().contains("1 overlapping hunk"), "{}", r.all());
    }
    assert_eq!(read(&b, ".a"), OVERLAP_LOCAL);
    assert_eq!(states(&b, "conflict"), [".a"]);
    assert!(
        !b.dw_ok(&["pending", "list"]).contains("resolve"),
        "an agent cannot queue a merge that needs a choice"
    );
    // Nothing with markers is left in HOME or in dotwarden's state.
    let mut all = Vec::new();
    files_under(&b.home, &mut all);
    for f in all.iter().filter(|f| !f.starts_with(repo(&b).join(".git"))) {
        let text = fs::read(f).unwrap_or_default();
        assert!(
            !String::from_utf8_lossy(&text).contains("<<<<<<<"),
            "marker in {}",
            f.display()
        );
    }
}

#[test]
fn a_variant_file_conflicts_per_the_variant_this_machine_receives() {
    let (a, b) = pair();
    // Both machines are darwin: they receive the same platform variant.
    a.dw_ok(&[
        "variant",
        "add",
        ".a",
        "--target",
        "platform:darwin",
        "--apply",
    ]);
    a.dw_ok(&["commit", "-m", "variant"]);
    a.dw_ok(&["push", "--yes"]);
    b.dw_ok(&["pull"]);
    b.dw_ok(&["apply", "--all"]);
    write(&b, ".a", "l1\nBETA\nl3\nl4\nl5\nl6\nl7\nl8\n");
    push_edit(&a, "l1\nALPHA\nl3\nl4\nl5\nl6\nl7\nl8\n");
    b.dw_ok(&["pull"]);
    assert_eq!(states(&b, "conflict"), [".a"]);
    let r = dw(&b, &["resolve", ".a", "--merge"]);
    assert_eq!(r.code, 5, "{}", r.all());
    let r = dw(&b, &["resolve", ".a", "--take", "incoming"]);
    assert_eq!(r.code, 0, "{}", r.all());
    assert_eq!(read(&b, ".a"), OVERLAP_INCOMING);
    assert!(
        b.dw_json(&["status"])["changes"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    // Non-overlapping edits merge into the variant file, not the default.
    let (a, b) = pair();
    a.dw_ok(&[
        "variant",
        "add",
        ".a",
        "--target",
        "platform:darwin",
        "--apply",
    ]);
    a.dw_ok(&["commit", "-m", "variant"]);
    a.dw_ok(&["push", "--yes"]);
    b.dw_ok(&["pull"]);
    b.dw_ok(&["apply", "--all"]);
    write(&b, ".a", "L1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\n");
    push_edit(&a, "l1\nl2\nl3\nl4\nl5\nl6\nl7\nA8\n");
    b.dw_ok(&["pull"]);
    b.dw_ok(&["resolve", ".a", "--merge"]);
    assert_eq!(read(&b, ".a"), "L1\nl2\nl3\nl4\nl5\nl6\nl7\nA8\n");
    let stored =
        fs::read_to_string(repo(&b).join("home/.dotwarden/variants/.a/platform-darwin")).unwrap();
    assert_eq!(stored, "L1\nl2\nl3\nl4\nl5\nl6\nl7\nA8\n");
    let default = fs::read_to_string(repo(&b).join("home/.dotwarden/variants/.a/default")).unwrap();
    assert_eq!(default, BASE);
}
