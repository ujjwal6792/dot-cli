use dot_cli::{
    backend,
    config::Config,
    git,
    operations::{self, Action},
    sync,
};
use std::{fs, path::Path};

struct Fixture {
    _dir: tempfile::TempDir,
    c: Config,
    config: std::path::PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        let c = Config {
            home: p.join("home"),
            source: p.join("source"),
            state: p.join("state"),
            chezmoi_config: Some(p.join("chezmoi-config/chezmoi.toml")),
            machine: "fixture".into(),
            ..Default::default()
        };
        for d in [&c.home, &c.source, &c.state] {
            fs::create_dir_all(d).unwrap();
        }
        fs::create_dir_all(c.chezmoi_config.as_ref().unwrap().parent().unwrap()).unwrap();
        fs::write(c.chezmoi_config.as_ref().unwrap(), "").unwrap();
        git::run(&c, &["init", "-b", "main"]).unwrap();
        git::run(&c, &["config", "user.name", "Fixture"]).unwrap();
        git::run(&c, &["config", "user.email", "fixture@example.invalid"]).unwrap();
        git::run(&c, &["config", "commit.gpgsign", "false"]).unwrap();
        git::run(&c, &["config", "core.hooksPath", "/dev/null"]).unwrap();
        let config = p.join("dot.toml");
        c.save(&config).unwrap();
        Self {
            _dir: dir,
            c,
            config,
        }
    }
    fn write(&self, name: &str, text: &str) {
        let path = self.c.home.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    fn do_action(&mut self, a: Action, paths: &[&str]) {
        let paths: Vec<_> = paths.iter().map(|s| s.to_string()).collect();
        let p = operations::plan(&self.c, a, &paths, None).unwrap();
        let r = operations::execute(&mut self.c, &self.config, &p, &p.id).unwrap();
        assert!(r.failed.is_none(), "{}", r.message);
    }
    fn commit(&self) {
        git::run(&self.c, &["add", "--all"]).unwrap();
        git::run(&self.c, &["commit", "-m", "fixture"]).unwrap();
    }
}
#[test]
fn capture_stage_commit_restore_preserves_scope_and_index() {
    let mut f = Fixture::new();
    f.write(".one", "original\n");
    f.write(".two", "other\n");
    f.do_action(Action::Capture, &[".one", ".two"]);
    f.commit();
    f.do_action(Action::Enable, &[".one", ".two"]);
    f.write(".one", "changed\n");
    let p = operations::plan(&f.c, Action::Capture, &[".one".into()], None).unwrap();
    assert!(p.preview.contains("+changed"));
    assert!(p.preview.contains("-original"));
    let r = operations::execute(&mut f.c, &f.config, &p, &p.id).unwrap();
    assert!(r.failed.is_none());
    f.do_action(Action::Stage, &["dot_one"]);
    let p = operations::plan(&f.c, Action::Commit, &[], Some("change one".into())).unwrap();
    assert!(p.preview.contains("+changed"));
    assert!(
        operations::execute(&mut f.c, &f.config, &p, &p.id)
            .unwrap()
            .failed
            .is_none()
    );
    f.write(".one", "unwanted\n");
    f.do_action(Action::Restore, &[".one"]);
    assert_eq!(
        fs::read_to_string(f.c.home.join(".one")).unwrap(),
        "changed\n"
    );
    assert_eq!(f.c.enabled.len(), 2);
}
#[test]
fn exact_plan_invalidates_after_external_edit() {
    let mut f = Fixture::new();
    f.write(".one", "a");
    f.do_action(Action::Capture, &[".one"]);
    let p = operations::plan(&f.c, Action::Restore, &[".one".into()], None).unwrap();
    f.write(".one", "external");
    assert!(
        operations::execute(&mut f.c, &f.config, &p, &p.id)
            .unwrap_err()
            .to_string()
            .contains("changed after review")
    );
    assert_eq!(
        fs::read_to_string(f.c.home.join(".one")).unwrap(),
        "external"
    );
}
#[test]
fn empty_selection_never_applies_everything() {
    let f = Fixture::new();
    for action in [
        Action::Capture,
        Action::Apply,
        Action::Restore,
        Action::Forget,
        Action::Enable,
        Action::Stage,
    ] {
        assert!(
            operations::plan(&f.c, action, &[], None).is_err(),
            "{action:?}"
        );
    }
}
#[test]
fn template_capture_refuses_to_flatten_source() {
    let mut f = Fixture::new();
    f.write(".one", "local");
    fs::write(f.c.source.join("dot_one.tmpl"), "{{ .chezmoi.os }}\n").unwrap();
    assert!(
        operations::plan(&f.c, Action::Capture, &[".one".into()], None)
            .unwrap_err()
            .to_string()
            .contains("templated")
    );
    assert!(sync::strict_check(&f.c).is_err());
    f.do_action(Action::Restore, &[".one"]);
    assert!(fs::read_to_string(f.c.home.join(".one")).unwrap().contains(
        if cfg!(target_os = "macos") {
            "darwin"
        } else {
            "linux"
        }
    ));
}
#[test]
fn folder_rows_aggregate_and_untracked_stays_visible() {
    let mut f = Fixture::new();
    for i in 0..40 {
        f.write(&format!(".agents/skill{i}/SKILL.md"), "skill");
    }
    f.write(".config/app/config", "original");
    f.write(".untracked", "leave");
    f.do_action(Action::Capture, &[".agents", ".config"]);
    f.commit();
    f.write(".config/app/config", "changed");
    let s = backend::snapshot(&f.c).unwrap();
    let rows = backend::rows(&f.c, &s, "", backend::Filter::All, "").unwrap();
    assert_eq!(rows.iter().filter(|r| r.path == ".agents").count(), 1);
    assert_eq!(
        rows.iter().find(|r| r.path == ".agents").unwrap().children,
        40
    );
    assert!(rows.iter().find(|r| r.path == ".config").unwrap().changed);
    assert!(
        !rows
            .iter()
            .find(|r| r.path == ".untracked")
            .unwrap()
            .managed
    );
}
#[test]
fn restore_backup_refuses_later_edits_then_restores() {
    let mut f = Fixture::new();
    f.write(".one", "source");
    f.do_action(Action::Capture, &[".one"]);
    f.write(".one", "local");
    let p = operations::plan(&f.c, Action::Restore, &[".one".into()], None).unwrap();
    operations::execute(&mut f.c, &f.config, &p, &p.id).unwrap();
    operations::recover(&f.c, &p.id).unwrap();
    assert_eq!(fs::read_to_string(f.c.home.join(".one")).unwrap(), "local");
    let p = operations::plan(&f.c, Action::Restore, &[".one".into()], None).unwrap();
    operations::execute(&mut f.c, &f.config, &p, &p.id).unwrap();
    f.write(".one", "later");
    assert!(operations::recover(&f.c, &p.id).is_err());
}
#[test]
fn path_escape_and_symlink_parent_are_rejected() {
    let f = Fixture::new();
    assert!(f.c.target("../etc/passwd").is_err());
    assert!(f.c.target("/etc/passwd").is_err());
    std::os::unix::fs::symlink("/tmp", f.c.home.join("link")).unwrap();
    assert!(f.c.target("link/file").is_err());
}
#[test]
fn forget_keeps_home_and_enablement_is_incremental() {
    let mut f = Fixture::new();
    f.write(".one", "one");
    f.write(".two", "two");
    f.do_action(Action::Capture, &[".one", ".two"]);
    f.do_action(Action::Enable, &[".one"]);
    f.do_action(Action::Enable, &[".two"]);
    f.do_action(Action::Disable, &[".one"]);
    assert!(f.c.enabled.contains(".two"));
    f.do_action(Action::Forget, &[".one"]);
    assert!(f.c.home.join(".one").exists());
    assert!(!f.c.source.join("dot_one").exists());
}
#[test]
fn report_publication_does_not_touch_config_branch_or_index() {
    let mut f = Fixture::new();
    f.write(".one", "one");
    f.do_action(Action::Capture, &[".one"]);
    f.commit();
    let remote = f._dir.path().join("remote.git");
    std::process::Command::new("git")
        .args(["init", "--bare"])
        .arg(&remote)
        .output()
        .unwrap();
    git::run(&f.c, &["remote", "add", "origin", remote.to_str().unwrap()]).unwrap();
    git::run(&f.c, &["push", "-u", "origin", "main"]).unwrap();
    f.c.reporting = true;
    let head = git::run(&f.c, &["rev-parse", "HEAD"]).unwrap();
    let index = fs::read(f.c.source.join(".git/index")).unwrap();
    let report = sync::Report {
        installation: f.c.installation.clone(),
        revision: head.trim().into(),
        observed_at: chrono::Utc::now().to_rfc3339(),
        outcome: "succeeded".into(),
        enabled: 1,
        updated: 0,
    };
    sync::publish(&f.c, &report).unwrap();
    assert_eq!(git::run(&f.c, &["rev-parse", "HEAD"]).unwrap(), head);
    assert_eq!(fs::read(f.c.source.join(".git/index")).unwrap(), index);
    assert_eq!(sync::fetch_reports(&f.c).unwrap().len(), 1);
}
#[test]
fn strict_mode_rejects_hooks_before_execution() {
    let f = Fixture::new();
    let marker = f._dir.path().join("ran");
    fs::write(
        f.c.chezmoi_config.as_ref().unwrap(),
        format!(
            "[hooks.read-source-state.pre]\ncommand = 'touch'\nargs = ['{}']\n",
            marker.display()
        ),
    )
    .unwrap();
    assert!(sync::strict_check(&f.c).is_err());
    assert!(!marker.exists());
}
#[test]
fn source_switch_archives_before_changing_config_and_clears_trust() {
    let mut f = Fixture::new();
    f.write(".one", "one");
    f.do_action(Action::Capture, &[".one"]);
    let new = f._dir.path().join("new");
    fs::create_dir(&new).unwrap();
    std::process::Command::new("git")
        .arg("init")
        .arg(&new)
        .output()
        .unwrap();
    f.c.trusted = true;
    let message = dot_cli::ui::switch_source(&mut f.c, &f.config, &new).unwrap();
    assert!(message.contains("Archived"));
    assert!(!f.c.trusted);
    assert!(f.c.home.join(".one").exists());
    assert!(Path::new(&f.c.source).ends_with("new"));
}
#[test]
fn shared_os_and_machine_settings_are_rendered_by_chezmoi() {
    let mut f = Fixture::new();
    let key = "ghostty.font-size";
    let os = dot_cli::settings::identity(&f.c, dot_cli::settings::Scope::Os).unwrap();
    let hostname = dot_cli::settings::identity(&f.c, dot_cli::settings::Scope::Machine).unwrap();
    for (scope, identity, value) in [
        (dot_cli::settings::Scope::Shared, "shared".into(), "16"),
        (dot_cli::settings::Scope::Os, os, "17"),
        (dot_cli::settings::Scope::Machine, hostname.clone(), "18"),
    ] {
        let change = dot_cli::settings::Change {
            key: key.into(),
            scope,
            value: Some(value.into()),
            identity,
        };
        let p = operations::plan(
            &f.c,
            Action::Setting,
            &[],
            Some(serde_json::to_string(&change).unwrap()),
        )
        .unwrap();
        assert!(
            operations::execute(&mut f.c, &f.config, &p, &p.id)
                .unwrap()
                .failed
                .is_none()
        );
    }
    fs::write(f.c.source.join("dot_ghostty.tmpl"),"font-size = {{ template \"dot-setting\" (dict \"root\" . \"key\" \"ghostty.font-size\") }}\n").unwrap();
    f.do_action(Action::Apply, &[".ghostty"]);
    assert_eq!(
        fs::read_to_string(f.c.home.join(".ghostty")).unwrap(),
        "font-size = 18\n"
    );
    let change = dot_cli::settings::Change {
        key: key.into(),
        scope: dot_cli::settings::Scope::Machine,
        value: None,
        identity: hostname,
    };
    let p = operations::plan(
        &f.c,
        Action::Setting,
        &[],
        Some(serde_json::to_string(&change).unwrap()),
    )
    .unwrap();
    operations::execute(&mut f.c, &f.config, &p, &p.id).unwrap();
    f.do_action(Action::Apply, &[".ghostty"]);
    assert_eq!(
        fs::read_to_string(f.c.home.join(".ghostty")).unwrap(),
        "font-size = 17\n"
    );
}
#[test]
fn real_fast_forward_sync_updates_only_enabled_files() {
    let mut f = Fixture::new();
    f.write(".one", "old\n");
    f.write(".two", "other\n");
    f.do_action(Action::Capture, &[".one", ".two"]);
    f.commit();
    f.do_action(Action::Apply, &[".one", ".two"]);
    let remote = f._dir.path().join("remote.git");
    std::process::Command::new("git")
        .args(["init", "--bare"])
        .arg(&remote)
        .output()
        .unwrap();
    git::run(&f.c, &["remote", "add", "origin", remote.to_str().unwrap()]).unwrap();
    git::run(&f.c, &["push", "-u", "origin", "main"]).unwrap();
    let other = f._dir.path().join("other");
    std::process::Command::new("git")
        .args(["clone", "-b", "main"])
        .arg(&remote)
        .arg(&other)
        .output()
        .unwrap();
    let mut peer = f.c.clone();
    peer.source = other;
    git::run(&peer, &["config", "user.name", "Fixture"]).unwrap();
    git::run(&peer, &["config", "user.email", "fixture@example.invalid"]).unwrap();
    git::run(&peer, &["config", "commit.gpgsign", "false"]).unwrap();
    fs::write(peer.source.join("dot_one"), "new\n").unwrap();
    fs::write(peer.source.join("dot_two"), "not enabled\n").unwrap();
    git::run(&peer, &["add", "--all"]).unwrap();
    git::run(&peer, &["commit", "-m", "remote changes"]).unwrap();
    git::run(&peer, &["push"]).unwrap();
    f.c.enabled.insert(".one".into());
    f.c.sync_enabled = true;
    f.c.save(&f.config).unwrap();
    let result = sync::once(&mut f.c, &f.config).unwrap();
    assert_eq!(result.outcome, "succeeded", "{}", result.message);
    assert_eq!(fs::read_to_string(f.c.home.join(".one")).unwrap(), "new\n");
    assert_eq!(
        fs::read_to_string(f.c.home.join(".two")).unwrap(),
        "other\n"
    );
}

