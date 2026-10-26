//! Track, untrack, tracked folders, hide rules and the pending list, against the
//! real bundled chezmoi and git in isolated temporary HOME/XDG directories.

mod common;

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;
use std::process::Command;

use common::{TestEnv, snapshot};
use serde_json::Value;

struct Run {
    code: i32,
    out: String,
    err: String,
}

impl Run {
    fn json(&self) -> Value {
        serde_json::from_str(&self.out).unwrap_or_else(|e| panic!("not JSON ({e}): {}", self.out))
    }
}

fn dw_in(t: &TestEnv, dir: &Path, args: &[&str]) -> Run {
    let o = Command::new(env!("CARGO_BIN_EXE_dotwarden"))
        .args(args)
        .env_clear()
        .envs(&t.env)
        .current_dir(dir)
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    Run {
        code: o.status.code().unwrap(),
        out: String::from_utf8_lossy(&o.stdout).into_owned(),
        err: String::from_utf8_lossy(&o.stderr).into_owned(),
    }
}

fn dw(t: &TestEnv, args: &[&str]) -> Run {
    dw_in(t, &t.home, args)
}

fn ok(t: &TestEnv, args: &[&str]) -> Run {
    let r = dw(t, args);
    assert_eq!(r.code, 0, "dw {args:?}: {}{}", r.err, r.out);
    r
}

fn setup() -> TestEnv {
    let t = TestEnv::new();
    t.dw_ok(&["init", "--name", "testbox"]);
    t
}

