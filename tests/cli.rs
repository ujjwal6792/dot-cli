mod common;

use common::TestEnv;
use std::fs;

#[test]
fn init_creates_the_spec_layout_and_registers_the_machine() {
    let t = TestEnv::new();
    let out = t.dw_ok(&["init", "--name", "testbox"]);
    assert!(out.contains("Registered machine testbox"), "{out}");

    let repo = t.dirs().repo();
    for rel in [
        ".chezmoiroot",
        "dotwarden.toml",
        "hide",
        "home/.chezmoiignore",
        "home/.chezmoidata/dotwarden.toml",
        "home/.chezmoitemplates/dotwarden/targets",
        "home/.chezmoitemplates/dotwarden/variant",
    ] {
        assert!(repo.join(rel).is_file(), "missing {rel}");
    }
    assert_eq!(
        fs::read_to_string(repo.join(".chezmoiroot")).unwrap(),
        "home\n"
    );
    let toml = fs::read_to_string(repo.join("dotwarden.toml")).unwrap();
    assert!(
        toml.contains("format = 1") && toml.contains("default_branch = \"main\""),
        "{toml}"
    );

    // Registry entry (committed) and local machine.toml.
    let det = dotwarden::machine::detect().unwrap();
    let registry = dotwarden::repo::read_registry(&repo).unwrap();
    assert_eq!(registry.len(), 1);
    assert_eq!(registry[0].name, "testbox");
    assert_eq!(registry[0].platform, det.platform);
    assert_eq!(registry[0].distribution, det.distribution);
    assert_eq!(registry[0].families, det.families);
    let local = fs::read_to_string(t.dirs().machine_toml()).unwrap();
    assert!(local.contains("name = \"testbox\""), "{local}");
    assert!(
        fs::read_to_string(t.dirs().chezmoi_config())
            .unwrap()
            .contains("testbox")
    );

    // History: layout commit then registration; the tree is clean.
    let log = t.git(&repo, &["log", "--format=%s"]);
    assert_eq!(
        log,
        "Register machine testbox\nInitialise Dotwarden repository"
    );
    assert_eq!(t.git(&repo, &["status", "--porcelain"]), "");
    assert_eq!(t.git(&repo, &["symbolic-ref", "--short", "HEAD"]), "main");
    assert_eq!(
        t.git(&repo, &["log", "-1", "--format=%an <%ae>"]),
        "Test User <test@example.com>"
    );
}

#[test]
fn init_refuses_to_overwrite_and_rejects_bad_names() {
    let t = TestEnv::new();
    let bad = t.dw(&["init", "--name", "Bad Name"]);
    assert_eq!(bad.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&bad.stderr).contains("invalid machine name"));
    assert!(!t.dirs().repo().exists(), "no half-created repository");

    t.dw_ok(&["init", "--name", "one"]);
    let again = t.dw(&["init", "--name", "two"]);
    assert_eq!(again.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&again.stderr).contains("already exists"));
}

