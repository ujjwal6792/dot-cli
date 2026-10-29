mod common;

use common::TestEnv;
use std::fs;

fn history() -> TestEnv {
    let t = TestEnv::new();
    t.dw_ok(&["init", "--name", "alpha"]);
    fs::write(t.home.join(".a"), "one\n").unwrap();
    t.dw_ok(&["track", ".a", "--apply"]);
    t.dw_ok(&["commit", "-m", "first"]);
    t
}

#[test]
fn log_path_count_author_agent_and_show() {
    let mut t = history();
    t.set("DOTWARDEN_AGENT", "tester");
    fs::write(t.home.join(".a"), "two\n").unwrap();
    t.dw_ok(&["stage", ".a"]);
    t.dw_ok(&["commit", "-m", "second"]);
    let log = t.dw_json(&["log", ".a", "-n", "1"]);
    assert_eq!(log.as_array().unwrap().len(), 1);
    assert_eq!(log[0]["subject"], "second");
    assert_eq!(log[0]["author"], "Test User");
    assert_eq!(log[0]["agent"], "tester");
    let show = t.dw_json(&["show", "HEAD", ".a"]);
    assert!(show["diff"].as_str().unwrap().contains("+two"));
    assert!(!t.dw_ok(&["show", "HEAD", ".missing"]).contains("+two"));
    let absolute = t.home.join(".a");
    assert!(
        t.dw_ok(&["show", "HEAD", absolute.to_str().unwrap()])
            .contains("+two")
    );
    t.git(&t.dirs().repo(), &["rm", "home/dot_a"]);
    t.git(&t.dirs().repo(), &["commit", "-m", "deleted"]);
    assert_eq!(
        t.dw_json(&["log", ".a", "-n", "1"])[0]["subject"],
        "deleted"
    );
}

#[test]
fn revert_round_trip_is_local_and_permitted_for_agents() {
    let mut t = history();
    fs::write(t.home.join(".a"), "two\n").unwrap();
    t.dw_ok(&["stage", ".a"]);
    t.dw_ok(&["commit", "-m", "second"]);
    t.set("DOTWARDEN_AGENT", "tester");
    t.dw_ok(&["revert", "HEAD"]);
    assert_eq!(
        fs::read_to_string(t.dirs().repo().join("home/dot_a")).unwrap(),
        "one\n"
    );
    assert_eq!(fs::read_to_string(t.home.join(".a")).unwrap(), "two\n");
    assert_eq!(t.dw_json(&["log", "-n", "1"])[0]["agent"], "tester");
    t.dw_ok(&["revert", "HEAD"]);
    assert_eq!(
        fs::read_to_string(t.dirs().repo().join("home/dot_a")).unwrap(),
        "two\n"
    );
}

#[test]
fn history_masks_secret_in_diff_and_commit_metadata() {
    const SECRET: &str = "Canary-History-727188ab";
    let mut t = history();
    let rt = t.root.path().join("runtime");
    fs::create_dir(&rt).unwrap();
    t.set("XDG_RUNTIME_DIR", rt);
    t.set("DOTWARDEN_TEST_WORK_FACTOR", "4");
    t.set("DOTWARDEN_TEST_NEW_PASSPHRASE", "test passphrase");
    t.set("DOTWARDEN_TEST_PASSPHRASE", "test passphrase");
    t.dw_ok(&["vault", "init"]);
    t.dw_ok(&["vault", "unlock"]);
    t.set("DOTWARDEN_TEST_SECRET_VALUE", SECRET);
    t.dw_ok(&["values", "add-secret", "API"]);
    t.dw_ok(&["commit", "-m", "vault"]);
    // Simulate legacy stored plaintext and a secret in a commit message.
    fs::write(
        t.dirs().repo().join("home/dot_a"),
        format!("token={SECRET}\n"),
    )
    .unwrap();
    t.git(&t.dirs().repo(), &["add", "home/dot_a"]);
    t.git(&t.dirs().repo(), &["commit", "-m", SECRET]);
    for args in [
        vec!["log"],
        vec!["log", "--json"],
        vec!["show", "HEAD"],
        vec!["show", "HEAD", "--json"],
    ] {
        let output = t.dw_ok(&args);
        assert!(!output.contains(SECRET), "secret leaked in {args:?}");
        assert!(output.contains("secret:API"), "mask missing: {output}");
    }
}
