mod common;
use common::TestEnv;
use std::fs;

fn setup() -> TestEnv {
    let t = TestEnv::new();
    t.dw_ok(&["init", "--name", "alpha"]);
    for p in [".a", ".b"] {
        fs::write(t.home.join(p), "one\n").unwrap();
    }
    t.dw_ok(&["track", ".a", ".b", "--apply"]);
    t.dw_ok(&["commit", "-m", "base"]);
    t
}
fn git(t: &TestEnv, args: &[&str]) -> String {
    t.git(&t.dirs().repo(), args)
}
fn read(t: &TestEnv, p: &str) -> String {
    fs::read_to_string(t.home.join(p)).unwrap()
}
fn edit(t: &TestEnv, p: &str, value: &str) {
    fs::write(t.home.join(p), value).unwrap();
}
fn commit(t: &TestEnv, p: &str, value: &str) {
    edit(t, p, value);
    t.dw_ok(&["stage", p]);
    t.dw_ok(&["commit", "-m", value]);
}
fn fails(t: &TestEnv, args: &[&str], code: i32) {
    let out = t.dw(args);
    assert_eq!(
        out.status.code(),
        Some(code),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn branch_switch_preserves_disjoint_index_and_home_and_warns() {
    let t = setup();
    t.dw_ok(&["branch", "create", "other"]);
    t.dw_ok(&["branch", "switch", "other"]);
    commit(&t, ".a", "other\n");
    t.dw_ok(&["branch", "switch", "main"]);
    // HOME is still the other branch's content; preserve this unstaged drift.
    edit(&t, ".b", "staged\n");
    t.dw_ok(&["stage", ".b"]);
    let index = git(&t, &["show", ":home/dot_b"]);
    let dry = t.dw_json(&["branch", "switch", "other", "--dry-run"]);
    assert!(
        dry["source_paths"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p == "home/dot_a")
    );
    assert_eq!(git(&t, &["branch", "--show-current"]).trim(), "main");
    t.dw_ok(&["branch", "switch", "other"]);
    assert_eq!(git(&t, &["show", ":home/dot_b"]), index);
    assert_eq!(read(&t, ".a"), "other\n");
    assert!(t.dw_ok(&["status"]).contains("non-default"));
}

#[test]
fn branch_switch_refuses_overlapping_stage_without_resetting_it() {
    let t = setup();
    t.dw_ok(&["branch", "create", "other"]);
    t.dw_ok(&["branch", "switch", "other"]);
    commit(&t, ".a", "other\n");
    t.dw_ok(&["branch", "switch", "main"]);
    edit(&t, ".a", "staged\n");
    fs::write(t.dirs().repo().join("home/dot_a"), "staged\n").unwrap();
    git(&t, &["add", "home/dot_a"]);
    let tree = git(&t, &["write-tree"]);
    fails(&t, &["branch", "switch", "other"], 1);
    assert_eq!(git(&t, &["write-tree"]), tree);
    assert_eq!(read(&t, ".a"), "staged\n");
}

#[test]
fn branch_delete_requires_force_and_agent_human_approval() {
    let mut t = setup();
    fails(&t, &["branch", "delete", "main", "--force", "--yes"], 1);
    t.dw_ok(&["branch", "create", "other"]);
    t.dw_ok(&["branch", "switch", "other"]);
    commit(&t, ".a", "other\n");
    t.dw_ok(&["branch", "switch", "main"]);
    fails(&t, &["branch", "delete", "other", "--yes"], 1);
    t.set("DOTWARDEN_AGENT", "tester");
    fails(&t, &["branch", "delete", "other", "--force", "--yes"], 3);
    let pending = t.dw_json(&["pending", "list"]);
    let id = pending["items"][0]["id"].as_u64().unwrap().to_string();
    t.env.remove(std::ffi::OsStr::new("DOTWARDEN_AGENT"));
    t.dw_ok(&["pending", "apply", &id, "--yes"]);
    assert_eq!(t.dw_json(&["branch", "list"]).as_array().unwrap().len(), 1);
}

#[test]
fn stash_preserves_partial_staging_and_untracked_home() {
    let t = setup();
    edit(&t, ".a", "staged\n");
    t.dw_ok(&["stage", ".a"]);
    edit(&t, ".a", "unstaged\n");
    edit(&t, ".untracked", "leave me\n");
    let tree = git(&t, &["write-tree"]);
    let dry = t.dw_json(&["stash", "push", "--dry-run"]);
    assert_eq!(dry["stash"]["paths"][0], ".a");
    assert!(t.dw_json(&["stash", "list"]).as_array().unwrap().is_empty());
    t.dw_ok(&["stash", "push", "--yes"]);
    assert_eq!(read(&t, ".a"), "one\n");
    assert!(git(&t, &["status", "--porcelain"]).is_empty());
    t.dw_ok(&["stash", "pop", "--yes"]);
    assert_eq!(read(&t, ".a"), "unstaged\n");
    assert_eq!(git(&t, &["write-tree"]), tree);
    assert_eq!(read(&t, ".untracked"), "leave me\n");
}

#[test]
fn stash_round_trips_staged_tracking_additions_and_removals() {
    let t = setup();
    edit(&t, ".new", "newly tracked\n");
    t.dw_ok(&["track", ".new", "--apply"]);
    t.dw_ok(&["untrack", ".b", "--apply"]);
    let tree = git(&t, &["write-tree"]);
    t.dw_ok(&["stash", "push", "--yes"]);
    assert!(!t.home.join(".new").exists());
    assert!(t.dirs().repo().join("home/dot_b").exists());
    t.dw_ok(&["stash", "pop", "--yes"]);
    assert_eq!(git(&t, &["write-tree"]), tree);
    assert_eq!(read(&t, ".new"), "newly tracked\n");
    assert_eq!(read(&t, ".b"), "one\n");
}

#[test]
fn stash_pop_conflicts_retain_payload_and_drop_is_explicit() {
    let t = setup();
    edit(&t, ".a", "saved\n");
    t.dw_ok(&["stash", "push", "--yes"]);
    edit(&t, ".a", "collision\n");
    fails(&t, &["stash", "pop", "--yes"], 5);
    assert_eq!(read(&t, ".a"), "collision\n");
    assert_eq!(t.dw_json(&["stash", "list"]).as_array().unwrap().len(), 1);
    t.dw_ok(&["stash", "drop", "--dry-run"]);
    t.dw_ok(&["stash", "drop", "--yes"]);
    assert!(t.dw_json(&["stash", "list"]).as_array().unwrap().is_empty());
}

#[test]
fn historical_apply_keeps_head_index_and_forward_incoming() {
    let t = setup();
    let old = git(&t, &["rev-parse", "HEAD"]);
    commit(&t, ".a", "two\n");
    // Model a forward change on a different file without touching HOME.
    fs::write(t.dirs().repo().join("home/dot_b"), "forward\n").unwrap();
    git(&t, &["add", "home/dot_b"]);
    git(&t, &["commit", "-m", "forward"]);
    let head = git(&t, &["rev-parse", "HEAD"]);
    let tree = git(&t, &["write-tree"]);
    t.dw_ok(&["restore", "--at", old.trim(), ".a", "--dry-run"]);
    assert!(dotwarden::restores::list(&t.store()).unwrap().is_empty());
    t.dw_ok(&["restore", "--at", old.trim(), ".a"]);
    let before_status = common::snapshot(t.root.path());
    t.dw_ok(&["status"]);
    assert_eq!(
        common::snapshot(t.root.path()),
        before_status,
        "candidate status must remain read-only"
    );
    assert_eq!(read(&t, ".a"), "two\n");
    assert!(t.dw_ok(&["diff", ".a"]).contains("+one"));
    t.dw_ok(&["apply", ".a"]);
    assert_eq!(read(&t, ".a"), "one\n");
    assert_eq!(read(&t, ".b"), "one\n");
    assert_eq!(git(&t, &["rev-parse", "HEAD"]), head);
    assert_eq!(git(&t, &["write-tree"]), tree);
    let status = t.dw_json(&["status"]);
    assert!(
        status["changes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["path"] == ".b" && c["state"] == "incoming")
    );
    assert!(!dotwarden::snapshot::list(&t.store()).unwrap().is_empty());
}

#[test]
fn historical_candidates_reject_local_collision_and_staleness() {
    let t = setup();
    let old = git(&t, &["rev-parse", "HEAD"]);
    commit(&t, ".a", "two\n");
    edit(&t, ".a", "local\n");
    let r = t.dw_json(&["restore", "--at", old.trim(), ".a"]);
    assert_eq!(r["candidates"][0]["conflict"], true);
    fails(&t, &["apply", ".a"], 5);
    t.dw_ok(&["restore", "--clear", "--all"]);
    edit(&t, ".a", "two\n");
    t.dw_ok(&["restore", "--at", old.trim(), ".a"]);
    edit(&t, ".a", "changed after planning\n");
    fails(&t, &["apply", ".a"], 7);
    assert_eq!(read(&t, ".a"), "changed after planning\n");
}

#[test]
fn stash_keeps_reference_templates_and_variant_sources_exact() {
    let t = setup();
    t.dw_ok(&["values", "set-variable", "WORD", "one", "--apply"]);
    t.dw_ok(&["values", "wire", ".a", "WORD", "--apply"]);
    t.dw_ok(&["commit", "-m", "reference"]);
    t.dw_ok(&[
        "variant",
        "add",
        ".b",
        "--target",
        "machine:alpha",
        "--apply",
    ]);
    t.dw_ok(&["commit", "-m", "variant"]);
    edit(&t, ".a", "one plus\n");
    t.dw_ok(&["stage", ".a"]);
    edit(&t, ".a", "one unstaged\n");
    edit(&t, ".b", "variant staged\n");
    t.dw_ok(&["stage", ".b"]);
    edit(&t, ".b", "variant unstaged\n");
    let tree = git(&t, &["write-tree"]);
    t.dw_ok(&["stash", "push", "--yes"]);
    assert_eq!(read(&t, ".a"), "one\n");
    assert_eq!(read(&t, ".b"), "one\n");
    t.dw_ok(&["stash", "pop", "--yes"]);
    assert_eq!(git(&t, &["write-tree"]), tree);
    assert_eq!(read(&t, ".a"), "one unstaged\n");
    assert_eq!(read(&t, ".b"), "variant unstaged\n");
}

#[test]
fn historical_restore_renders_historical_variables_variants_and_assignments() {
    let t = setup();
    t.dw_ok(&["values", "set-variable", "WORD", "one", "--apply"]);
    t.dw_ok(&["values", "wire", ".a", "WORD", "--apply"]);
    t.dw_ok(&[
        "variant",
        "add",
        ".b",
        "--target",
        "machine:alpha",
        "--apply",
    ]);
    t.dw_ok(&["commit", "-m", "old variable and variant"]);
    let old = git(&t, &["rev-parse", "HEAD"]);
    edit(&t, ".a", "two\n");
    t.dw_ok(&["stage", ".a"]);
    edit(&t, ".b", "two\n");
    t.dw_ok(&["stage", ".b"]);
    t.dw_ok(&["commit", "-m", "new values"]);
    t.dw_ok(&["restore", "--at", old.trim(), "--all"]);
    t.dw_ok(&["apply", "--all"]);
    assert_eq!(read(&t, ".a"), "one\n");
    assert_eq!(read(&t, ".b"), "one\n");
    t.dw_ok(&["assign", ".a", "--exclude", "machine:alpha", "--apply"]);
    t.dw_ok(&["commit", "-m", "excluded"]);
    fails(&t, &["restore", "--at", old.trim(), ".a"], 1);
    assert_eq!(read(&t, ".a"), "one\n");
}

fn vault(t: &mut TestEnv) {
    let rt = t.root.path().join("runtime");
    fs::create_dir_all(&rt).unwrap();
    t.set("XDG_RUNTIME_DIR", rt);
    t.set("DOTWARDEN_TEST_WORK_FACTOR", "4");
    t.set("DOTWARDEN_TEST_NEW_PASSPHRASE", "test passphrase");
    t.set("DOTWARDEN_TEST_PASSPHRASE", "test passphrase");
    t.set("DOTWARDEN_BIN", env!("CARGO_BIN_EXE_dotwarden"));
    t.dw_ok(&["vault", "init"]);
    t.dw_ok(&["vault", "unlock"]);
}
fn has_bytes(root: &std::path::Path, secret: &[u8]) -> bool {
    if root.is_dir() {
        fs::read_dir(root)
            .unwrap()
            .flatten()
            .any(|e| has_bytes(&e.path(), secret))
    } else {
        fs::read(root).is_ok_and(|bytes| bytes.windows(secret.len()).any(|s| s == secret))
    }
}

#[test]
fn secret_stash_and_restore_use_encrypted_payloads_and_snapshots() {
    const SECRET: &str = "Canary-T11B-d324cf109d";
    let mut t = setup();
    vault(&mut t);
    t.dw_ok(&["commit", "-m", "vault"]);
    edit(&t, ".a", &format!("key={SECRET}\nname=old\n"));
    // Replace the existing stored file by wiring a newly registered secret.
    t.set("DOTWARDEN_TEST_SECRET_VALUE", SECRET);
    t.dw_ok(&["values", "add-secret", "API"]);
    fs::remove_file(t.dirs().repo().join("home/dot_a")).unwrap();
    fs::write(
        t.dirs().repo().join("home/dot_a.tmpl"),
        format!("key={}\nname=old\n", dotwarden::values::reference("API")),
    )
    .unwrap();
    git(&t, &["add", "-A"]);
    git(&t, &["commit", "-m", "secret reference"]);
    t.store().chezmoi(&["apply", "--force"]).unwrap();
    let old = git(&t, &["rev-parse", "HEAD"]);
    edit(&t, ".a", &format!("key={SECRET}\nname=staged\n"));
    t.dw_ok(&["stage", ".a"]);
    edit(&t, ".a", &format!("key={SECRET}\nname=unstaged\n"));
    t.dw_ok(&["stash", "push", "--yes"]);
    assert!(!has_bytes(
        &t.dirs().state_dir().join("stashes"),
        SECRET.as_bytes()
    ));
    assert!(!git(&t, &["log", "--all", "-p"]).contains(SECRET));
    assert!(!has_bytes(&t.dirs().snapshots_dir(), SECRET.as_bytes()));
    t.dw_ok(&["vault", "lock"]);
    fails(&t, &["stash", "pop", "--yes"], 4);
    fails(&t, &["restore", "--at", old.trim(), ".a"], 4);
    t.dw_ok(&["vault", "unlock"]);
    t.dw_ok(&["stash", "pop", "--yes"]);
    assert!(read(&t, ".a").contains("name=unstaged"));
    t.dw_ok(&["stage", ".a"]);
    t.dw_ok(&["commit", "-m", "new name"]);
    t.dw_ok(&["restore", "--at", old.trim(), ".a"]);
    assert!(!t.dw_ok(&["diff", ".a"]).contains(SECRET));
    t.dw_ok(&["apply", ".a"]);
    assert!(read(&t, ".a").contains("name=old"));
    assert!(!has_bytes(&t.dirs().snapshots_dir(), SECRET.as_bytes()));
}
