//! Machines and assignments with real git and chezmoi: assigning, folder
//! assignments, rename, retire, preview as another machine, and the two-machine
//! round trip through a local bare remote.

mod common;

use std::fs;
use std::path::PathBuf;

use common::{TestEnv, snapshot};
use serde_json::Value;

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
    let p = t.home.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

fn repo(t: &TestEnv) -> PathBuf {
    t.dirs().repo()
}

/// A single machine `alpha` (darwin/macos) with a linux server `srv` registered.
fn alpha_and_srv() -> TestEnv {
    let t = TestEnv::new();
    t.dw_ok(&["init", "--name", "alpha"]);
    t.dw_ok(&[
        "machine",
        "add",
        "srv",
        "--platform",
        "linux",
        "--distribution",
        "ubuntu",
        "--family",
        "debian",
        "--apply",
    ]);
    t
}

/// The files `name` would receive.
fn receives(t: &TestEnv, name: &str) -> Vec<String> {
    t.dw_json(&["machine", "preview", name])["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap().to_string())
        .collect()
}

fn effects(r: &Run) -> Vec<String> {
    let v: Value = serde_json::from_str(&r.out).unwrap();
    v["items"][0]["item"]["preview"]["effects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e.as_str().unwrap().to_string())
        .collect()
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

#[test]
fn assignment_keeps_a_file_off_the_other_machine_after_pull_and_apply() {
    let a = TestEnv::new();
    a.dw_ok(&["init", "--name", "alpha"]);
    write(&a, ".base", "base\n");
    a.dw_ok(&["track", ".base", "--apply"]);
    a.dw_ok(&["commit", "-m", "base"]);
    let remote = bare(&a, "remote.git");
    a.dw_ok(&["remote", "add", "origin", remote.to_str().unwrap(), "--yes"]);
    a.dw_ok(&["push", "--yes"]);
    let b = TestEnv::new();
    b.dw_ok(&["clone", remote.to_str().unwrap(), "--name", "beta"]);
    b.dw_ok(&["push", "--yes"]);
    a.dw_ok(&["pull"]);

    write(&a, ".shared", "everywhere\n");
    write(&a, ".mine", "alpha only\n");
    a.dw_ok(&["track", ".shared", ".mine", "--apply"]);
    a.dw_ok(&["assign", ".mine", "--exclude", "machine:beta", "--apply"]);
    a.dw_ok(&["commit", "-m", "files"]);
    a.dw_ok(&["push", "--yes"]);

    b.dw_ok(&["pull"]);
    b.dw_ok(&["apply", "--all"]);
    assert_eq!(
        fs::read_to_string(b.home.join(".shared")).unwrap(),
        "everywhere\n"
    );
    assert!(
        !b.home.join(".mine").exists(),
        "the excluded file must not be written"
    );
    let status = b.dw_ok(&["status"]);
    assert!(!status.contains(".mine"), "{status}");
}

#[test]
fn exclude_wins_over_include() {
    let t = alpha_and_srv();
    t.dw_ok(&[
        "machine",
        "add",
        "mac2",
        "--platform",
        "darwin",
        "--distribution",
        "macos",
        "--apply",
    ]);
    write(&t, ".a", "a\n");
    t.dw_ok(&["track", ".a", "--apply"]);
    t.dw_ok(&[
        "assign",
        ".a",
        "--include",
        "platform:darwin",
        "--exclude",
        "machine:alpha",
        "--apply",
    ]);
    assert_eq!(receives(&t, "mac2"), [".a"]);
    assert!(
        receives(&t, "alpha").is_empty(),
        "darwin included but alpha excluded"
    );
    assert!(receives(&t, "srv").is_empty(), "not included");
}

#[test]
fn a_folder_assignment_covers_its_descendants_and_the_preview_names_them() {
    let t = alpha_and_srv();
    write(&t, "d/f", "f\n");
    write(&t, "d/sub/g", "g\n");
    write(&t, ".top", "t\n");
    t.dw_ok(&["track", "d", ".top", "--apply"]);

    let dry = dw(
        &t,
        &[
            "assign",
            "d",
            "--exclude",
            "platform:darwin",
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(dry.code, 0, "{}", dry.err);
    assert_eq!(
        effects(&dry),
        [
            "~/d/f: stops applying on alpha",
            "~/d/sub/g: stops applying on alpha"
        ]
    );
    t.dw_ok(&["assign", "d", "--exclude", "platform:darwin", "--apply"]);
    assert_eq!(receives(&t, "alpha"), [".top"]);
    assert_eq!(receives(&t, "srv"), [".top", "d/f", "d/sub/g"]);

    let back = dw(&t, &["assign", "d", "--clear", "--dry-run", "--json"]);
    assert_eq!(
        effects(&back),
        [
            "~/d/f: starts applying on alpha",
            "~/d/sub/g: starts applying on alpha"
        ]
    );
}

#[test]
fn targets_are_validated_against_the_registry() {
    let t = alpha_and_srv();
    write(&t, ".a", "a\n");
    write(&t, ".untracked", "u\n");
    t.dw_ok(&["track", ".a", "--apply"]);
    let unknown_machine = dw(&t, &["assign", ".a", "--include", "machine:ghost"]);
    assert_eq!(unknown_machine.code, 2, "{}", unknown_machine.err);
    assert!(unknown_machine.err.contains("not in the registry"));
    assert_eq!(
        dw(&t, &["assign", ".a", "--include", "platform:windows"]).code,
        2
    );
    let untracked = dw(&t, &["assign", ".untracked", "--include", "platform:linux"]);
    assert_eq!(untracked.code, 1);
    assert!(untracked.err.contains("not tracked"), "{}", untracked.err);

    let unseen = dw(
        &t,
        &[
            "assign",
            ".a",
            "--include",
            "distribution:fedora",
            "--dry-run",
        ],
    );
    assert_eq!(unseen.code, 0, "{}", unseen.err);
    assert!(
        unseen
            .out
            .contains("warning: no registered machine runs distribution"),
        "{}",
        unseen.out
    );
}

#[test]
fn odd_paths_are_assigned_exactly_through_dw_assign() {
    let t = alpha_and_srv();
    let odd = [".a[1]b", ".a1b", "#hash", "!bang", "sp ace ", "star*"];
    for f in odd {
        write(&t, f, "x\n");
    }
    let tracked: Vec<&str> = odd.to_vec();
    let mut args = vec!["track"];
    args.extend(&tracked);
    args.push("--apply");
    t.dw_ok(&args);
    for f in [".a[1]b", "#hash", "!bang", "sp ace ", "star*"] {
        t.dw_ok(&["assign", f, "--exclude", "machine:alpha", "--apply"]);
    }
    assert_eq!(receives(&t, "alpha"), [".a1b"], "only the sibling stays");
    assert_eq!(receives(&t, "srv").len(), odd.len());
}

#[test]
fn rename_updates_registry_targets_overrides_and_this_machine_and_the_other_machine_still_matches()
{
    let a = TestEnv::new();
    a.dw_ok(&["init", "--name", "alpha"]);
    write(&a, ".x", "x\n");
    write(&a, ".y", "y\n");
    a.dw_ok(&["track", ".x", ".y", "--apply"]);
    a.dw_ok(&["commit", "-m", "base"]);
    let remote = bare(&a, "remote.git");
    a.dw_ok(&["remote", "add", "origin", remote.to_str().unwrap(), "--yes"]);
    a.dw_ok(&["push", "--yes"]);
    let b = TestEnv::new();
    b.dw_ok(&["clone", remote.to_str().unwrap(), "--name", "beta"]);
    b.dw_ok(&["push", "--yes"]);
    a.dw_ok(&["pull"]);
    a.dw_ok(&["assign", ".x", "--exclude", "machine:alpha", "--apply"]);
    a.dw_ok(&["assign", ".y", "--include", "machine:beta", "--apply"]);
    // A hand-written variant override naming the machine.
    let data = repo(&a).join("home/.chezmoidata/dotwarden.toml");
    let mut text = fs::read_to_string(&data).unwrap();
    text.push_str(
        "\n[[dotwarden.variants.\".x\"]]\ntarget = \"machine:alpha\"\nfile = \"x/alpha\"\n",
    );
    fs::write(&data, text).unwrap();
    a.git(&repo(&a), &["add", "-A"]);
    a.dw_ok(&["commit", "-m", "assign"]);

    let dry = dw(&a, &["machine", "rename", "alpha", "mac", "--dry-run"]);
    assert!(
        dry.out.contains("variants / .x") && dry.out.contains("assignments / .x / exclude"),
        "{}",
        dry.out
    );
    a.dw_ok(&["machine", "rename", "alpha", "mac", "--apply"]);

    let list = a.dw_json(&["machine", "list"]);
    let names: Vec<(&str, bool)> = list["machines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            (
                m["name"].as_str().unwrap(),
                m["this_machine"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(names, [("beta", false), ("mac", true)]);
    let data = fs::read_to_string(&data).unwrap();
    assert!(
        data.contains("machine:mac") && !data.contains("machine:alpha"),
        "{data}"
    );
    assert!(
        fs::read_to_string(a.dirs().machine_toml())
            .unwrap()
            .contains("mac")
    );
    assert!(
        fs::read_to_string(a.dirs().chezmoi_config())
            .unwrap()
            .contains("\"mac\"")
    );
    assert!(receives(&a, "mac").is_empty(), "x excluded, y is beta only");
    assert_eq!(receives(&a, "beta"), [".x", ".y"]);
    a.dw_ok(&["status"]);

    a.dw_ok(&["commit", "-m", "rename"]);
    a.dw_ok(&["push", "--yes"]);
    b.dw_ok(&["pull"]);
    b.dw_ok(&["apply", "--all"]);
    assert!(b.home.join(".x").exists(), "beta was never excluded");
    assert!(
        b.home.join(".y").exists(),
        "machine:beta still matches after the rename"
    );
}

#[test]
fn retire_needs_a_human_removes_targets_and_cannot_hit_this_machine() {
    let t = alpha_and_srv();
    write(&t, ".a", "a\n");
    write(&t, ".b", "b\n");
    t.dw_ok(&["track", ".a", ".b", "--apply"]);
    t.dw_ok(&["assign", ".a", "--include", "machine:srv", "--apply"]);
    t.dw_ok(&["assign", ".b", "--exclude", "machine:srv", "--apply"]);
    assert_eq!(receives(&t, "alpha"), [".b"]);

    let before = fs::read_to_string(repo(&t).join("home/.chezmoidata/dotwarden.toml")).unwrap();
    let agent = run_with(&t, true, &["machine", "retire", "srv", "--yes"]);
    assert_eq!(agent.code, 3, "{}{}", agent.out, agent.err);
    assert_eq!(
        fs::read_to_string(repo(&t).join("home/.chezmoidata/dotwarden.toml")).unwrap(),
        before,
        "an agent changed nothing"
    );
    let listed = t.dw_json(&["pending", "list"]);
    assert_eq!(listed["items"][0]["kind"], "machine_retire");

    let this = dw(&t, &["machine", "retire", "alpha", "--yes"]);
    assert_eq!(this.code, 1);
    assert!(this.err.contains("this machine"), "{}", this.err);

    let dry = dw(&t, &["machine", "retire", "srv", "--dry-run"]);
    assert!(
        dry.out.contains("~/.a") && dry.out.contains("~/.b"),
        "{}",
        dry.out
    );
    assert!(
        dry.out.contains("~/.a: kept as the only include"),
        "{}",
        dry.out
    );
    assert!(
        dry.out.contains("~/.b: remove machine:srv from exclude"),
        "{}",
        dry.out
    );
    let done = dw(&t, &["machine", "retire", "srv", "--yes"]);
    assert_eq!(done.code, 0, "{}{}", done.out, done.err);
    let data = fs::read_to_string(repo(&t).join("home/.chezmoidata/dotwarden.toml")).unwrap();
    assert!(!data.contains("exclude"), "{data}");
    let list = t.dw_json(&["machine", "list"]);
    let srv = &list["machines"][1];
    assert_eq!(
        (
            srv["name"].as_str().unwrap(),
            srv["retired"].as_bool().unwrap()
        ),
        ("srv", true)
    );
    // Retired machines are not targets any more, and receive nothing.
    assert_eq!(
        dw(&t, &["assign", ".a", "--include", "machine:srv"]).code,
        2
    );
    assert_eq!(dw(&t, &["machine", "preview", "srv"]).code, 1);
    assert_eq!(receives(&t, "alpha"), [".b"], "only .b was freed");
}

#[test]
fn preview_as_a_machine_writes_nothing_and_diffs_against_home() {
    let t = alpha_and_srv();
    write(&t, ".a", "repo version\n");
    write(&t, ".s", "server only\n");
    t.dw_ok(&["track", ".a", ".s", "--apply"]);
    t.dw_ok(&["assign", ".s", "--include", "platform:linux", "--apply"]);
    t.dw_ok(&["commit", "-m", "base"]);
    write(&t, ".a", "local version\n");

    let before = snapshot(&t.home);
    let plain = t.dw_ok(&["machine", "preview", "srv"]);
    assert!(plain.contains("~/.s") && plain.contains("~/.a"), "{plain}");
    let diff = t.dw_json(&["machine", "preview", "srv", "--diff"]);
    let d = diff["diff"].as_str().unwrap();
    assert!(
        d.contains("+repo version") && d.contains("-local version"),
        "{d}"
    );
    let on_alpha = t.dw_json(&["machine", "preview", "alpha", "--diff"]);
    assert!(
        !on_alpha["diff"].as_str().unwrap().contains(".s"),
        "alpha never gets .s"
    );
    assert_eq!(
        snapshot(&t.home),
        before,
        "HOME, state and repository are untouched"
    );
    assert_eq!(
        fs::read_to_string(t.home.join(".a")).unwrap(),
        "local version\n"
    );
}

#[test]
fn agents_may_assign_and_a_stale_assignment_is_not_applied() {
    let t = alpha_and_srv();
    write(&t, ".a", "a\n");
    t.dw_ok(&["track", ".a", "--apply"]);
    let agent = run_with(&t, true, &["assign", ".a", "--include", "platform:linux"]);
    assert_eq!(agent.code, 0, "{}", agent.err);
    t.dw_ok(&[
        "assign",
        ".a",
        "--include",
        "platform:linux",
        "--exclude",
        "machine:srv",
        "--apply",
    ]);
    let applied = dw(&t, &["pending", "apply", "--all"]);
    assert_eq!(applied.code, 7, "{}", applied.out);
    assert!(
        applied.out.contains("assignment of ~/.a"),
        "{}",
        applied.out
    );
}

#[test]
fn machine_add_rename_and_duplicates_are_checked() {
    let t = alpha_and_srv();
    assert_eq!(
        dw(
            &t,
            &[
                "machine",
                "add",
                "srv",
                "--platform",
                "linux",
                "--distribution",
                "x"
            ]
        )
        .code,
        1
    );
    assert_eq!(
        dw(
            &t,
            &[
                "machine",
                "add",
                "Bad",
                "--platform",
                "linux",
                "--distribution",
                "x"
            ]
        )
        .code,
        2
    );
    assert_eq!(
        dw(
            &t,
            &[
                "machine",
                "add",
                "n",
                "--platform",
                "bsd",
                "--distribution",
                "x"
            ]
        )
        .code,
        2
    );
    assert_eq!(dw(&t, &["machine", "rename", "srv", "alpha"]).code, 1);
    assert_eq!(dw(&t, &["machine", "rename", "ghost", "x"]).code, 1);
    let list = dw(&t, &["machine", "list"]).out;
    assert!(
        list.contains("* alpha") && list.contains("srv") && list.contains("debian"),
        "{list}"
    );
}

#[test]
fn retiring_the_only_included_machine_leaves_the_file_on_no_machine() {
    let t = alpha_and_srv();
    write(&t, ".srvrc", "s\n");
    t.dw_ok(&["track", ".srvrc", "--apply"]);
    t.dw_ok(&["assign", ".srvrc", "--include", "machine:srv", "--apply"]);
    let dry = dw(&t, &["machine", "retire", "srv", "--dry-run"]);
    assert!(dry.out.contains("applies to no machine now"), "{}", dry.out);
    t.dw_ok(&["machine", "retire", "srv", "--yes"]);
    assert!(
        receives(&t, "alpha").is_empty(),
        "a server-only file must not widen"
    );
    let data = fs::read_to_string(repo(&t).join("home/.chezmoidata/dotwarden.toml")).unwrap();
    assert!(data.contains("machine:srv"), "{data}");
}

#[test]
fn a_machine_renamed_elsewhere_is_detected_and_machine_use_repoints_it() {
    let a = TestEnv::new();
    a.dw_ok(&["init", "--name", "alpha"]);
    write(&a, ".x", "x\n");
    a.dw_ok(&["track", ".x", "--apply"]);
    a.dw_ok(&["commit", "-m", "base"]);
    let remote = bare(&a, "remote.git");
    a.dw_ok(&["remote", "add", "origin", remote.to_str().unwrap(), "--yes"]);
    a.dw_ok(&["push", "--yes"]);
    let b = TestEnv::new();
    b.dw_ok(&["clone", remote.to_str().unwrap(), "--name", "beta"]);
    b.dw_ok(&["push", "--yes"]);
    a.dw_ok(&["pull"]);
    a.dw_ok(&["machine", "rename", "beta", "bee", "--apply"]);
    a.dw_ok(&["commit", "-m", "rename"]);
    a.dw_ok(&["push", "--yes"]);
    b.dw_ok(&["pull"]);

    let status = dw(&b, &["status"]);
    assert_eq!(status.code, 0, "status still works: {}", status.err);
    assert!(
        status.out.contains("beta is not in the machine registry"),
        "{}",
        status.out
    );
    let doctor = dw(&b, &["doctor"]);
    assert_ne!(doctor.code, 0);
    assert!(doctor.out.contains("dw machine use"), "{}", doctor.out);

    assert_eq!(run_with(&b, true, &["machine", "use", "bee"]).code, 3);
    assert_eq!(dw(&b, &["machine", "use", "ghost"]).code, 1);
    a.dw_ok(&[
        "machine",
        "add",
        "srv",
        "--platform",
        "linux",
        "--distribution",
        "ubuntu",
        "--apply",
    ]);
    b.dw_ok(&[
        "machine",
        "add",
        "srv",
        "--platform",
        "linux",
        "--distribution",
        "ubuntu",
        "--apply",
    ]);
    let wrong = dw(&b, &["machine", "use", "srv"]);
    assert_eq!(wrong.code, 1);
    assert!(wrong.err.contains("this computer is"), "{}", wrong.err);

    b.dw_ok(&["machine", "use", "bee"]);
    assert!(
        fs::read_to_string(b.dirs().machine_toml())
            .unwrap()
            .contains("bee")
    );
    assert!(
        !dw(&b, &["status"])
            .out
            .contains("not in the machine registry")
    );
    b.dw_ok(&["apply", "--all"]);
    assert!(b.home.join(".x").exists());
}
