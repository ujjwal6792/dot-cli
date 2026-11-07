//! Agents (spec §8, §9): identity, approvals, the activity log, dry-runs and the
//! generated artefacts, with real git and chezmoi in isolated directories.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::TestEnv;

struct Run {
    code: i32,
}

fn run_env(t: &TestEnv, vars: &[(&str, &str)], args: &[&str]) -> Run {
    let o = std::process::Command::new(env!("CARGO_BIN_EXE_dotwarden"))
        .args(args)
        .env_clear()
        .envs(&t.env)
        .envs(vars.iter().copied())
        .current_dir(&t.home)
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    Run {
        code: o.status.code().unwrap(),
    }
}

fn repo(t: &TestEnv) -> PathBuf {
    t.dirs().repo()
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

fn machine_with_remote() -> (TestEnv, PathBuf) {
    let t = TestEnv::new();
    t.dw_ok(&["init", "--name", "alpha"]);
    fs::write(t.home.join(".a"), "one\n").unwrap();
    t.dw_ok(&["track", ".a", "--apply"]);
    t.dw_ok(&["commit", "-m", "base"]);
    let remote = bare(&t);
    t.dw_ok(&["remote", "add", "origin", remote.to_str().unwrap(), "--yes"]);
    t.dw_ok(&["push", "--yes"]);
    (t, remote)
}

#[test]
fn a_known_agent_environment_is_held_to_the_agent_rules_and_named() {
    let (t, remote) = machine_with_remote();
    let claude = [("CLAUDECODE", "1")];
    fs::write(t.home.join(".a"), "two\n").unwrap();
    t.dw_ok(&["stage", ".a"]);
    assert_eq!(run_env(&t, &claude, &["commit", "-m", "second"]).code, 0);

    let trailer = t.git(
        &repo(&t),
        &["log", "-1", "--format=%(trailers:key=Agent,valueonly)"],
    );
    assert_eq!(trailer.trim(), "claude-code");

    let before = t.git(&remote, &["rev-parse", "main"]);
    assert_eq!(run_env(&t, &claude, &["push", "--yes"]).code, 3);
    assert_eq!(
        t.git(&remote, &["rev-parse", "main"]),
        before,
        "nothing was pushed"
    );
    let pending = t.dw_ok(&["pending"]);
    assert!(pending.contains("requested by claude-code"), "{pending}");

    // The agent cannot approve its own request; the explicit name wins over detection.
    assert_eq!(run_env(&t, &claude, &["approve", "1"]).code, 3);
    let named = run_env(
        &t,
        &[("CLAUDECODE", "1"), ("DOTWARDEN_AGENT", "mine")],
        &["approve", "1"],
    );
    assert_eq!(named.code, 3);
    assert_eq!(t.git(&remote, &["rev-parse", "main"]), before);

    // Without a terminal a human has no way to confirm either; `pending apply --yes` does.
    assert_eq!(run_env(&t, &[], &["approve", "1"]).code, 3);
    t.dw_ok(&["pending", "apply", "1", "--yes"]);
    assert_ne!(t.git(&remote, &["rev-parse", "main"]), before);
}

#[test]
fn the_activity_log_names_who_ran_what_and_never_records_arguments() {
    let (t, _) = machine_with_remote();
    fs::write(t.home.join(".a"), "two\n").unwrap();
    t.dw_ok(&["stage", ".a"]);
    run_env(
        &t,
        &[("DOTWARDEN_AGENT", "bot")],
        &["commit", "-m", "TOPSECRET-message"],
    );
    run_env(&t, &[("DOTWARDEN_AGENT", "bot")], &["push", "--yes"]);
    t.dw_ok(&["status"]);
    t.dw_ok(&["pending"]);

    let all = t.dw_json(&["activity", "-n", "100"]);
    let rows: Vec<(String, String, i64)> = all
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["actor"].as_str().unwrap().to_string(),
                e["command"].as_str().unwrap().to_string(),
                e["exit"].as_i64().unwrap(),
            )
        })
        .collect();
    assert!(
        rows.contains(&("bot".into(), "commit".into(), 0)),
        "{rows:?}"
    );
    assert!(rows.contains(&("bot".into(), "push".into(), 3)), "{rows:?}");
    assert!(
        rows.contains(&("human".into(), "stage".into(), 0)),
        "{rows:?}"
    );
    assert!(
        !rows
            .iter()
            .any(|r| ["status", "pending", "activity"].contains(&r.1.as_str())),
        "reads are not logged: {rows:?}"
    );

    let bot = t.dw_json(&["activity", "--actor", "bot"]);
    assert!(bot.as_array().unwrap().iter().all(|e| e["actor"] == "bot"));

    let file = fs::read_to_string(t.dirs().state_dir().join("activity.jsonl")).unwrap();
    assert!(!file.contains("TOPSECRET"), "arguments must not be logged");
}