fn write(t: &TestEnv, rel: &str, content: &str) {
    let p = t.home.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

fn chmod(t: &TestEnv, rel: &str, mode: u32) {
    fs::set_permissions(t.home.join(rel), fs::Permissions::from_mode(mode)).unwrap();
}

fn repo(t: &TestEnv) -> std::path::PathBuf {
    t.dirs().repo()
}

fn git_status(t: &TestEnv) -> String {
    t.git(
        &repo(t),
        &["status", "--porcelain", "--untracked-files=all"],
    )
}

fn commits(t: &TestEnv) -> usize {
    t.git(&repo(t), &["rev-list", "--count", "HEAD"])
        .parse()
        .unwrap()
}

fn managed(t: &TestEnv) -> BTreeSet<String> {
    t.store()
        .chezmoi(&["managed", "--include=files,symlinks"])
        .unwrap()
        .stdout
        .lines()
        .map(str::to_string)
        .collect()
}

/// Path, size and content hash of everything in the repository except `.git`
/// (no mtimes: a rollback legitimately rewrites files).
fn digest(dir: &Path) -> Vec<(String, u64, u64)> {
    use std::hash::{Hash, Hasher};
    fn walk(base: &Path, p: &Path, out: &mut Vec<(String, u64, u64)>) {
        let mut kids: Vec<_> = fs::read_dir(p)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .collect();
        kids.sort();
        for k in kids {
            if k.file_name().unwrap() == ".git" {
                continue;
            }
            let rel = k.strip_prefix(base).unwrap().display().to_string();
            if k.is_dir() {
                out.push((format!("{rel}/"), 0, 0));
                walk(base, &k, out);
            } else {
                let body = fs::read(&k).unwrap();
                let mut h = std::collections::hash_map::DefaultHasher::new();
                body.hash(&mut h);
                out.push((rel, body.len() as u64, h.finish()));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn pending_json(t: &TestEnv) -> Value {
    ok(t, &["pending", "list", "--json"]).json()
}

fn pending_ids(t: &TestEnv) -> Vec<u64> {
    pending_json(t)["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_u64().unwrap())
        .collect()
}

// ---- track a file ---------------------------------------------------------

#[test]
fn tracking_a_file_goes_through_pending_then_stages_in_git_without_committing() {
    let t = setup();
    write(&t, ".zshrc", "export A=1\n");
    let before_commits = commits(&t);

    let r = ok(&t, &["track", ".zshrc", "--json"]);
    let added = &r.json()["added"][0];
    assert_eq!(added["already_pending"], false);
    let item = &added["item"];
    assert_eq!(item["kind"], "track");
    assert_eq!(item["id"], 1);
    assert_eq!(item["requester"], "human");
    assert_eq!(item["path"], ".zshrc");
    assert_eq!(item["preview"]["applies_to"], "all machines");
    assert_eq!(item["preview"]["files"][0]["source"], "home/dot_zshrc");
    assert_eq!(item["preview"]["files"][0]["action"], "add");
    assert!(item["created"].as_str().unwrap().ends_with('Z'));

    // Pending only: the repository and the chezmoi view are unchanged.
    assert_eq!(git_status(&t), "");
    assert!(!managed(&t).contains(".zshrc"));
    assert!(t.dirs().pending_file().is_file());

    let r = ok(&t, &["pending", "apply", "--all", "--json"]);
    assert_eq!(r.json()["applied"][0]["status"], "applied");
    assert_eq!(git_status(&t), "A  home/dot_zshrc");
    assert_eq!(commits(&t), before_commits, "applying must not commit");
    assert!(managed(&t).contains(".zshrc"));
    assert_eq!(
        fs::read_to_string(repo(&t).join("home/dot_zshrc")).unwrap(),
        "export A=1\n"
    );
    assert_eq!(
        fs::read_to_string(t.home.join(".zshrc")).unwrap(),
        "export A=1\n"
    );
    assert!(pending_ids(&t).is_empty());
    // chezmoi's last-written state matches HOME, so there is nothing to apply.
    assert_eq!(t.store().chezmoi(&["status"]).unwrap().stdout.trim(), "");
    // No assignment: applies to every machine.
    let data = fs::read_to_string(repo(&t).join("home/.chezmoidata/dotwarden.toml")).unwrap();
    assert!(!data.contains("dotwarden.assignments"), "{data}");
}

#[test]
fn the_preview_names_exactly_the_repository_files_that_get_staged() {
    let t = setup();
    write(&t, ".plain", "p");
    write(&t, ".secret-ish", "s");
    chmod(&t, ".secret-ish", 0o600);
    write(&t, ".script", "#!/bin/sh\n");
    chmod(&t, ".script", 0o755);
    write(&t, ".empty", "");
    write(&t, "dot_literal", "x");
    write(&t, ".config/we ird/a \"q\" b", "q");
    write(&t, "ünï b", "u");
    let args = [
        ".plain",
        ".secret-ish",
        ".script",
        ".empty",
        "dot_literal",
        ".config/we ird/a \"q\" b",
        "ünï b",
    ];
    let mut a = vec!["track"];
    a.extend(args);
    a.push("--json");
    let r = ok(&t, &a);
    let previewed: BTreeSet<String> = r.json()["added"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|a| a["item"]["preview"]["files"].as_array().unwrap().clone())
        .map(|f| f["source"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(previewed.len(), 7);
    assert!(previewed.contains("home/private_dot_secret-ish"));
    assert!(previewed.contains("home/executable_dot_script"));
    assert!(previewed.contains("home/empty_dot_empty"));
    assert!(previewed.contains("home/literal_dot_literal"));
    ok(&t, &["pending", "apply", "--all"]);
    let staged: BTreeSet<String> = t
        .git(&repo(&t), &["diff", "--cached", "--name-only", "-z"])
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    assert_eq!(staged, previewed, "preview must equal what was staged");
    assert_eq!(git_status(&t).lines().count(), 7);
}

#[test]
fn dry_run_shows_the_preview_and_changes_nothing() {
    let t = setup();
    write(&t, ".zshrc", "x");
    let repo_before = snapshot(&repo(&t));
    let state_before = snapshot(&t.dirs().state_dir());
    let r = ok(&t, &["track", ".zshrc", "--dry-run"]);
    assert!(
        r.out.contains("Dry run: nothing was changed")
            && r.out.contains("home/dot_zshrc")
            && r.out.contains("all machines"),
        "{}",
        r.out
    );
    let j = ok(&t, &["track", ".zshrc", "--dry-run", "--apply", "--json"]).json();
    assert_eq!(j["dry_run"], true);
    assert_eq!(
        j["items"][0]["item"]["preview"]["files"][0]["source"],
        "home/dot_zshrc"
    );
    assert_eq!(snapshot(&repo(&t)), repo_before);
    // pending.json is not created and chezmoi's state is untouched (the lock file may be).
    assert!(!t.dirs().pending_file().exists());
    let state_after = snapshot(&t.dirs().state_dir());
    let strip = |s: Option<Vec<String>>| {
        s.unwrap()
            .into_iter()
            .filter(|l| !l.starts_with("lock "))
            .collect::<Vec<_>>()
    };
    assert_eq!(strip(state_after), strip(state_before));
}

#[test]
fn apply_flag_tracks_immediately_without_leaving_a_pending_item() {
    let t = setup();
    write(&t, ".zshrc", "x");
    let r = ok(&t, &["track", ".zshrc", "--apply"]);
    assert!(r.out.contains("applied"), "{}", r.out);
    assert_eq!(git_status(&t), "A  home/dot_zshrc");
    assert!(pending_ids(&t).is_empty());
}

#[test]
fn agents_are_recorded_as_the_requester_and_may_apply_track() {
    let mut t = setup();
    t.set("DOTWARDEN_AGENT", "claude");
    write(&t, ".zshrc", "x");
    write(&t, ".b", "b");
    let j = ok(&t, &["track", ".zshrc", "--json"]).json();
    assert_eq!(j["added"][0]["item"]["requester"], "claude");
    assert_eq!(pending_json(&t)["items"][0]["requester"], "claude");
    // Track and untrack are permitted local actions: no approval exit code.
    let r = ok(&t, &["track", ".b", "--apply"]);
    assert_eq!(r.code, 0);
    let r = ok(&t, &["pending", "list"]);
    assert!(
        r.out.contains("claude") && r.out.contains("Track ~/.zshrc"),
        "{}",
        r.out
    );
}

// ---- folders and symlinks ---------------------------------------------------

#[test]
fn tracking_a_folder_records_it_tracks_current_files_and_new_files_appear_as_new() {
    let t = setup();
    write(&t, ".config/app/a.conf", "a");
    write(&t, ".config/app/sub/b.conf", "b");
    write(&t, ".config/app/node_modules/pkg/x.js", "hidden");
    write(&t, ".config/app/sub/.git/config", "nested repo");

    let j = ok(&t, &["track", ".config/app", "--json"]).json();
    let item = &j["added"][0]["item"];
    assert_eq!(item["folder"], true);
    assert_eq!(item["record_folder"], true);
    let targets: BTreeSet<String> = item["preview"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["target"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        targets,
        set(&[".config/app/a.conf", ".config/app/sub/b.conf"])
    );
    let notes = item["preview"]["notes"].to_string();
    assert!(
        notes.contains("hide rules") && notes.contains(".git"),
        "{notes}"
    );
    ok(&t, &["pending", "apply", "--all"]);

    assert_eq!(
        managed(&t),
        set(&[".config/app/a.conf", ".config/app/sub/b.conf"])
    );
    let data = fs::read_to_string(repo(&t).join("home/.chezmoidata/dotwarden.toml")).unwrap();
    assert!(
        data.contains("tracked_folders = [\".config/app\"]"),
        "{data}"
    );
    assert!(git_status(&t).contains("M  home/.chezmoidata/dotwarden.toml"));

    // A file created later is New: visible in status and ls, not tracked.
    write(&t, ".config/app/new.conf", "n");
    write(&t, ".config/app/node_modules/pkg/y.js", "still hidden");
    let s = ok(&t, &["status", "--json"]).json();
    assert_eq!(s["new_files"], serde_json::json!([".config/app/new.conf"]));
    assert_eq!(s["tracked_files"], 2);
    assert_eq!(s["tracked_folders"], 1);
    let human = ok(&t, &["status"]).out;
    assert!(
        human.contains("New files:  1") && human.contains("~/.config/app/new.conf"),
        "{human}"
    );
    let ls = ok(&t, &["ls", ".config/app", "--json"]).json();
    let state = |name: &str| {
        ls["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["name"] == name)
            .unwrap()["state"]
            .clone()
    };
    assert_eq!(state("a.conf"), "tracked");
    assert_eq!(state("new.conf"), "new");
    assert_eq!(state("sub"), "contains-tracked");
    assert!(
        ls["entries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["name"] != "node_modules")
    );
    assert_eq!(
        ok(&t, &["ls", ".config", "--json"]).json()["entries"][0]["state"],
        "tracked-folder"
    );
    assert!(!managed(&t).contains(".config/app/new.conf"));

    // Tracking the folder again picks the new file up.
    let j = ok(&t, &["track", ".config/app", "--apply", "--json"]).json();
    assert_eq!(j["applied"][0]["status"], "applied");
    assert!(managed(&t).contains(".config/app/new.conf"));
    assert_eq!(
        ok(&t, &["status", "--json"]).json()["new_files"],
        serde_json::json!([])
    );
    // Nothing left to track now.
    let r = dw(&t, &["track", ".config/app"]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("nothing to track"), "{}", r.err);
}

#[test]
fn symlinks_are_tracked_as_symlinks_and_never_followed() {
    let t = setup();
    write(&t, ".zshrc", "x");
    symlink(".zshrc", t.home.join("link")).unwrap();
    fs::create_dir_all(t.home.join("realdir")).unwrap();
    write(&t, "realdir/f", "f");
    symlink("realdir", t.home.join("dirlink")).unwrap();
    write(&t, "folder/real", "r");
    symlink("../.zshrc", t.home.join("folder/inner")).unwrap();

    let j = ok(
        &t,
        &["track", "link", "dirlink", "folder", "--apply", "--json"],
    )
    .json();
    assert!(
        j["applied"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["status"] == "applied")
    );
    assert_eq!(
        fs::read_to_string(repo(&t).join("home/symlink_link"))
            .unwrap()
            .trim(),
        ".zshrc"
    );
    assert_eq!(
        fs::read_to_string(repo(&t).join("home/symlink_dirlink"))
            .unwrap()
            .trim(),
        "realdir"
    );
    assert!(!repo(&t).join("home/dirlink").exists());
    assert!(repo(&t).join("home/folder/symlink_inner").is_file());
    let m = t
        .store()
        .chezmoi(&["managed", "--include=symlinks"])
        .unwrap()
        .stdout;
    let links: BTreeSet<_> = m.lines().collect();
    assert_eq!(links, ["dirlink", "folder/inner", "link"].into());
    let ls = ok(&t, &["ls", "--json"]).json();
    let link = ls["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["name"] == "link")
        .unwrap();
    assert_eq!(
        (link["kind"].as_str(), link["state"].as_str()),
        (Some("symlink"), Some("tracked"))
    );
    // The HOME symlinks are untouched.
    assert!(
        fs::symlink_metadata(t.home.join("link"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

// ---- refusals ---------------------------------------------------------------

#[test]
fn hidden_paths_are_refused_with_the_rule_named() {
    let t = setup();
    write(&t, ".cache/x/y", "c");
    write(&t, ".zsh_history", "h");
    for p in [".cache/x/y", ".cache", ".zsh_history"] {
        let r = dw(&t, &["track", p]);
        assert_eq!(r.code, 1, "{p}: {}", r.err);
        assert!(
            r.err.contains("hidden") && r.err.contains("hide rule"),
            "{}",
            r.err
        );
    }
    let r = dw(&t, &["track", ".zsh_history", "--json"]);
    assert_eq!(r.code, 1);
    let e = &r.json()["error"];
    assert_eq!(e["code"], 1);
    assert!(e["message"].as_str().unwrap().contains(".zsh_history"));
    assert!(!t.dirs().pending_file().exists());
}

#[test]
fn paths_outside_home_own_directories_and_unrepresentable_names_are_refused() {
    let t = setup();
    let outside = t.root.path().canonicalize().unwrap().join("outside.txt");
    fs::write(&outside, "o").unwrap();
    let r = dw(&t, &["track", outside.to_str().unwrap()]);
    assert_eq!(r.code, 2);
    assert!(r.err.contains("outside HOME"), "{}", r.err);
    assert_eq!(dw(&t, &["track", "../outside.txt"]).code, 2);
    assert_eq!(dw(&t, &["track", "~"]).code, 2);
    assert_eq!(dw(&t, &["track", "~otheruser/x"]).code, 2);

    let own = t.dirs().repo().join("dotwarden.toml");
    let r = dw(&t, &["track", own.to_str().unwrap()]);
    assert_eq!(r.code, 2);
    assert!(r.err.contains("own directories"), "{}", r.err);
    let r = dw(&t, &["track", "~/.local/state/dotwarden/machine.toml"]);
    assert_eq!(r.code, 2, "{}", r.err);

    write(&t, "bad\nname", "n");
    let r = dw(&t, &["track", "bad\nname"]);
    assert_eq!(r.code, 2);
    assert!(r.err.contains("control character"), "{}", r.err);

    // Below a symlinked folder.
    fs::create_dir_all(t.home.join("real")).unwrap();
    write(&t, "real/f", "f");
    symlink("real", t.home.join("via")).unwrap();
    let r = dw(&t, &["track", "via/f"]);
    assert_eq!(r.code, 2);
    assert!(r.err.contains("symlink"), "{}", r.err);

    let r = dw(&t, &["track", "missing"]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("does not exist"), "{}", r.err);
    assert!(!t.dirs().pending_file().exists());
}

#[test]
fn one_refused_path_means_nothing_is_added() {
    let t = setup();
    write(&t, ".ok", "ok");
    write(&t, ".zsh_history", "h");
    let r = dw(&t, &["track", ".ok", ".zsh_history", "nope"]);
    assert_ne!(r.code, 0);
    assert!(
        r.err.contains(".zsh_history") && r.err.contains("nope"),
        "all reasons: {}",
        r.err
    );
    assert!(pending_ids(&t).is_empty());
}

#[test]
fn already_tracked_and_duplicate_requests() {
    let t = setup();
    write(&t, ".a", "a");
    ok(&t, &["track", ".a", "--apply"]);
    let r = dw(&t, &["track", ".a"]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("already tracked"), "{}", r.err);

    write(&t, ".b", "b");
    ok(&t, &["track", ".b"]);
    let j = ok(&t, &["track", ".b", ".b", "--json"]).json();
    assert_eq!(
        j["added"].as_array().unwrap().len(),
        1,
        "duplicate args collapse"
    );
    assert_eq!(j["added"][0]["already_pending"], true);
    assert_eq!(pending_ids(&t), [1]);
}

#[test]
fn relative_paths_resolve_against_the_current_folder() {
    let t = setup();
    write(&t, ".config/app/a.conf", "a");
    let cwd = t.home.join(".config/app");
    let r = dw_in(&t, &cwd, &["track", "a.conf", "--json"]);
    assert_eq!(r.code, 0, "{}", r.err);
    assert_eq!(r.json()["added"][0]["item"]["path"], ".config/app/a.conf");
    let abs = t.home.join(".config/app");
    let r = dw_in(
        &t,
        Path::new("/"),
        &["track", abs.to_str().unwrap(), "--dry-run", "--json"],
    );
    assert_eq!(r.code, 0, "{}", r.err);
    let r = dw_in(&t, &cwd, &["ls"]);
    assert!(
        r.out.contains("a.conf"),
        "ls defaults to the current folder: {}",
        r.out
    );
}

// ---- untrack ----------------------------------------------------------------

#[test]
fn untracking_removes_it_from_the_repository_stages_the_deletion_and_keeps_home() {
    let t = setup();
    write(&t, ".zshrc", "keep me");
    write(&t, ".other", "o");
    ok(&t, &["track", ".zshrc", ".other", "--apply"]);
    t.git(&repo(&t), &["commit", "-qm", "track"]);

    let j = ok(&t, &["untrack", ".zshrc", "--json"]).json();
    let item = &j["added"][0]["item"];
    assert_eq!(item["kind"], "untrack");
    assert_eq!(item["preview"]["files"][0]["action"], "remove");
    assert_eq!(item["preview"]["files"][0]["source"], "home/dot_zshrc");
    assert!(managed(&t).contains(".zshrc"), "pending only so far");
    ok(&t, &["pending", "apply", "--all"]);
    assert_eq!(git_status(&t), "D  home/dot_zshrc");
    assert!(!repo(&t).join("home/dot_zshrc").exists());
    assert_eq!(managed(&t), set(&[".other"]));
    assert_eq!(
        fs::read_to_string(t.home.join(".zshrc")).unwrap(),
        "keep me"
    );
    // And it can be tracked again.
    ok(&t, &["track", ".zshrc", "--apply"]);
    assert_eq!(git_status(&t), "");
}

#[test]
fn untracking_a_folder_forgets_it_its_files_and_their_assignments() {
    let t = setup();
    write(&t, ".config/app/a", "a");
    write(&t, ".config/app/deep/b", "b");
    write(&t, ".keep", "k");
    ok(&t, &["track", ".config/app", ".keep", "--apply"]);
    dotwarden::assign::set_assignment(&repo(&t), ".config/app/a", &["platform:linux".into()], &[])
        .unwrap();
    dotwarden::assign::set_assignment(&repo(&t), ".keep", &["platform:linux".into()], &[]).unwrap();
    t.git(&repo(&t), &["add", "-A"]);
    t.git(&repo(&t), &["commit", "-qm", "setup"]);

    let j = ok(&t, &["untrack", ".config/app", "--dry-run", "--json"]).json();
    let data = j["items"][0]["item"]["preview"]["data"].to_string();
    assert!(
        data.contains("tracked folder") && data.contains("assignment of ~/.config/app/a"),
        "{data}"
    );

    ok(&t, &["untrack", ".config/app", "--apply"]);
    let status = git_status(&t);
    assert!(status.contains("D  home/dot_config/app/a"), "{status}");
    assert!(status.contains("D  home/dot_config/app/deep/b"), "{status}");
    assert!(
        status.contains("M  home/.chezmoidata/dotwarden.toml"),
        "{status}"
    );
    assert!(
        !repo(&t).join("home/dot_config").exists(),
        "empty source folders are pruned"
    );
    let data = fs::read_to_string(repo(&t).join("home/.chezmoidata/dotwarden.toml")).unwrap();
    assert!(
        !data.contains("tracked_folders") && !data.contains(".config/app"),
        "{data}"
    );
    assert!(data.contains("\".keep\""), "other assignments stay: {data}");
    assert!(t.home.join(".config/app/deep/b").is_file());
    assert_eq!(
        ok(&t, &["status", "--json"]).json()["new_files"],
        serde_json::json!([])
    );
}

#[test]
fn untracking_a_file_inside_a_tracked_folder_leaves_the_folder_and_shows_it_as_new() {
    let t = setup();
    write(&t, "dir/a", "a");
    write(&t, "dir/b", "b");
    ok(&t, &["track", "dir", "--apply"]);
    let j = ok(&t, &["untrack", "dir/a", "--dry-run", "--json"]).json();
    assert!(
        j["items"][0]["item"]["preview"]["notes"]
            .to_string()
            .contains("shows as New")
    );
    ok(&t, &["untrack", "dir/a", "--apply"]);
    assert_eq!(managed(&t), set(&["dir/b"]));
    assert_eq!(
        ok(&t, &["status", "--json"]).json()["new_files"],
        serde_json::json!(["dir/a"])
    );
}

#[test]
fn untracking_something_untracked_is_an_error() {
    let t = setup();
    write(&t, ".x", "x");
    let r = dw(&t, &["untrack", ".x"]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("not tracked"), "{}", r.err);
}

#[test]
fn a_file_assigned_away_from_this_machine_still_counts_as_tracked() {
    let t = setup();
    write(&t, ".linux-only", "shared content");
    ok(&t, &["track", ".linux-only", "--apply"]);
    dotwarden::assign::set_assignment(&repo(&t), ".linux-only", &["platform:linux".into()], &[])
        .unwrap();
    t.git(&repo(&t), &["add", "-A"]);
    t.git(&repo(&t), &["commit", "-qm", "assign"]);
    // chezmoi no longer lists it on this machine ...
    let det = dotwarden::machine::detect().unwrap();
    if det.platform != "linux" {
        assert!(!managed(&t).contains(".linux-only"));
    }
    // ... but dotwarden does, so it cannot be tracked over the shared version.
    write(&t, ".linux-only", "this machine's content");
    let r = dw(&t, &["track", ".linux-only"]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("already tracked"), "{}", r.err);
    let e = &ok(&t, &["ls", "--json"]).json()["entries"];
    let st = e
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == ".linux-only")
        .unwrap()["state"]
        .clone();
    assert_eq!(st, "tracked");
    // It can be untracked, and the assignment goes with it.
    ok(&t, &["untrack", ".linux-only", "--apply"]);
    let data = fs::read_to_string(repo(&t).join("home/.chezmoidata/dotwarden.toml")).unwrap();
    assert!(!data.contains(".linux-only"), "{data}");
    assert!(!repo(&t).join("home/dot_linux-only").exists());
}

// ---- decoding the source state ------------------------------------------------

#[test]
fn the_source_decoder_agrees_with_chezmoi_managed() {
    let t = setup();
    for (name, mode) in [
        (".plain", 0o644),
        (".priv", 0o600),
        (".exe", 0o755),
        (".pe", 0o700),
        (".ro", 0o444),
        ("dot_literal", 0o644),
        ("private_x", 0o644),
        ("exact_y", 0o644),
        ("a b/c d", 0o644),
        (".ssh/config", 0o600),
        ("ünï/ö", 0o644),
        ("x.tmpl", 0o644),
        ("run_thing", 0o644),
        ("symlink_z", 0o644),
        ("literal_w", 0o644),
        ("empty_q", 0o644),
    ] {
        write(&t, name, "c");
        chmod(&t, name, mode);
    }
    write(&t, "emptyfile", "");
    fs::create_dir_all(t.home.join(".ssh")).unwrap();
    chmod(&t, ".ssh", 0o700);
    symlink("emptyfile", t.home.join("sl")).unwrap();
    let all: Vec<String> = fs::read_dir(&t.home)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !matches!(n.as_str(), ".gitconfig" | ".local" | ".cache" | ".config"))
        .collect();
    let mut a: Vec<&str> = vec!["track"];
    a.extend(all.iter().map(String::as_str));
    a.push("--apply");
    let r = dw(&t, &a);
    assert_eq!(r.code, 0, "{}{}", r.err, r.out);
    let tracked = dotwarden::source::load(&repo(&t)).unwrap();
    let ours: BTreeSet<String> = tracked.files.keys().cloned().collect();
    assert_eq!(ours, managed(&t));
    assert!(ours.len() >= 15);
    let links: BTreeSet<String> = tracked
        .files
        .iter()
        .filter(|(_, e)| e.symlink)
        .map(|(k, _)| k.clone())
        .collect();
    let chezmoi_links: BTreeSet<String> = t
        .store()
        .chezmoi(&["managed", "--include=symlinks"])
        .unwrap()
        .stdout
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(links, chezmoi_links);
}

// ---- pending list ---------------------------------------------------------------

#[test]
fn pending_list_show_drop_and_ids_are_never_reused() {
    let t = setup();
    for f in [".a", ".b", ".c"] {
        write(&t, f, f);
    }
    ok(&t, &["track", ".a", ".b", ".c"]);
    assert_eq!(pending_ids(&t), [1, 2, 3]);
    let show = ok(&t, &["pending", "show", "2"]).out;
    assert!(
        show.contains("#2")
            && show.contains("Track ~/.b")
            && show.contains("requested by human")
            && show.contains("home/dot_b")
            && show.contains("applies to: all machines"),
        "{show}"
    );
    let j = ok(&t, &["pending", "show", "2", "--json"]).json();
    assert_eq!(j["id"], 2);
    assert!(j["stale"].is_null());

    let dry = ok(&t, &["pending", "drop", "2", "--dry-run"]);
    assert!(dry.out.contains("Dry run"), "{}", dry.out);
    assert_eq!(pending_ids(&t), [1, 2, 3]);
    ok(&t, &["pending", "drop", "2"]);
    assert_eq!(pending_ids(&t), [1, 3]);
    write(&t, ".d", "d");
    ok(&t, &["track", ".d"]);
    assert_eq!(pending_ids(&t), [1, 3, 4], "ids are not reused");

    let r = dw(&t, &["pending", "drop", "1", "9"]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("no pending item #9"), "{}", r.err);
    assert_eq!(
        pending_ids(&t),
        [1, 3, 4],
        "nothing dropped when one id is unknown"
    );
    assert_eq!(dw(&t, &["pending", "drop"]).code, 2);
    assert_eq!(dw(&t, &["pending", "apply"]).code, 2);
    assert_eq!(dw(&t, &["pending", "show", "77"]).code, 1);
    ok(&t, &["pending", "drop", "--all"]);
    assert!(pending_ids(&t).is_empty());
    assert!(ok(&t, &["pending"]).out.contains("No pending items"));
}

#[test]
fn applying_selected_items_leaves_the_others_pending() {
    let t = setup();
    for f in [".a", ".b"] {
        write(&t, f, f);
    }
    ok(&t, &["track", ".a", ".b"]);
    let dry = ok(&t, &["pending", "apply", "2", "--dry-run"]);
    assert!(dry.out.contains("Track ~/.b") && !dry.out.contains("Track ~/.a"));
    assert_eq!(git_status(&t), "");
    ok(&t, &["pending", "apply", "2"]);
    assert_eq!(git_status(&t), "A  home/dot_b");
    assert_eq!(pending_ids(&t), [1]);
}

#[test]
fn status_counts_pending_items() {
    let t = setup();
    write(&t, ".a", "a");
    assert_eq!(ok(&t, &["status", "--json"]).json()["pending"], 0);
    ok(&t, &["track", ".a"]);
    assert_eq!(ok(&t, &["status", "--json"]).json()["pending"], 1);
    assert!(ok(&t, &["status"]).out.contains("Pending:    1"));
}

#[test]
fn a_corrupt_pending_file_is_reported_not_overwritten() {
    let t = setup();
    write(&t, ".a", "a");
    fs::create_dir_all(t.dirs().state_dir()).unwrap();
    fs::write(t.dirs().pending_file(), "{ nope").unwrap();
    let r = dw(&t, &["track", ".a"]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("not a valid pending list"), "{}", r.err);
    assert_eq!(
        fs::read_to_string(t.dirs().pending_file()).unwrap(),
        "{ nope"
    );
}

// ---- freshness ------------------------------------------------------------------

#[test]
fn an_item_whose_file_changed_is_stale_exit_7_and_kept() {
    let t = setup();
    write(&t, ".a", "one");
    write(&t, ".b", "fine");
    ok(&t, &["track", ".a", ".b"]);
    write(&t, ".a", "changed after the item was added");

    let list = pending_json(&t);
    let stale = list["items"][0]["stale"].as_str().unwrap();
    assert!(stale.contains("~/.a in HOME"), "{stale}");
    assert!(list["items"][1]["stale"].is_null());
    assert!(ok(&t, &["pending"]).out.contains("[stale]"));

    let r = dw(&t, &["pending", "apply", "--all", "--json"]);
    assert_eq!(r.code, 7, "{}", r.err);
    let res = r.json();
    assert_eq!(res["applied"][0]["status"], "stale");
    assert_eq!(
        res["applied"][1]["status"], "applied",
        "fresh items still apply"
    );
    assert_eq!(git_status(&t), "A  home/dot_b");
    assert_eq!(pending_ids(&t), [1], "the stale item is kept");
    let human = dw(&t, &["pending", "apply", "1"]);
    assert_eq!(human.code, 7);
    assert!(
        human.out.contains("stale") && human.out.contains("item kept"),
        "{}",
        human.out
    );
    let dry = dw(&t, &["pending", "apply", "1", "--dry-run"]);
    assert_eq!(dry.code, 7);
    assert!(dry.out.contains("STALE"), "{}", dry.out);

    // Dropping and re-adding gives a fresh preview.
    ok(&t, &["pending", "drop", "1"]);
    ok(&t, &["track", ".a", "--apply"]);
    assert_eq!(
        fs::read_to_string(repo(&t).join("home/dot_a")).unwrap(),
        "changed after the item was added"
    );
}

#[test]
fn an_item_is_stale_when_the_repository_changed_underneath_it() {
    let t = setup();
    write(&t, ".a", "a");
    ok(&t, &["track", ".a"]);
    ok(&t, &["track", ".a", "--apply"]); // same path tracked meanwhile
    let r = dw(&t, &["pending", "apply", "--all"]);
    assert_eq!(r.code, 7);
    assert!(r.out.contains("repository entry for ~/.a"), "{}", r.out);

    // An untrack item goes stale when the tracked file's source was rewritten.
    write(&t, ".b", "b");
    ok(&t, &["track", ".b", "--apply"]);
    ok(&t, &["untrack", ".b"]);
    std::thread::sleep(std::time::Duration::from_millis(20));
    fs::write(repo(&t).join("home/dot_b"), "changed in the repo").unwrap();
    t.git(&repo(&t), &["add", "-A"]);
    let list = pending_json(&t);
    let reasons: Vec<_> = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|i| i["stale"].as_str())
        .collect();
    assert!(reasons.iter().any(|r| r.contains("~/.b")), "{reasons:?}");
}

#[test]
fn a_hide_rule_added_after_planning_makes_the_track_item_stale() {
    let t = setup();
    write(&t, ".thing", "t");
    ok(&t, &["track", ".thing"]);
    ok(&t, &["hide", "add", ".thing", "--apply"]);
    let r = dw(&t, &["pending", "apply", "1"]);
    assert_eq!(r.code, 7, "{}{}", r.out, r.err);
    assert!(r.out.contains("hide rules"), "{}", r.out);
}

// ---- all-or-nothing -------------------------------------------------------------------

/// Commit a source file that makes chezmoi fail when it adds files below `dir`
/// (a file where a folder is needed). Done after planning, so the item is fresh.
fn plant_conflict(t: &TestEnv, rel: &str) {
    let p = repo(t).join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(&p, "in the way").unwrap();
    t.git(&repo(t), &["add", "-A"]);
    t.git(&repo(t), &["commit", "-qm", "conflict"]);
}

#[test]
fn a_failing_item_is_reported_and_leaves_nothing_behind_while_others_apply() {
    let t = setup();
    write(&t, "good/a", "a");
    write(&t, "bad/a", "1");
    write(&t, "bad/z/second", "2");
    write(&t, ".fine", "f");
    ok(&t, &["track", "bad", "good", ".fine"]);
    plant_conflict(&t, "home/bad/z");

    let r = dw(&t, &["pending", "apply", "--all", "--json"]);
    assert_eq!(r.code, 1, "{}{}", r.err, r.out);
    let res = r.json();
    let by_summary = |needle: &str| {
        res["applied"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["summary"].as_str().unwrap().contains(needle))
            .unwrap()
            .clone()
    };
    let bad = by_summary("~/bad");
    assert_eq!(bad["status"], "failed");
    assert!(bad["message"].as_str().unwrap().len() > 5, "{bad}");
    assert_eq!(by_summary("~/good")["status"], "applied");
    assert_eq!(by_summary("~/.fine")["status"], "applied");

    assert!(!repo(&t).join("home/bad/a").exists(), "no partial files");
    let data = fs::read_to_string(repo(&t).join("home/.chezmoidata/dotwarden.toml")).unwrap();
    assert!(!data.contains("bad"), "{data}");
    assert_eq!(
        git_status(&t).lines().collect::<BTreeSet<_>>(),
        [
            "A  home/dot_fine",
            "A  home/good/a",
            "M  home/.chezmoidata/dotwarden.toml"
        ]
        .into()
    );
    assert_eq!(pending_ids(&t).len(), 1);
    assert!(
        pending_json(&t)["items"][0]["summary"]
            .as_str()
            .unwrap()
            .contains("~/bad")
    );
    // Once the problem is fixed the kept item applies.
    t.git(&repo(&t), &["rm", "-q", "home/bad/z"]);
    t.git(&repo(&t), &["commit", "-qm", "fix"]);
    ok(&t, &["pending", "apply", "--all"]);
    assert!(managed(&t).contains("bad/z/second"));
    assert!(pending_ids(&t).is_empty());
}

#[test]
fn rollback_restores_the_repository_exactly_even_after_files_and_data_were_written() {
    let t = setup();
    write(&t, ".kept", "k");
    ok(&t, &["track", ".kept", "--apply"]);
    write(&t, "bad/x", "x");
    write(&t, "bad/y", "y");
    let store = t.store();
    let mut items = dotwarden::track::plan_paths(
        &store,
        dotwarden::track::Plan::Track,
        &["bad".to_string()],
        &t.home,
        "human",
    )
    .unwrap();
    // The preview promises a different file than chezmoi will write, so the
    // check after writing fails.
    items[0].preview.files[0].source = "home/bogus".into();
    let status_before = git_status(&t);
    let head_before = t.git(&repo(&t), &["rev-parse", "HEAD"]);
    let tree_before = digest(&repo(&t));

    let results = dotwarden::track::apply_now(&store, &items).unwrap();
    assert_eq!(results[0].status, dotwarden::track::Outcome::Failed);
    assert!(
        results[0]
            .message
            .as_ref()
            .unwrap()
            .contains("differently than previewed"),
        "{:?}",
        results[0].message
    );
    assert_eq!(git_status(&t), status_before);
    assert_eq!(t.git(&repo(&t), &["rev-parse", "HEAD"]), head_before);
    assert_eq!(digest(&repo(&t)), tree_before);
    assert!(!repo(&t).join("home/bad").exists());
    // The same plan, untampered, applies.
    let items = dotwarden::track::plan_paths(
        &store,
        dotwarden::track::Plan::Track,
        &["bad".to_string()],
        &t.home,
        "human",
    )
    .unwrap();
    let results = dotwarden::track::apply_now(&store, &items).unwrap();
    assert_eq!(results[0].status, dotwarden::track::Outcome::Applied);
}

#[test]
fn unstaged_edits_in_the_repository_block_applying_and_are_left_alone() {
    let t = setup();
    write(&t, ".a", "a");
    ok(&t, &["track", ".a"]);
    fs::write(repo(&t).join("stray.txt"), "someone's edit").unwrap();
    let r = dw(&t, &["pending", "apply", "--all"]);
    assert_eq!(r.code, 1);
    assert!(
        r.err.contains("not staged") && r.err.contains("stray.txt"),
        "{}",
        r.err
    );
    assert_eq!(
        fs::read_to_string(repo(&t).join("stray.txt")).unwrap(),
        "someone's edit"
    );
    assert_eq!(pending_ids(&t), [1]);
}

// ---- concurrency -----------------------------------------------------------------------

#[test]
fn concurrent_cli_processes_never_lose_or_duplicate_pending_items() {
    let t = setup();
    let n = 8;
    for i in 0..n {
        write(&t, &format!(".c{i}"), "c");
    }
    let handles: Vec<_> = (0..n)
        .map(|i| {
            let env = t.env.clone();
            let home = t.home.clone();
            std::thread::spawn(move || {
                Command::new(env!("CARGO_BIN_EXE_dotwarden"))
                    .args(["track", &format!(".c{i}")])
                    .env_clear()
                    .envs(&env)
                    .current_dir(&home)
                    .output()
                    .unwrap()
            })
        })
        .collect();
    for h in handles {
        let o = h.join().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    }
    let mut ids = pending_ids(&t);
    ids.sort();
    assert_eq!(ids, (1..=n as u64).collect::<Vec<_>>());
    let paths: BTreeSet<String> = pending_json(&t)["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["path"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(paths.len(), n);

    // Concurrent apply of everything plus a concurrent drop attempt stays consistent.
    let r = ok(&t, &["pending", "apply", "--all", "--json"]);
    assert_eq!(r.json()["applied"].as_array().unwrap().len(), n);
    assert_eq!(managed(&t).len(), n);
    assert_eq!(git_status(&t).lines().count(), n);
}

// ---- hide rules ------------------------------------------------------------------------

#[test]
fn hide_rules_change_through_the_pending_list_and_refuse_tracking_afterwards() {
    let t = setup();
    write(&t, "scratch/notes", "n");
    assert!(ok(&t, &["hide"]).out.contains(".cache/"));
    let j = ok(&t, &["hide", "list", "--json"]).json();
    assert!(
        j["rules"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r == ".cache/")
    );

    let dry = ok(&t, &["hide", "add", "/scratch/", "--dry-run"]);
    assert!(dry.out.contains("add `/scratch/`"), "{}", dry.out);
    let hide_before = fs::read_to_string(repo(&t).join("hide")).unwrap();
    let j = ok(&t, &["hide", "add", "/scratch/", "--json"]).json();
    assert_eq!(j["added"][0]["item"]["kind"], "hide_add");
    assert_eq!(
        fs::read_to_string(repo(&t).join("hide")).unwrap(),
        hide_before,
        "pending only"
    );
    ok(&t, &["pending", "apply", "--all"]);
    assert_eq!(git_status(&t), "M  hide");
    let hide = fs::read_to_string(repo(&t).join("hide")).unwrap();
    assert!(
        hide.ends_with("/scratch/\n") && hide.starts_with(hide_before.trim_end()),
        "{hide}"
    );
    assert_eq!(dw(&t, &["track", "scratch"]).code, 1);
    let ls = ok(&t, &["ls", "--json"]).json();
    assert!(
        ls["entries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["name"] != "scratch")
    );
    let ls = ok(&t, &["ls", "--all", "--json"]).json();
    let e = ls["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["name"] == "scratch")
        .unwrap();
    assert_eq!(
        (e["state"].as_str(), e["hidden"].as_bool()),
        (Some("hidden"), Some(true))
    );

    // Duplicates and bad rules are refused up front.
    assert_eq!(dw(&t, &["hide", "add", "/scratch/"]).code, 1);
    assert_eq!(dw(&t, &["hide", "add", "#comment"]).code, 2);
    assert_eq!(dw(&t, &["hide", "remove", "/never-was/"]).code, 1);

    // Remove goes through pending too, and tracking works again.
    ok(&t, &["hide", "remove", "/scratch/", "--apply"]);
    assert!(
        !fs::read_to_string(repo(&t).join("hide"))
            .unwrap()
            .contains("/scratch/")
    );
    ok(&t, &["track", "scratch", "--apply"]);
}

#[test]
fn a_negated_rule_unhides_a_path_for_tracking() {
    let t = setup();
    write(&t, ".cache/keep/file", "k");
    assert_eq!(dw(&t, &["track", ".cache/keep/file"]).code, 1);
    ok(&t, &["hide", "add", "!/.cache/keep/", "--apply"]);
    ok(&t, &["track", ".cache/keep/file", "--apply"]);
    assert!(managed(&t).contains(".cache/keep/file"));
}

#[test]
fn hiding_a_pattern_warns_about_tracked_files_it_matches_and_leaves_them_tracked() {
    let t = setup();
    write(&t, ".a.log", "l");
    ok(&t, &["track", ".a.log", "--apply"]);
    let j = ok(&t, &["hide", "add", "*.log", "--dry-run", "--json"]).json();
    let notes = j["items"][0]["item"]["preview"]["notes"].to_string();
    assert!(
        notes.contains("stay tracked") && notes.contains(".a.log"),
        "{notes}"
    );
    ok(&t, &["hide", "add", "*.log", "--apply"]);
    assert!(managed(&t).contains(".a.log"));
}

// ---- isolation ----------------------------------------------------------------------------

#[test]
fn tracking_never_touches_a_users_own_chezmoi_directories() {
    let t = setup();
    for d in [".config/chezmoi", ".local/share/chezmoi"] {
        write(&t, &format!("{d}/marker"), "mine");
    }
    write(&t, ".zshrc", "z");
    let own = |t: &TestEnv| {
        (
            snapshot(&t.home.join(".config/chezmoi")),
            snapshot(&t.home.join(".local/share/chezmoi")),
        )
    };
    let before = own(&t);
    ok(&t, &["track", ".zshrc", "--apply"]);
    ok(&t, &["untrack", ".zshrc", "--apply"]);
    ok(&t, &["track", ".zshrc"]);
    ok(&t, &["pending", "apply", "--all"]);
    assert_eq!(own(&t), before);
    // They cannot be tracked either: they are HOME files, so this is allowed,
    // but dotwarden's own store is refused.
    assert_eq!(dw(&t, &["track", "~/.local/share/dotwarden"]).code, 2);
}

#[test]
fn json_errors_use_the_documented_shape_for_usage_problems() {
    let t = setup();
    let r = dw(&t, &["track", "/etc/hosts", "--json"]);
    assert_eq!(r.code, 2);
    let e = &r.json()["error"];
    assert_eq!(
        (e["kind"].as_str(), e["code"].as_u64()),
        (Some("usage"), Some(2))
    );
    let r = dw(&t, &["track", "--json"]);
    assert_eq!(r.code, 2);
}

#[test]
fn commands_need_an_initialised_repository() {
    let t = TestEnv::new();
    for args in [&["track", ".x"][..], &["pending"], &["hide"], &["ls"]] {
        let r = dw(&t, args);
        assert_eq!(r.code, 2, "{args:?}: {}", r.err);
        assert!(r.err.contains("not initialised"), "{}", r.err);
    }
}

// ---- review fixes ---------------------------------------------------------------------

fn entry_keys(t: &TestEnv) -> BTreeSet<String> {
    let out = t
        .store()
        .chezmoi(&["state", "dump", "--format", "json"])
        .unwrap()
        .stdout;
    let v: Value = serde_json::from_str(&out).unwrap();
    v["entryState"]
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default()
}

fn key(t: &TestEnv, rel: &str) -> String {
    t.home.join(rel).display().to_string()
}

/// A libexec whose chezmoi sleeps before `add` (not before a dry run) and then
/// runs the real one; git is the real one.
fn slow_add_libexec(t: &TestEnv, seconds: &str) -> std::path::PathBuf {
    let fake = t.root.path().join("slow-libexec");
    fs::create_dir_all(&fake).unwrap();
    let _ = symlink(common::libexec().join("git"), fake.join("git"));
    let script = format!(
        "#!/bin/sh\nadd=0; dry=0\nfor a in \"$@\"; do [ \"$a\" = add ] && add=1; [ \"$a\" = --dry-run ] && dry=1; done\n\
         [ $add = 1 ] && [ $dry = 0 ] && sleep {seconds}\nexec {} \"$@\"\n",
        common::libexec().join("chezmoi").display()
    );
    fs::write(fake.join("chezmoi"), script).unwrap();
    fs::set_permissions(fake.join("chezmoi"), fs::Permissions::from_mode(0o755)).unwrap();
    fake
}

fn dw_with(t: &TestEnv, extra: &[(&str, &str)], args: &[&str]) -> Run {
    let o = Command::new(env!("CARGO_BIN_EXE_dotwarden"))
        .args(args)
        .env_clear()
        .envs(&t.env)
        .envs(extra.iter().copied())
        .current_dir(&t.home)
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    Run {
        code: o.status.code().unwrap(),
        out: String::from_utf8_lossy(&o.stdout).into_owned(),
        err: String::from_utf8_lossy(&o.stderr).into_owned(),
    }
}

// 1. case ------------------------------------------------------------------------------

#[test]
fn a_path_spelled_with_the_wrong_case_is_refused_naming_the_real_spelling() {
    let t = setup();
    write(&t, ".Zshrc", "z");
    write(&t, "Dir/inner", "i");
    let insensitive = t.home.join(".ZSHRC").exists();
    if insensitive {
        let r = dw(&t, &["track", "~/.ZSHRC"]);
        assert_eq!(r.code, 2, "{}{}", r.out, r.err);
        assert!(
            r.err.contains("~/.Zshrc") && r.err.contains("ignores case"),
            "{}",
            r.err
        );
        let r = dw(&t, &["track", "dir/inner"]);
        assert_eq!(r.code, 2, "{}", r.err);
        assert!(
            r.err.contains("~/Dir/inner"),
            "the folder part too: {}",
            r.err
        );
        let r = dw(&t, &["untrack", ".ZSHRC"]);
        assert_eq!(r.code, 2, "{}", r.err);
        assert!(pending_ids(&t).is_empty());
        assert!(!repo(&t).join("home/dot_ZSHRC").exists());
    }
    // The exact spelling always works and records the real name.
    ok(&t, &["track", ".Zshrc", "Dir/inner", "--apply"]);
    assert!(repo(&t).join("home/dot_Zshrc").is_file());
    assert!(managed(&t).contains(".Zshrc"));
    // A path that does not exist is not a case problem.
    assert_eq!(dw(&t, &["track", ".NoSuch"]).code, 1);
}

// 2. symlinked HOME and canonical paths ------------------------------------------------------

#[test]
fn absolute_paths_through_a_symlink_to_home_are_not_outside_home() {
    let t = setup();
    write(&t, ".zshrc", "z");
    write(&t, ".config/app/a", "a");
    let alias = t.root.path().canonicalize().unwrap().join("alias");
    symlink(&t.home, &alias).unwrap();
    let via = alias.join(".zshrc");
    let j = ok(&t, &["track", via.to_str().unwrap(), "--dry-run", "--json"]).json();
    assert_eq!(j["items"][0]["item"]["path"], ".zshrc");
    let via_dir = alias.join(".config/app");
    let r = ok(
        &t,
        &["track", via_dir.to_str().unwrap(), "--apply", "--json"],
    );
    assert_eq!(r.json()["applied"][0]["status"], "applied");
    assert!(managed(&t).contains(".config/app/a"));
    let r = ok(
        &t,
        &["ls", alias.join(".config").to_str().unwrap(), "--json"],
    );
    assert_eq!(r.json()["path"], "~/.config");
    let r = ok(
        &t,
        &["untrack", via_dir.to_str().unwrap(), "--dry-run", "--json"],
    );
    assert_eq!(r.json()["items"][0]["item"]["path"], ".config/app");
}

#[test]
fn a_symlinked_home_accepts_its_canonical_and_its_symlinked_spelling() {
    let mut t = TestEnv::new();
    let link = t.root.path().canonicalize().unwrap().join("home-link");
    symlink(&t.home, &link).unwrap();
    t.set("HOME", link.clone());
    t.dw_ok(&["init", "--name", "testbox"]);
    fs::write(t.home.join(".zshrc"), "z").unwrap();
    for spelled in [t.home.join(".zshrc"), link.join(".zshrc")] {
        let j = ok(
            &t,
            &["track", spelled.to_str().unwrap(), "--dry-run", "--json"],
        )
        .json();
        assert_eq!(j["items"][0]["item"]["path"], ".zshrc", "{spelled:?}");
    }
    // Symlinks *below* HOME are still not followed.
    fs::create_dir_all(t.home.join("real")).unwrap();
    fs::write(t.home.join("real/f"), "f").unwrap();
    symlink("real", t.home.join("via")).unwrap();
    assert_eq!(
        dw(&t, &["track", link.join("via/f").to_str().unwrap()]).code,
        2
    );
}

// 3. chezmoi's last-written state ---------------------------------------------------------------

#[test]
fn a_rolled_back_track_restores_chezmoi_state() {
    let t = setup();
    write(&t, ".kept", "k");
    ok(&t, &["track", ".kept", "--apply"]);
    write(&t, "bad/x", "x");
    let store = t.store();
    let mut items = dotwarden::track::plan_paths(
        &store,
        dotwarden::track::Plan::Track,
        &["bad".to_string()],
        &t.home,
        "human",
    )
    .unwrap();
    items[0].preview.files[0].source = "home/bogus".into();
    let before = entry_keys(&t);
    assert!(before.contains(&key(&t, ".kept")));
    let results = dotwarden::track::apply_now(&store, &items).unwrap();
    assert_eq!(results[0].status, dotwarden::track::Outcome::Failed);
    assert_eq!(
        entry_keys(&t),
        before,
        "chezmoi remembers nothing of the failed item"
    );
    assert_eq!(t.store().chezmoi(&["status"]).unwrap().stdout.trim(), "");
}

#[test]
fn untracking_cleans_the_state_of_files_and_of_folders_left_empty() {
    let t = setup();
    write(&t, "a/b/f", "f");
    write(&t, "a/g", "g");
    ok(&t, &["track", "a/b/f", "a/g", "--apply"]);
    for k in ["a", "a/b", "a/b/f", "a/g"] {
        assert!(entry_keys(&t).contains(&key(&t, k)), "{k} before");
    }
    ok(&t, &["untrack", "a/b/f", "--apply"]);
    let keys = entry_keys(&t);
    assert!(
        !keys.contains(&key(&t, "a/b/f")) && !keys.contains(&key(&t, "a/b")),
        "{keys:?}"
    );
    assert!(
        keys.contains(&key(&t, "a/g")) && keys.contains(&key(&t, "a")),
        "{keys:?}"
    );
    ok(&t, &["untrack", "a/g", "--apply"]);
    let keys = entry_keys(&t);
    assert!(keys.iter().all(|k| !k.contains("/a")), "{keys:?}");
    // Still consistent: tracking again works and chezmoi sees nothing to apply.
    ok(&t, &["track", "a/b/f", "--apply"]);
    assert_eq!(t.store().chezmoi(&["status"]).unwrap().stdout.trim(), "");
}

#[test]
fn state_cleanup_leaves_entries_of_paths_that_were_tracked_again_meanwhile() {
    let t = setup();
    write(&t, ".again", "x");
    ok(&t, &["track", ".again", "--apply"]);
    let store = t.store();
    let fake = dotwarden::track::ItemResult {
        id: None,
        kind: "untrack",
        summary: "x".into(),
        status: dotwarden::track::Outcome::Applied,
        message: None,
        staged: vec![],
        forgotten: vec![".again".into()],
    };
    let warnings = dotwarden::track::cleanup_state(&store, &[fake]);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(
        entry_keys(&t).contains(&key(&t, ".again")),
        "still tracked: keep its state"
    );
}

// 4. lock hold ---------------------------------------------------------------------------------

#[test]
fn planning_and_dry_runs_do_not_need_the_mutation_lock() {
    let mut t = setup();
    t.set("DOTWARDEN_LOCK_WAIT_MS", "300");
    write(&t, ".a", "a");
    let held = t.store().lock().unwrap();
    // Previews never wait for a running mutation ...
    let r = ok(&t, &["track", ".a", "--dry-run"]);
    assert!(r.out.contains("home/dot_a"));
    ok(&t, &["hide", "add", "/x/", "--dry-run"]);
    // ... only storing the item does, and says so.
    let r = dw(&t, &["track", ".a"]);
    assert_eq!(r.code, 1);
    assert!(r.err.contains("another dotwarden command"), "{}", r.err);
    drop(held);
    ok(&t, &["track", ".a"]);
}

#[test]
fn slow_planning_does_not_block_other_commands() {
    let mut t = setup();
    write(&t, ".slow", "s");
    // Planning (a dry run) is slow here: chezmoi sleeps before running.
    let fake = t.root.path().join("slow-plan");
    fs::create_dir_all(&fake).unwrap();
    symlink(common::libexec().join("git"), fake.join("git")).unwrap();
    fs::write(
        fake.join("chezmoi"),
        format!(
            "#!/bin/sh\nsleep 2\nexec {} \"$@\"\n",
            common::libexec().join("chezmoi").display()
        ),
    )
    .unwrap();
    fs::set_permissions(fake.join("chezmoi"), fs::Permissions::from_mode(0o755)).unwrap();
    let env = t.env.clone();
    let home = t.home.clone();
    let fake2 = fake.clone();
    let slow = std::thread::spawn(move || {
        Command::new(env!("CARGO_BIN_EXE_dotwarden"))
            .args(["track", ".slow"])
            .env_clear()
            .envs(&env)
            .env("DOTWARDEN_LIBEXEC", &fake2)
            .current_dir(&home)
            .output()
            .unwrap()
    });
    std::thread::sleep(std::time::Duration::from_millis(500));
    t.set("DOTWARDEN_LOCK_WAIT_MS", "600");
    let start = std::time::Instant::now();
    ok(&t, &["hide", "add", "/other/", "--apply"]);
    assert!(
        start.elapsed() < std::time::Duration::from_millis(1500),
        "{:?}",
        start.elapsed()
    );
    assert!(slow.join().unwrap().status.success());
    assert_eq!(pending_ids(&t), [1]);
}

#[test]
fn a_batch_releases_the_lock_between_items() {
    let t = setup();
    for f in [".a", ".b", ".c"] {
        write(&t, f, f);
    }
    ok(&t, &["track", ".a", ".b", ".c"]);
    let fake = slow_add_libexec(&t, "1.5");
    let env = t.env.clone();
    let home = t.home.clone();
    let batch = std::thread::spawn(move || {
        Command::new(env!("CARGO_BIN_EXE_dotwarden"))
            .args(["pending", "apply", "--all"])
            .env_clear()
            .envs(&env)
            .env("DOTWARDEN_LIBEXEC", &fake)
            .current_dir(&home)
            .output()
            .unwrap()
    });
    std::thread::sleep(std::time::Duration::from_millis(400));
    // The batch needs ~4.5 s; with the lock held throughout this would time out.
    let started = std::time::Instant::now();
    let r = dw_with(
        &t,
        &[("DOTWARDEN_LOCK_WAIT_MS", "3500")],
        &["hide", "add", "/y/", "--dry-run"],
    );
    assert_eq!(r.code, 0);
    let r = dw_with(
        &t,
        &[("DOTWARDEN_LOCK_WAIT_MS", "3500")],
        &["pending", "drop", "--dry-run", "--all"],
    );
    assert_eq!(r.code, 0, "{}", r.err);
    let r = dw_with(
        &t,
        &[("DOTWARDEN_LOCK_WAIT_MS", "3500")],
        &["hide", "add", "/y/"],
    );
    assert_eq!(r.code, 0, "waited {:?}: {}", started.elapsed(), r.err);
    assert!(started.elapsed() < std::time::Duration::from_millis(3500));
    let o = batch.join().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(managed(&t).len(), 3);
}

// 5. stale items are replaced -----------------------------------------------------------------------

#[test]
fn a_fresh_request_replaces_a_stale_pending_item() {
    let t = setup();
    write(&t, ".a", "one");
    ok(&t, &["track", ".a"]);
    // Not stale yet: an identical request changes nothing.
    let j = ok(&t, &["track", ".a", "--json"]).json();
    assert_eq!(j["added"][0]["already_pending"], true);
    write(&t, ".a", "changed since");
    assert!(pending_json(&t)["items"][0]["stale"].is_string());

    let j = ok(&t, &["track", ".a", "--json"]).json();
    assert_eq!(j["added"][0]["replaced"], true);
    assert_eq!(j["added"][0]["already_pending"], false);
    assert_eq!(j["added"][0]["item"]["id"], 1, "keeps its id");
    let list = pending_json(&t);
    assert_eq!(list["items"].as_array().unwrap().len(), 1);
    assert!(list["items"][0]["stale"].is_null());
    let human = ok(&t, &["pending", "apply", "--all"]);
    assert!(human.out.contains("applied"), "{}", human.out);
    assert_eq!(
        fs::read_to_string(repo(&t).join("home/dot_a")).unwrap(),
        "changed since"
    );
}

#[test]
fn pending_commands_without_ids_say_how_to_proceed() {
    let t = setup();
    write(&t, ".a", "a");
    ok(&t, &["track", ".a"]);
    for verb in ["apply", "drop"] {
        let r = dw(&t, &["pending", verb]);
        assert_eq!(r.code, 2);
        assert!(
            r.err.contains(&format!("dw pending {verb} --all")) && r.err.contains("1 pending item"),
            "{}",
            r.err
        );
    }
}

// 7. folder assignments --------------------------------------------------------------------------------

#[test]
fn untracking_a_folder_also_removes_the_assignment_of_the_folder_itself() {
    let t = setup();
    write(&t, ".config/app/a", "a");
    write(&t, ".config/other", "o");
    ok(&t, &["track", ".config/app", ".config/other", "--apply"]);
    for p in [".config/app", ".config/app/a", ".config/other"] {
        dotwarden::assign::set_assignment(&repo(&t), p, &["platform:linux".into()], &[]).unwrap();
    }
    t.git(&repo(&t), &["add", "-A"]);
    t.git(&repo(&t), &["commit", "-qm", "assign"]);
    let j = ok(&t, &["untrack", ".config/app", "--dry-run", "--json"]).json();
    let data = j["items"][0]["item"]["preview"]["data"].to_string();
    assert!(data.contains("assignment of ~/.config/app from"), "{data}");
    ok(&t, &["untrack", ".config/app", "--apply"]);
    let data = fs::read_to_string(repo(&t).join("home/.chezmoidata/dotwarden.toml")).unwrap();
    assert!(
        !data.contains("\".config/app\"") && !data.contains("\".config/app/a\""),
        "{data}"
    );
    assert!(
        data.contains("\".config/other\""),
        "unrelated assignment stays: {data}"
    );
}

// 8. content hash ------------------------------------------------------------------------------------------

#[test]
fn an_edit_with_the_same_size_and_restored_mtime_makes_the_item_stale() {
    let t = setup();
    write(&t, ".a", "AAAA");
    let mtime = fs::metadata(t.home.join(".a")).unwrap().modified().unwrap();
    ok(&t, &["track", ".a"]);
    fs::write(t.home.join(".a"), "BBBB").unwrap();
    fs::File::options()
        .write(true)
        .open(t.home.join(".a"))
        .unwrap()
        .set_modified(mtime)
        .unwrap();
    let meta = fs::metadata(t.home.join(".a")).unwrap();
    assert_eq!(
        (meta.len(), meta.modified().unwrap()),
        (4, mtime),
        "the edit is invisible to size and mtime"
    );
    let list = pending_json(&t);
    assert!(
        list["items"][0]["stale"]
            .as_str()
            .unwrap()
            .contains("~/.a in HOME"),
        "{list}"
    );
    let r = dw(&t, &["pending", "apply", "--all"]);
    assert_eq!(r.code, 7, "{}", r.out);
    assert_eq!(git_status(&t), "");
}