#[test]
fn migration_archives_and_holds_ambiguous_entries_without_touching_home() {
    let f = Fixture::new();
    let old = f._dir.path().join("old");
    fs::create_dir_all(old.join("home")).unwrap();
    fs::write(old.join("home/common"), "source value\n").unwrap();
    fs::write(old.join("manifest.json"),serde_json::to_vec(&serde_json::json!({"entries":{"common":{"source":"common","destination":".common","os":["darwin","linux"],"machines":{"include":["all"],"exclude":[]}},"ambiguous":{"source":"common","destination":".gitconfig","git_keys":["user.name"]}}})).unwrap()).unwrap();
    f.write(".common", "live untouched\n");
    let new = f._dir.path().join("converted");
    let p = dot_cli::migration::preview(&old, &new).unwrap();
    assert_eq!(p.entries.len(), 1);
    assert_eq!(p.held.len(), 1);
    dot_cli::migration::convert(&f.c, &old, &new).unwrap();
    assert_eq!(
        fs::read_to_string(new.join("dot_common")).unwrap(),
        "source value\n"
    );
    assert_eq!(
        fs::read_to_string(f.c.home.join(".common")).unwrap(),
        "live untouched\n"
    );
    assert!(f.c.state.join("archives").exists());
}
#[test]
fn config_changes_invalidate_plan_and_plan_tampering_is_rejected() {
    let mut f = Fixture::new();
    f.c.runtime_config = Some(f.config.clone());
    f.write(".one", "one");
    f.do_action(Action::Capture, &[".one"]);
    let mut p = operations::plan(&f.c, Action::Restore, &[".one".into()], None).unwrap();
    p.paths.push(".other".into());
    assert!(
        operations::execute(&mut f.c, &f.config, &p, &p.id)
            .unwrap_err()
            .to_string()
            .contains("Approval")
    );
    let p = operations::plan(&f.c, Action::Restore, &[".one".into()], None).unwrap();
    let original = fs::read_to_string(&f.config).unwrap();
    fs::write(&f.config, format!("{original}\n# external rule change\n")).unwrap();
    assert!(
        operations::execute(&mut f.c, &f.config, &p, &p.id)
            .unwrap_err()
            .to_string()
            .contains("changed after review")
    );
}