#[test]
fn dry_runs_of_fetch_pull_revert_and_machine_use_change_nothing() {
    let (a, remote) = machine_with_remote();
    let b = TestEnv::new();
    b.dw_ok(&["clone", remote.to_str().unwrap(), "--name", "beta"]);
    b.dw_ok(&["push", "--yes"]);

    // Machine A publishes a new commit that B has not fetched.
    a.dw_ok(&["pull"]);
    fs::write(a.home.join(".a"), "two\n").unwrap();
    a.dw_ok(&["stage", ".a"]);
    a.dw_ok(&["commit", "-m", "second"]);
    a.dw_ok(&["push", "--yes"]);

    let refs = |t: &TestEnv| {
        t.git(
            &repo(t),
            &["for-each-ref", "--format=%(refname) %(objectname)"],
        )
    };
    let head = |t: &TestEnv| t.git(&repo(t), &["rev-parse", "HEAD"]);
    let (refs_before, head_before) = (refs(&b), head(&b));

    let f = b.dw_json(&["fetch", "--dry-run"]);
    assert_eq!(f["would_fetch"][0], "origin");
    assert_eq!(refs(&b), refs_before, "fetch --dry-run must not fetch");

    b.dw_ok(&["fetch"]);
    let fetched = refs(&b);
    let p = b.dw_json(&["pull", "--dry-run"]);
    assert_eq!(p["plan"]["how"], "fast-forward");
    assert_eq!(p["plan"]["behind"], 1);
    assert_eq!(head(&b), head_before, "pull --dry-run must not move HEAD");
    assert_eq!(refs(&b), fetched);

    let r = a.dw_json(&["revert", "HEAD", "--dry-run"]);
    assert_eq!(r["dry_run"], true);
    assert!(r["files"].as_array().unwrap().len() == 1, "{r}");
    let count = a.git(&repo(&a), &["rev-list", "--count", "HEAD"]);
    a.dw_ok(&["revert", "HEAD", "--dry-run"]);
    assert_eq!(a.git(&repo(&a), &["rev-list", "--count", "HEAD"]), count);

    let local = fs::read_to_string(a.dirs().state_dir().join("machine.toml")).unwrap();
    let m = a.dw_json(&["machine", "use", "alpha", "--dry-run"]);
    assert_eq!(m["dry_run"], true);
    assert_eq!(
        fs::read_to_string(a.dirs().state_dir().join("machine.toml")).unwrap(),
        local
    );
}

#[test]
fn completions_man_pages_and_the_agent_skill_are_generated() {
    let t = TestEnv::new();
    assert!(t.dw_ok(&["completions", "zsh"]).contains("#compdef dw"));
    assert!(t.dw_ok(&["completions", "bash"]).contains("_dw"));

    let dir = t.root.path().join("man");
    t.dw_ok(&["man", "--dir", dir.to_str().unwrap()]);
    assert!(dir.join("dw.1").is_file());
    assert!(dir.join("dw-pending.1").is_file());

    let skill = t.dw_ok(&["skill"]);
    assert!(skill.starts_with("---\nname: dotwarden"));
    assert!(skill.contains("exit"), "documents the exit codes");

    let target = t.root.path().join("skills");
    t.dw_ok(&["skill", "--install", target.to_str().unwrap()]);
    let file: &Path = &target.join("dotwarden/SKILL.md");
    assert_eq!(fs::read_to_string(file).unwrap(), skill);
    fs::write(file, "mine").unwrap();
    assert_eq!(
        t.dw(&["skill", "--install", target.to_str().unwrap()])
            .status
            .code(),
        Some(1)
    );
    assert_eq!(
        fs::read_to_string(file).unwrap(),
        "mine",
        "a differing skill is kept"
    );
    t.dw_ok(&["skill", "--install", target.to_str().unwrap(), "--force"]);
    assert_eq!(fs::read_to_string(file).unwrap(), skill);
}