#[test]
fn init_without_a_name_on_a_non_terminal_fails_clearly() {
    let t = TestEnv::new();
    let o = t.dw(&["init"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("--name"));
}

#[test]
fn init_requires_a_git_identity() {
    let t = TestEnv::new();
    fs::remove_file(t.home.join(".gitconfig")).unwrap();
    let o = t.dw(&["init", "--name", "x"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("git identity"));
    assert!(!t.dirs().repo().exists());
}

#[test]
fn xdg_directories_are_respected() {
    let mut t = TestEnv::new();
    let x = t.root.path().canonicalize().unwrap().join("xdg");
    t.set("XDG_DATA_HOME", x.join("data"))
        .set("XDG_STATE_HOME", x.join("state"))
        .set("XDG_CACHE_HOME", x.join("cache"));
    t.dw_ok(&["init", "--name", "xdg"]);
    assert!(x.join("data/dotwarden/repo/.chezmoiroot").is_file());
    assert!(x.join("data/dotwarden/chezmoi.toml").is_file());
    assert!(x.join("state/dotwarden/machine.toml").is_file());
    assert!(x.join("cache/dotwarden/chezmoi").is_dir());
    assert!(
        !t.home.join(".local").exists(),
        "nothing under the default XDG paths"
    );
}

#[test]
fn clone_registers_a_second_machine_and_refuses_duplicates() {
    let a = TestEnv::new();
    a.dw_ok(&["init", "--name", "alpha"]);
    let url = format!("file://{}", a.dirs().repo().display());

    let b = TestEnv::new();
    let out = b.dw_ok(&["clone", &url, "--name", "beta"]);
    assert!(out.contains("Registered machine beta"), "{out}");
    let repo = b.dirs().repo();
    let names: Vec<_> = dotwarden::repo::read_registry(&repo)
        .unwrap()
        .into_iter()
        .map(|m| m.name)
        .collect();
    assert_eq!(names, ["alpha", "beta"]);
    assert_eq!(t_head(&b, &repo), "Register machine beta");
    assert_eq!(b.git(&repo, &["status", "--porcelain"]), "");
    assert_eq!(b.git(&repo, &["remote"]), "origin");

    // The same name from a third computer is refused, and nothing is left behind.
    let c = TestEnv::new();
    let o = c.dw(&["clone", &url, "--name", "alpha"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("already registered"));
    assert!(!c.dirs().repo().exists());

    // ...unless it is the same machine being reinstalled.
    let d = TestEnv::new();
    let out = d.dw_ok(&["clone", &url, "--name", "alpha", "--adopt"]);
    assert!(out.contains("Adopted machine alpha"), "{out}");
    assert_eq!(
        d.git(&d.dirs().repo(), &["log", "-1", "--format=%s"]),
        "Register machine alpha"
    );
}

fn t_head(t: &TestEnv, repo: &std::path::Path) -> String {
    t.git(repo, &["log", "-1", "--format=%s"])
}

#[test]
fn clone_rejects_things_that_are_not_dotwarden_repositories() {
    let t = TestEnv::new();
    let other = t.root.path().join("plain");
    fs::create_dir(&other).unwrap();
    t.git(&other, &["init", "--quiet"]);
    fs::write(other.join("README"), "x").unwrap();
    t.git(&other, &["add", "."]);
    t.git(&other, &["commit", "--quiet", "-m", "x"]);
    let o = t.dw(&["clone", other.to_str().unwrap(), "--name", "m"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("not a Dotwarden repository"));
    assert!(!t.dirs().repo().exists());
}

#[test]
fn status_json_before_and_after_init() {
    let t = TestEnv::new();
    let s = t.dw_json(&["status"]);
    assert_eq!(s["initialised"], false);
    assert!(s["machine"].is_null());

    t.dw_ok(&["init", "--name", "me"]);
    let s = t.dw_json(&["status"]);
    assert_eq!(s["initialised"], true);
    assert_eq!(s["machine"]["name"], "me");
    assert_eq!(s["machine"]["registered"]["name"], "me");
    assert_eq!(s["repo"]["branch"], "main");
    assert_eq!(s["repo"]["changed_paths"], 0);
    assert!(s["repo"]["upstream"].is_null());
    assert_eq!(s["pending"], 0);
    let human = t.dw_ok(&["status"]);
    assert!(human.contains("Machine:    me"), "{human}");
}

#[test]
fn status_reports_ahead_behind_against_the_upstream() {
    let a = TestEnv::new();
    a.dw_ok(&["init", "--name", "alpha"]);
    let url = format!("file://{}", a.dirs().repo().display());
    let b = TestEnv::new();
    b.dw_ok(&["clone", &url, "--name", "beta"]);
    let s = b.dw_json(&["status"]);
    assert_eq!(s["repo"]["upstream"], "origin/main");
    assert_eq!(s["repo"]["ahead"], 1);
    assert_eq!(s["repo"]["behind"], 0);
}

#[test]
fn doctor_passes_before_and_after_init_and_lists_tool_versions() {
    let t = TestEnv::new();
    let before = t.dw_json(&["doctor"]);
    assert_eq!(before["ok"], true, "{before}");

    t.dw_ok(&["init", "--name", "doc"]);
    let after = t.dw_json(&["doctor"]);
    assert_eq!(after["ok"], true, "{after}");
    let text = t.dw_ok(&["doctor"]);
    for needle in [
        "v2.73.3",
        "2.56.0",
        "0.20.1",
        "doc is in the machine registry",
        "Test User <test@example.com>",
    ] {
        assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
    }
}

#[test]
fn doctor_fails_when_the_identity_is_missing() {
    let t = TestEnv::new();
    fs::remove_file(t.home.join(".gitconfig")).unwrap();
    let o = t.dw(&["doctor"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stdout).contains("[FAIL] git identity"));
}

#[test]
fn the_dw_alias_is_the_same_program() {
    let t = TestEnv::new();
    let o = t.run(env!("CARGO_BIN_EXE_dw"), &["--help"]);
    assert!(o.status.success());
    let help = String::from_utf8_lossy(&o.stdout);
    assert!(help.contains("Usage: dw"), "{help}");
    for sub in ["init", "clone", "status", "doctor", "tui"] {
        assert!(help.contains(sub), "{sub}");
    }
    t.dw_ok(&["--version"]);
}

// ---- spec §9: dry-run, lock, typed errors -------------------------------

use common::snapshot;
use std::time::{Duration, Instant};

#[test]
fn init_dry_run_reports_the_plan_and_changes_nothing() {
    let t = TestEnv::new();
    let before = snapshot(&t.root.path().canonicalize().unwrap());
    let plan = t.dw_json(&["init", "--dry-run", "--name", "drybox"]);
    assert_eq!(plan["dry_run"], true);
    assert_eq!(plan["command"], "init");
    assert_eq!(plan["machine"]["name"], "drybox");
    let files: Vec<_> = plan["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap())
        .collect();
    for f in [
        ".chezmoiroot",
        "hide",
        "home/.chezmoiignore",
        "home/.chezmoitemplates/dotwarden/select",
    ] {
        assert!(files.contains(&f), "{f} missing from {files:?}");
    }
    let human = t.dw_ok(&["init", "--dry-run", "--name", "drybox"]);
    assert!(
        human.contains("Dry run") && human.contains("register machine drybox"),
        "{human}"
    );
    // A name is not needed (and not prompted for) in a dry run.
    let unnamed = t.dw_json(&["init", "--dry-run"]);
    assert_eq!(unnamed["machine"], serde_json::Value::Null);
    assert_eq!(
        snapshot(&t.root.path().canonicalize().unwrap()),
        before,
        "dry run wrote something"
    );
    assert!(!t.dirs().repo().exists() && !t.dirs().state_dir().exists());

    // It fails the same way the real command would.
    t.dw_ok(&["init", "--name", "real"]);
    let again = t.dw(&["init", "--dry-run", "--name", "x"]);
    assert_eq!(again.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&again.stderr).contains("already exists"));
    let bad = t.dw(&["init", "--dry-run", "--name", "Bad"]);
    assert_eq!(bad.status.code(), Some(2));
}

#[test]
fn clone_dry_run_reports_the_plan_without_touching_the_network_or_disk() {
    let t = TestEnv::new();
    let before = snapshot(&t.root.path().canonicalize().unwrap());
    let plan = t.dw_json(&[
        "clone",
        "https://example.invalid/never-contacted.git",
        "--dry-run",
        "--name",
        "c",
    ]);
    assert_eq!(plan["dry_run"], true);
    assert_eq!(plan["command"], "clone");
    assert_eq!(plan["url"], "https://example.invalid/never-contacted.git");
    assert_eq!(plan["machine"]["name"], "c");
    let human = t.dw_ok(&["clone", "x", "--dry-run", "--name", "c", "--adopt"]);
    assert!(
        human.contains("Dry run") && human.contains("adopting"),
        "{human}"
    );
    assert_eq!(snapshot(&t.root.path().canonicalize().unwrap()), before);
}

#[test]
fn mutations_wait_for_the_lock_and_read_only_commands_do_not_take_it() {
    let mut t = TestEnv::new();
    t.set(dotwarden::lock::WAIT_ENV, "400");
    let lock_file = t.dirs().lock_file();
    let held = dotwarden::lock::acquire(&lock_file, Duration::from_secs(1)).unwrap();

    // Read-only commands and dry runs go straight through.
    let start = Instant::now();
    t.dw_ok(&["status"]);
    t.dw_ok(&["doctor"]);
    t.dw_ok(&["init", "--dry-run", "--name", "x"]);
    assert!(start.elapsed() < Duration::from_secs(5));

    // A mutation waits, then gives up with a plain error.
    let start = Instant::now();
    let o = t.dw(&["init", "--name", "blocked"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        start.elapsed() >= Duration::from_millis(350),
        "did not wait: {:?}",
        start.elapsed()
    );
    assert!(String::from_utf8_lossy(&o.stderr).contains("another dotwarden command"));
    assert!(!t.dirs().repo().exists());

    // With a longer wait it succeeds once the holder lets go.
    t.set(dotwarden::lock::WAIT_ENV, "8000");
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(600));
        drop(held);
    });
    let start = Instant::now();
    t.dw_ok(&["init", "--name", "waited"]);
    assert!(start.elapsed() >= Duration::from_millis(500));
    releaser.join().unwrap();
    assert!(t.dirs().repo().join(".chezmoiroot").is_file());
}

#[test]
fn errors_carry_exit_codes_and_json_error_objects() {
    let t = TestEnv::new();
    // Usage error (bad machine name): code 2.
    let o = t.dw(&["init", "--name", "Bad Name", "--json"]);
    assert_eq!(o.status.code(), Some(2));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).expect("JSON on stdout");
    assert_eq!(v["error"]["code"], 2);
    assert_eq!(v["error"]["kind"], "usage");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("invalid machine name")
    );
    assert!(
        o.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );

    // clap's own usage errors are JSON too, with code 2.
    let o = t.dw(&["--json", "no-such-command"]);
    assert_eq!(o.status.code(), Some(2));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["error"]["kind"], "usage");
    // Without --json they stay on stderr.
    let o = t.dw(&["no-such-command"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(o.stdout.is_empty());

    // A plain failure: code 1, kind "error".
    t.dw_ok(&["init", "--name", "one"]);
    let o = t.dw(&["init", "--name", "two", "--json"]);
    assert_eq!(o.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(
        (v["error"]["code"].as_i64(), v["error"]["kind"].as_str()),
        (Some(1), Some("error"))
    );

    // Help and version still succeed with --json.
    assert!(t.dw(&["--json", "--help"]).status.success());
}

#[test]
fn status_and_doctor_write_nothing() {
    let t = TestEnv::new();
    t.dw_ok(&["init", "--name", "ro"]);
    let repo = t.dirs().repo();
    let root = t.root.path().canonicalize().unwrap();

    // Healthy: nothing is created, rewritten or touched, not even the git index.
    // (Rewrite a tracked file with identical content so a refresh would show.)
    std::thread::sleep(Duration::from_millis(1100));
    fs::write(repo.join(".chezmoiroot"), "home\n").unwrap();
    let before = snapshot(&root);
    t.dw_ok(&["status"]);
    t.dw_ok(&["status", "--json"]);
    t.dw_ok(&["doctor"]);
    assert_eq!(snapshot(&root), before, "a read-only command wrote to disk");

    // A missing generated config is reported, never silently regenerated.
    fs::remove_file(t.dirs().chezmoi_config()).unwrap();
    let before = snapshot(&root);
    t.dw_ok(&["status"]);
    let o = t.dw(&["doctor"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stdout).contains("[FAIL] chezmoi config"));
    assert!(!t.dirs().chezmoi_config().exists());
    assert_eq!(snapshot(&root), before);
}
