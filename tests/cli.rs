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
    assert_eq!(bad.status.code(), Some(1));
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
    assert_eq!(o.status.code(), Some(1));
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