#[test]
fn partial_apply_stops_and_reports_exact_progress() {
    let mut f = Fixture::new();
    for name in [".a", ".b", ".c"] {
        f.write(name, "before\n");
    }
    f.do_action(Action::Capture, &[".a", ".b", ".c"]);
    f.do_action(Action::Apply, &[".a", ".b", ".c"]);
    for name in ["dot_a", "dot_b", "dot_c"] {
        fs::write(f.c.source.join(name), "after\n").unwrap();
    }
    f.write(".b", "human edit\n");
    let p = operations::plan(
        &f.c,
        Action::Apply,
        &[".a".into(), ".b".into(), ".c".into()],
        None,
    )
    .unwrap();
    let result = operations::execute(&mut f.c, &f.config, &p, &p.id).unwrap();
    assert_eq!(result.succeeded, vec![".a"]);
    assert_eq!(result.failed, Some(".b".into()));
    assert_eq!(result.not_attempted, vec![".c"]);
    assert_eq!(fs::read_to_string(f.c.home.join(".a")).unwrap(), "after\n");
    assert_eq!(
        fs::read_to_string(f.c.home.join(".b")).unwrap(),
        "human edit\n"
    );
    assert_eq!(fs::read_to_string(f.c.home.join(".c")).unwrap(), "before\n");
    assert_eq!(
        operations::history(&f.c).unwrap().last().unwrap().outcome,
        "partial"
    );
}
#[test]
fn source_lock_serializes_different_local_state_roots() {
    let f = Fixture::new();
    let held = operations::Lock::acquire(&f.c).unwrap();
    let mut other = f.c.clone();
    other.state = f._dir.path().join("other-state");
    assert!(operations::Lock::acquire(&other).is_err());
    drop(held);
    assert!(operations::Lock::acquire(&other).is_ok());
}
