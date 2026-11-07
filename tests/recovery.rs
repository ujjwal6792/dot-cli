//! Recovery (spec §11): `undo` of local actions and the offline archive, with
//! the real bundled chezmoi and git in isolated HOME/XDG directories.

mod common;

use std::fs;
use std::path::PathBuf;
use std::process::Stdio;

use common::TestEnv;

const PASS: &str = "correct horse battery";

struct Run {
    code: i32,
    out: String,
    err: String,
}

fn run(t: &TestEnv, extra: &[(&str, &str)], args: &[&str]) -> Run {
    let o = std::process::Command::new(env!("CARGO_BIN_EXE_dotwarden"))
        .args(args)
        .env_clear()
        .envs(&t.env)
        .envs(extra.iter().copied())
        .current_dir(&t.home)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    Run {
        code: o.status.code().unwrap(),
        out: String::from_utf8_lossy(&o.stdout).into_owned(),
        err: String::from_utf8_lossy(&o.stderr).into_owned(),
    }
}

fn dw(t: &TestEnv, args: &[&str]) -> Run {
    run(t, &[], args)
}

fn env() -> TestEnv {
    let mut t = TestEnv::new();
    let rt = t.root.path().canonicalize().unwrap().join("rt");
    fs::create_dir_all(&rt).unwrap();
    t.set("XDG_RUNTIME_DIR", rt);
    t.set("DOTWARDEN_TEST_WORK_FACTOR", "4");
    t
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

/// A machine with `.a` tracked and committed.
fn setup() -> TestEnv {
    let t = env();
    t.dw_ok(&["init", "--name", "testbox"]);
    write(&t, ".a", "one\n");
    t.dw_ok(&["track", ".a", "--apply"]);
    t.dw_ok(&["commit", "-m", "base"]);
    t
}

/// `path:state` of every change.
fn changes(t: &TestEnv) -> Vec<String> {
    t.dw_json(&["status"])["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| format!("{}:{}", c["path"], c["state"]).replace('"', ""))
        .collect()
}

/// HEAD plus the staged tree: the whole local source state.
fn source_state(t: &TestEnv) -> String {
    format!(
        "{} {}",
        t.git(&repo(t), &["rev-parse", "HEAD"]),
        t.git(&repo(t), &["write-tree"])
    )
}

#[test]
fn undo_walks_back_stage_track_commit_and_discard_in_order() {
    let t = setup();
    let start = source_state(&t);

    // stage an edit, track a new file, commit both: three actions on the stack.
    write(&t, ".a", "two\n");
    let local = changes(&t);
    t.dw_ok(&["stage", ".a"]);
    let staged = source_state(&t);
    write(&t, ".b", "bee\n");
    t.dw_ok(&["track", ".b", "--apply"]);
    t.dw_ok(&["commit", "-m", "both"]);
    let committed = source_state(&t);
    assert_ne!(committed, staged);

    // dry run reports and changes nothing.
    let dry = t.dw_ok(&["undo", "--dry-run"]);
    assert!(dry.contains("would undo: commit"), "{dry}");
    assert_eq!(source_state(&t), committed);

    // commit: HEAD moves back, the changes stay staged.
    t.dw_ok(&["undo"]);
    assert_eq!(
        t.git(&repo(&t), &["rev-parse", "HEAD"]),
        start.split(' ').next().unwrap()
    );
    assert!(
        changes(&t).iter().any(|c| c.starts_with(".b:staged")),
        "{:?}",
        changes(&t)
    );

    // track: .b is untracked again, its HOME file untouched.
    t.dw_ok(&["undo"]);
    assert_eq!(source_state(&t), staged);
    assert_eq!(read(&t, ".b"), "bee\n");

    // stage: back to a plain local edit.
    t.dw_ok(&["undo"]);
    assert_eq!(
        changes(&t)
            .iter()
            .filter(|c| c.starts_with(".a:"))
            .collect::<Vec<_>>(),
        local
            .iter()
            .filter(|c| c.starts_with(".a:"))
            .collect::<Vec<_>>()
    );
    assert_eq!(source_state(&t), start);
    assert_eq!(read(&t, ".a"), "two\n");

    // discard: the edit comes back from its snapshot.
    t.dw_ok(&["discard", ".a", "--yes"]);
    assert_eq!(read(&t, ".a"), "one\n");
    t.dw_ok(&["undo"]);
    assert_eq!(read(&t, ".a"), "two\n");
}

#[test]
fn undo_with_an_empty_journal_says_so() {
    let t = env();
    t.dw_ok(&["init", "--name", "testbox"]);
    let none = dw(&t, &["undo"]);
    assert_eq!(none.code, 1, "{}{}", none.out, none.err);
    assert!(none.err.contains("nothing to undo"), "{}", none.err);
}

#[test]
fn undo_refuses_a_pushed_commit_and_a_repository_that_changed_underneath() {
    let t = setup();
    write(&t, ".a", "two\n");
    t.dw_ok(&["stage", ".a"]);
    t.dw_ok(&["commit", "-m", "second"]);
    let head = t.git(&repo(&t), &["rev-parse", "HEAD"]);

    // Once a remote branch contains the commit, undo must not rewrite it.
    t.git(
        &repo(&t),
        &["update-ref", "refs/remotes/origin/trunk", &head],
    );
    let r = dw(&t, &["undo"]);
    assert_eq!(r.code, 1, "{}{}", r.out, r.err);
    assert!(r.err.contains("shared history"), "{}", r.err);
    assert_eq!(t.git(&repo(&t), &["rev-parse", "HEAD"]), head);

    // A change the journal does not know about makes the entry stale (exit 7).
    let t = setup();
    write(&t, ".a", "two\n");
    t.dw_ok(&["stage", ".a"]);
    t.dw_ok(&["commit", "-m", "second"]);
    t.dw_ok(&["revert", "HEAD"]);
    let r = dw(&t, &["undo"]);
    assert_eq!(r.code, 7, "{}{}", r.out, r.err);
}

#[test]
fn archive_restores_a_fresh_machine_offline_and_stays_sealed() {
    let a = setup();
    a.dw_ok(&["branch", "create", "feature"]);
    let remote = a.root.path().join("remote.git");
    a.git(
        &a.home,
        &["init", "--bare", "--quiet", remote.to_str().unwrap()],
    );
    a.dw_ok(&["remote", "add", "origin", remote.to_str().unwrap(), "--yes"]);
    let pass = [("DOTWARDEN_TEST_PASSPHRASE", PASS)];
    let new_pass = [("DOTWARDEN_TEST_NEW_PASSPHRASE", PASS)];
    assert_eq!(run(&a, &new_pass, &["vault", "init"]).code, 0);
    a.dw_ok(&["commit", "-m", "vault"]);
    let head = a.git(&repo(&a), &["rev-parse", "HEAD"]);

    let file = a.root.path().join("backup.archive");
    let f = file.to_str().unwrap();
    let wrong = run(
        &a,
        &[("DOTWARDEN_TEST_PASSPHRASE", "not the passphrase")],
        &["archive", "export", f],
    );
    assert_ne!(wrong.code, 0);
    assert!(!file.exists());
    let ok = run(&a, &pass, &["archive", "export", f]);
    assert_eq!(ok.code, 0, "{}{}", ok.out, ok.err);
    let sealed = fs::read(&file).unwrap();
    assert!(
        !String::from_utf8_lossy(&sealed).contains("one\n")
            && !sealed.windows(6).any(|w| w == b"bundle"),
        "the archive must not hold readable content"
    );
    assert_eq!(
        run(&a, &pass, &["archive", "export", f]).code,
        1,
        "an existing archive is not replaced without --force"
    );

    // A fresh computer with no network access to the old remote.
    fs::remove_dir_all(&remote).unwrap();
    let b = env();
    let bad = run(
        &b,
        &[("DOTWARDEN_TEST_PASSPHRASE", "not the passphrase")],
        &["archive", "import", f],
    );
    assert_ne!(bad.code, 0);
    assert!(!repo(&b).exists(), "a failed import leaves nothing behind");

    let imported = run(&b, &pass, &["archive", "import", f]);
    assert_eq!(imported.code, 0, "{}{}", imported.out, imported.err);
    assert_eq!(b.git(&repo(&b), &["rev-parse", "HEAD"]), head);
    let branches = b.git(&repo(&b), &["branch", "--format=%(refname:short)"]);
    assert!(branches.lines().any(|l| l == "feature"), "{branches}");
    assert_eq!(
        b.git(&repo(&b), &["config", "remote.origin.url"]),
        remote.to_str().unwrap()
    );
    let status = b.dw_json(&["status"]);
    assert_eq!(status["machine"]["name"], "testbox", "{status}");

    b.dw_ok(&["apply", "--all"]);
    assert_eq!(read(&b, ".a"), "one\n");
    // The vault came along: the same passphrase unlocks it.
    assert_eq!(run(&b, &pass, &["vault", "unlock"]).code, 0);
    // Importing again is refused: the machine already has a repository.
    assert_ne!(run(&b, &pass, &["archive", "import", f]).code, 0);
}
