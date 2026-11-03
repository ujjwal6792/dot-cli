//! Hunk and line staging with real chezmoi and git in isolated HOME/XDG
//! directories: partial stage, partial unstage, refusals and masking.

mod common;

use std::fs;
use std::path::PathBuf;

use common::TestEnv;

fn write(t: &TestEnv, rel: &str, content: &str) {
    let p = t.home.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

fn repo(t: &TestEnv) -> PathBuf {
    t.dirs().repo()
}

fn source(t: &TestEnv) -> String {
    fs::read_to_string(repo(t).join("home/dot_a")).unwrap()
}

fn head(t: &TestEnv) -> String {
    t.git(&repo(t), &["show", "HEAD:home/dot_a"])
}

fn states(t: &TestEnv) -> Vec<String> {
    let v = t.dw_json(&["status"]);
    v["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| format!("{}:{}", c["path"], c["state"]).replace('"', ""))
        .collect()
}

const BASE: &str = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\nl11\nl12\n";
/// Lines 2 and 11 changed: two hunks (3 lines of context do not meet).
const EDIT: &str = "l1\nL2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\nL11\nl12\n";

/// `.a` committed as BASE, then edited to EDIT in HOME.
fn edited() -> TestEnv {
    let t = TestEnv::new();
    t.dw_ok(&["init", "--name", "testbox"]);
    write(&t, ".a", BASE);
    t.dw_ok(&["track", ".a", "--apply"]);
    t.dw_ok(&["commit", "-m", "base"]);
    write(&t, ".a", EDIT);
    t
}

#[test]
fn staging_one_hunk_leaves_the_other_local_and_home_untouched() {
    let t = edited();
    let v = t.dw_json(&["diff", ".a", "--hunks"]);
    assert_eq!(v["hunks"].as_array().unwrap().len(), 2);
    t.dw_ok(&["stage", ".a", "--hunk", "2"]);
    assert_eq!(source(&t), BASE.replace("l11", "L11"));
    assert_eq!(fs::read_to_string(t.home.join(".a")).unwrap(), EDIT);
    // Staged and still Local (the first hunk), not Conflict or Incoming.
    let s = states(&t);
    assert!(
        s.contains(&".a:staged".into()) && s.contains(&".a:local".into()),
        "{s:?}"
    );
    // The remaining hunk is the first one and stages the rest.
    let v = t.dw_json(&["diff", ".a", "--hunks"]);
    assert_eq!(v["hunks"].as_array().unwrap().len(), 1);
    t.dw_ok(&["stage", ".a", "--hunk", "1"]);
    assert_eq!(source(&t), EDIT);
    assert_eq!(states(&t), vec![".a:staged"]);
    t.dw_ok(&["commit", "-m", "both"]);
    assert!(states(&t).is_empty());
}

#[test]
fn staging_one_line_of_a_replacement_keeps_the_rest_of_the_hunk() {
    let t = edited();
    // Changed lines: 1 = -l2, 2 = +L2, 3 = -l11, 4 = +L11. Staging only the
    // addition of L2 keeps the old l2 as well.
    t.dw_ok(&["stage", ".a", "--line", "2"]);
    assert_eq!(source(&t), BASE.replace("l2\n", "l2\nL2\n"));
    assert_eq!(states(&t).len(), 2);
    // Numbers restart on every view: the removal of l11 is now line 2.
    t.dw_ok(&["stage", ".a", "--line", "2"]);
    assert!(!source(&t).contains("l11\n"));
    assert!(source(&t).contains("l10\nl12\n"));
}

#[test]
fn unstaging_a_hunk_puts_only_that_hunk_back() {
    let t = edited();
    t.dw_ok(&["stage", ".a"]);
    assert_eq!(source(&t), EDIT);
    let v = t.dw_json(&["diff", ".a", "--hunks", "--staged"]);
    assert_eq!(v["side"], "unstage");
    assert_eq!(v["hunks"].as_array().unwrap().len(), 2);
    t.dw_ok(&["unstage", ".a", "--hunk", "1"]);
    assert_eq!(source(&t), EDIT.replace("L2", "l2"));
    assert_eq!(head(&t), BASE.trim_end());
    let s = states(&t);
    assert!(
        s.contains(&".a:staged".into()) && s.contains(&".a:local".into()),
        "{s:?}"
    );
    // The index holds exactly the working tree: nothing is left unstaged in git.
    assert_eq!(
        t.git(&repo(&t), &["status", "--porcelain"]).lines().count(),
        1
    );
    t.dw_ok(&["unstage", ".a", "--line", "1", "--line", "2"]);
    assert_eq!(source(&t), BASE);
    assert_eq!(states(&t), vec![".a:local"]);
}

#[test]
fn bad_selections_and_dry_runs_change_nothing() {
    let t = edited();
    let r = t.dw(&["stage", ".a", "--hunk", "3"]);
    assert_eq!(r.status.code(), Some(2));
    let r = t.dw(&["stage", ".a", "--line", "9"]);
    assert_eq!(r.status.code(), Some(2));
    let r = t.dw(&["stage", ".a", ".b", "--hunk", "1"]);
    assert_eq!(r.status.code(), Some(2));
    let out = t.dw_ok(&["stage", ".a", "--hunk", "1", "--dry-run"]);
    assert!(out.contains("would stage 2 changed line(s)"), "{out}");
    assert_eq!(source(&t), BASE);
    assert_eq!(states(&t), vec![".a:local"]);
}

#[test]
fn new_files_are_read_only_and_staged_whole() {
    let t = edited();
    write(&t, ".d/x", "x\n");
    t.dw_ok(&["track", ".d", "--apply"]);
    t.dw_ok(&["commit", "-m", "dir"]);
    write(&t, ".d/n", "x\n");
    let v = t.dw_json(&["diff", ".d/n", "--hunks"]);
    assert!(v["readonly"].as_str().unwrap().contains("new"));
    let r = t.dw(&["stage", ".d/n", "--hunk", "1"]);
    assert_eq!(r.status.code(), Some(1));
}

#[test]
fn a_detected_secret_is_never_shown_and_cannot_be_staged_by_hunk() {
    let t = TestEnv::new();
    t.dw_ok(&["init", "--name", "testbox"]);
    write(&t, ".env", "host = h\n");
    t.dw_ok(&["track", ".env", "--apply"]);
    t.dw_ok(&["commit", "-m", "env"]);
    let secret = "Zq8x7Lm2Pq9wRt5vYb3nKd6HfJs1AaGc";
    write(&t, ".env", &format!("host = h\napi_token = {secret}\n"));
    let human = t.dw_ok(&["diff", ".env", "--hunks"]);
    let json = t.dw_ok(&["diff", ".env", "--hunks", "--json"]);
    assert!(
        human.contains("read-only") && human.contains("hidden"),
        "{human}"
    );
    assert!(!human.contains(secret) && !json.contains(secret));
    let r = t.dw(&["stage", ".env", "--hunk", "1"]);
    assert_eq!(r.status.code(), Some(1));
    assert!(!String::from_utf8_lossy(&r.stderr).contains(secret));
    let tracked = fs::read_to_string(repo(&t).join("home/dot_env")).unwrap();
    assert_eq!(tracked, "host = h\n");
}

#[test]
fn a_file_with_a_vault_reference_shows_the_marker_not_the_value() {
    let mut t = TestEnv::new();
    let rt = t.root.path().canonicalize().unwrap().join("rt");
    fs::create_dir_all(&rt).unwrap();
    t.set("XDG_RUNTIME_DIR", rt);
    t.set("DOTWARDEN_SESSION_BACKEND", "file");
    t.set("DOTWARDEN_TEST_WORK_FACTOR", "4");
    t.dw_ok(&["init", "--name", "mac"]);
    let o = std::process::Command::new(env!("CARGO_BIN_EXE_dotwarden"))
        .args(["vault", "init", "--json"])
        .env_clear()
        .envs(&t.env)
        .env("DOTWARDEN_TEST_NEW_PASSPHRASE", "correct horse battery")
        .current_dir(&t.home)
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    t.dw_ok(&["commit", "-m", "vault"]);
    let secret = "Zq8x7Lm2Pq9wRt5vYb3nKd6HfJs1AaGc";
    write(&t, ".env", &format!("host = h\napi_token = {secret}\n"));
    t.dw_ok(&["track", ".env", "--apply"]);
    t.dw_ok(&["commit", "-m", "env"]);
    write(&t, ".env", &format!("host = i\napi_token = {secret}\n"));
    let human = t.dw_ok(&["diff", ".env", "--hunks"]);
    let json = t.dw_ok(&["diff", ".env", "--hunks", "--json"]);
    assert!(
        human.contains("«secret:") && human.contains("host = i"),
        "{human}"
    );
    assert!(!human.contains(secret) && !json.contains(secret));
    let r = t.dw(&["stage", ".env", "--hunk", "1"]);
    assert_eq!(r.status.code(), Some(1));
    assert!(!String::from_utf8_lossy(&r.stderr).contains(secret));
}
