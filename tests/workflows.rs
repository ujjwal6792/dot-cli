use dot_cli::{
    assignments::{self, Os, Scope},
    backend,
    config::Config,
    git,
    operations::{self, Action},
    sync,
};
use std::{fs, path::Path};

fn assign(f: &mut Fixture, path: &str, scope: &Scope) -> operations::Plan {
    let plan = operations::plan(
        &f.c,
        Action::Assign,
        &[path.into()],
        Some(serde_json::to_string(scope).unwrap()),
    )
    .unwrap();
    let outcome = operations::execute(&mut f.c, &f.config, &plan, &plan.id).unwrap();
    assert!(outcome.failed.is_none(), "{}", outcome.message);
    plan
}

fn managed_as(f: &Fixture, os: &str, hostname: &str) -> Vec<String> {
    let data = serde_json::json!({"chezmoi":{"os":os,"hostname":hostname}}).to_string();
    backend::chez(
        &f.c,
        &[
            "--override-data",
            &data,
            "managed",
            "--path-style=relative",
            "--include=files,symlinks",
            "-0",
        ],
    )
    .unwrap()
    .split('\0')
    .filter(|p| !p.is_empty())
    .map(str::to_owned)
    .collect()
}

#[test]
fn file_assignments_use_native_os_and_host_rules_preserve_copies_index_and_recover() {
    let mut f = Fixture::new();
    f.write(".one", "original");
    f.write(".two", "unrelated");
    f.do_action(Action::Capture, &[".one", ".two"]);
    f.commit();
    f.write(".two", "staged edit");
    f.do_action(Action::Capture, &[".two"]);
    f.do_action(Action::Stage, &["dot_two"]);
    let index = fs::read(f.c.source.join(".git/index")).unwrap();
    let scope = Scope {
        os: Os::Darwin,
        machines: ["dot-test-workstation".into(), "dot-test-desktop".into()].into(),
    };
    let plan = operations::plan(
        &f.c,
        Action::Assign,
        &[".one".into()],
        Some(serde_json::to_string(&scope).unwrap()),
    )
    .unwrap();
    assert!(
        !f.c.source.join(".chezmoiignore").exists(),
        "planning modified source"
    );
    assert!(plan.preview.contains("OS: All → Darwin"));
    let outcome = operations::execute(&mut f.c, &f.config, &plan, &plan.id).unwrap();
    assert!(outcome.failed.is_none(), "{}", outcome.message);
    for (os, hostname, allowed) in [
        ("darwin", "dot-test-workstation", true),
        ("darwin", "dot-test-desktop", true),
        ("linux", "dot-test-workstation", false),
        ("darwin", "unassigned", false),
        ("windows", "dot-test-workstation", false),
    ] {
        let managed = managed_as(&f, os, hostname);
        assert_eq!(managed.contains(&".one".into()), allowed, "{os}/{hostname}");
        assert!(managed.contains(&".two".into()));
    }
    assert_eq!(fs::read(f.c.source.join(".git/index")).unwrap(), index);
    assert_eq!(
        fs::read_to_string(f.c.source.join("dot_one")).unwrap(),
        "original"
    );
    assert_eq!(
        fs::read_to_string(f.c.home.join(".one")).unwrap(),
        "original"
    );
    let snapshot = backend::snapshot(&f.c).unwrap();
    assert!(snapshot.scoped_out.contains_key(".one"));
    let rows = backend::rows(&f.c, &snapshot, "", backend::Filter::Managed, "").unwrap();
    let row = rows.iter().find(|r| r.path == ".one").unwrap();
    assert!(row.managed);
    assert_eq!(row.status, "OFF");
    for action in [
        Action::Apply,
        Action::Restore,
        Action::Capture,
        Action::Trash,
    ] {
        assert!(
            operations::plan(&f.c, action, &[".one".into()], None).is_err(),
            "{action:?}"
        );
    }
    let reset = assign(&mut f, ".one", &Scope::default());
    assert!(managed_as(&f, "linux", "any-machine").contains(&".one".into()));
    operations::recover(&f.c, &reset.id).unwrap();
    assert_eq!(assignments::read(&f.c).unwrap()[".one"], scope);
    assert!(!managed_as(&f, "linux", "any-machine").contains(&".one".into()));
    assert_eq!(fs::read(f.c.source.join(".git/index")).unwrap(), index);
}

#[test]
fn folder_assignments_handle_source_roots_attributes_templates_and_literal_names() {
    let mut f = Fixture::new();
    fs::create_dir(f.c.source.join("settings")).unwrap();
    fs::write(f.c.source.join(".chezmoiroot"), "settings\n").unwrap();
    let source =
        f.c.source
            .join("settings/private_dot_config/private_app[dev]");
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join("private_space name.conf.tmpl"),
        "{{ .chezmoi.os }}\n",
    )
    .unwrap();
    fs::write(
        f.c.source.join("settings/.chezmoiignore.tmpl"),
        "# keep custom rules\nother/**\n",
    )
    .unwrap();
    let scope = Scope {
        os: Os::Linux,
        ..Default::default()
    };
    assign(&mut f, ".config/app[dev]", &scope);
    let rule = fs::read_to_string(f.c.source.join("settings/.chezmoiignore.tmpl")).unwrap();
    assert!(rule.starts_with("# keep custom rules\nother/**\n"));
    assert!(!f.c.source.join(".chezmoiignore").exists());
    let target = ".config/app[dev]/space name.conf";
    assert!(managed_as(&f, "linux", "laptop").contains(&target.into()));
    assert!(!managed_as(&f, "darwin", "laptop").contains(&target.into()));
    assert_eq!(
        fs::read_to_string(source.join("private_space name.conf.tmpl")).unwrap(),
        "{{ .chezmoi.os }}\n"
    );
    // A more-specific rule adds a restriction; clearing it preserves the parent rule.
    assign(
        &mut f,
        target,
        &Scope {
            machines: ["workstation".into()].into(),
            ..Default::default()
        },
    );
    assert!(!managed_as(&f, "linux", "laptop").contains(&target.into()));
    assert!(managed_as(&f, "linux", "workstation").contains(&target.into()));
    assign(&mut f, target, &Scope::default());
    assert!(managed_as(&f, "linux", "laptop").contains(&target.into()));
    assert!(!managed_as(&f, "darwin", "laptop").contains(&target.into()));
}

#[test]
fn assignments_refuse_native_include_conflicts_and_stale_reviews_without_writes() {
    let mut f = Fixture::new();
    f.write(".one", "original");
    f.do_action(Action::Capture, &[".one"]);
    let path = f.c.source.join(".chezmoiignore");
    fs::write(&path, "# custom\n!.one\n").unwrap();
    let scope = Scope {
        os: Os::Darwin,
        ..Default::default()
    };
    let error = operations::plan(
        &f.c,
        Action::Assign,
        &[".one".into()],
        Some(serde_json::to_string(&scope).unwrap()),
    )
    .unwrap_err();
    assert!(error.to_string().contains("include (!)"), "{error:#}");
    assert_eq!(fs::read_to_string(&path).unwrap(), "# custom\n!.one\n");
    fs::write(&path, "# custom\n").unwrap();
    let plan = operations::plan(
        &f.c,
        Action::Assign,
        &[".one".into()],
        Some(serde_json::to_string(&scope).unwrap()),
    )
    .unwrap();
    fs::write(&path, "# external change\n").unwrap();
    assert!(operations::execute(&mut f.c, &f.config, &plan, &plan.id).is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), "# external change\n");
    assert_eq!(
        fs::read_to_string(f.c.home.join(".one")).unwrap(),
        "original"
    );
}

#[test]
fn assignments_preserve_symlink_controls_and_manually_edited_blocks() {
    let mut f = Fixture::new();
    f.write(".one", "original");
    f.do_action(Action::Capture, &[".one"]);
    let outside = f._dir.path().join("external-ignore");
    fs::write(&outside, "# keep this file\n").unwrap();
    let control = f.c.source.join(".chezmoiignore");
    std::os::unix::fs::symlink(&outside, &control).unwrap();
    let scope = Scope {
        os: Os::Darwin,
        ..Default::default()
    };
    assert!(
        operations::plan(
            &f.c,
            Action::Assign,
            &[".one".into()],
            Some(serde_json::to_string(&scope).unwrap())
        )
        .is_err()
    );
    assert_eq!(fs::read_to_string(&outside).unwrap(), "# keep this file\n");
    assert!(
        fs::symlink_metadata(&control)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    fs::remove_file(&control).unwrap();
    assign(&mut f, ".one", &scope);
    let edited = fs::read_to_string(&control)
        .unwrap()
        .replace("ne .chezmoi.os \"darwin\"", "ne .chezmoi.os \"linux\"");
    fs::write(&control, &edited).unwrap();
    assert!(
        assignments::read(&f.c)
            .unwrap_err()
            .to_string()
            .contains("edited manually")
    );
    assert_eq!(fs::read_to_string(&control).unwrap(), edited);
    assert_eq!(
        Scope::parse_machines("laptop.local, desktop.company, laptop").unwrap(),
        ["laptop".into(), "desktop".into()].into()
    );
    assert!(Scope::parse_machines("{{ bad }}").is_err());
    assert!(Scope::parse_machines("laptop,,desktop").is_err());
}

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
    // An intentional assignment exclusion skips an enabled file, including when HOME lacks it.
    assign(
        &mut f,
        ".two",
        &Scope {
            machines: ["dot-test-other-machine".into()].into(),
            ..Default::default()
        },
    );
    f.commit();
    git::run(&f.c, &["push"]).unwrap();
    f.c.enabled.insert(".two".into());
    f.c.trusted = true;
    f.c.save(&f.config).unwrap();
    fs::remove_file(f.c.home.join(".two")).unwrap();
    let result = sync::once(&mut f.c, &f.config).unwrap();
    assert_eq!(result.outcome, "succeeded", "{}", result.message);
    assert!(!f.c.home.join(".two").exists());
    // If a remote change makes it apply here again, first application needs a review.
    git::run(&peer, &["pull", "--ff-only"]).unwrap();
    let plan = operations::plan(
        &peer,
        Action::Assign,
        &[".two".into()],
        Some(serde_json::to_string(&Scope::default()).unwrap()),
    )
    .unwrap();
    assert!(
        operations::execute(&mut peer, &f.config, &plan, &plan.id)
            .unwrap()
            .failed
            .is_none()
    );
    git::run(&peer, &["add", "--all"]).unwrap();
    git::run(&peer, &["commit", "-m", "share file with all machines"]).unwrap();
    git::run(&peer, &["push"]).unwrap();
    let result = sync::once(&mut f.c, &f.config).unwrap();
    assert_eq!(result.outcome, "paused");
    assert!(
        result.message.contains("now applies to this machine"),
        "{}",
        result.message
    );
    assert!(!f.c.home.join(".two").exists());
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

#[test]
fn symlinked_binary_uses_its_bundled_chezmoi() {
    use std::os::unix::{fs::PermissionsExt, fs::symlink};
    let f = Fixture::new();
    let base = f._dir.path();
    let bundle = base.join("releases/current-version");
    fs::create_dir_all(bundle.join("libexec")).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_dot"), bundle.join("dot")).unwrap();
    let private = bundle.join("libexec/chezmoi");
    fs::write(
        &private,
        "#!/bin/sh\necho 'chezmoi version v2.73.1, bundled fixture'\n",
    )
    .unwrap();
    fs::set_permissions(&private, fs::Permissions::from_mode(0o755)).unwrap();
    symlink(&bundle, base.join("current")).unwrap();
    fs::create_dir(base.join("bin")).unwrap();
    symlink(base.join("current/dot"), base.join("bin/dot")).unwrap();
    let result = std::process::Command::new(base.join("bin/dot"))
        .arg("--config")
        .arg(&f.config)
        .arg("health")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output = String::from_utf8(result.stdout).unwrap();
    let config: serde_json::Value =
        serde_json::from_str(&output[output.find('{').unwrap()..]).unwrap();
    assert_eq!(
        Path::new(config["chezmoi"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        private.canonicalize().unwrap()
    );
    assert!(output.contains("bundled fixture"));
}

#[test]
fn developer_defaults_are_native_editable_and_skip_generated_files() {
    let mut f = Fixture::new();
    dot_cli::ignores::install_defaults(&f.c).unwrap();
    let names = [
        ".DS_Store",
        "node_modules",
        "target",
        "__pycache__",
        ".venv",
        "dist",
        "build",
        ".next",
        ".gradle",
        ".build",
        ".dart_tool",
        ".terraform",
        ".Trash-1000",
        "._settings",
        "thing.swp",
    ];
    for name in names {
        f.write(&format!("project/deep/{name}/junk"), "generated");
        fs::create_dir_all(f.c.source.join(format!("junk/{name}"))).unwrap();
        fs::write(f.c.source.join(format!("junk/{name}/file")), "junk").unwrap();
    }
    f.write("project/settings.toml", "keep");
    f.write("project/target.rs", "keep");
    f.write("project/package-lock.json", "keep");
    let p = operations::plan(&f.c, Action::Capture, &["project".into()], None).unwrap();
    assert_eq!(
        p.paths,
        [
            "project/package-lock.json",
            "project/settings.toml",
            "project/target.rs"
        ]
    );
    assert!(
        operations::execute(&mut f.c, &f.config, &p, &p.id)
            .unwrap()
            .failed
            .is_none()
    );
    assert!(
        !git::status(&f.c)
            .unwrap()
            .files
            .iter()
            .any(|p| p.path.starts_with("junk/"))
    );
    assert!(
        operations::plan(
            &f.c,
            Action::Capture,
            &["project/deep/target/junk".into()],
            None
        )
        .is_err()
    );
    // Native exceptions work: defaults are editable rules, not a second hard-coded filter.
    let file = f.c.source.join(".chezmoiignore");
    let mut rules = fs::read_to_string(&file).unwrap();
    rules.push_str("\n!project/deep/target\n!project/deep/target/**\n");
    fs::write(file, rules).unwrap();
    f.do_action(Action::Capture, &["project/deep/target/junk"]);
    assert!(f.c.source.join("project/deep/target/junk").exists());
}

#[test]
fn capture_respects_native_templated_nested_rules_and_exceptions() {
    let mut f = Fixture::new();
    for name in ["keep", "skip", "local", "other"] {
        f.write(&format!("app/{name}"), "setting");
    }
    fs::create_dir_all(f.c.source.join("app")).unwrap();
    fs::write(
        f.c.source.join(".chezmoiignore"),
        "{{ if .chezmoi.os }}\napp/*\n!app/keep\n!app/other\n{{ end }}\n",
    )
    .unwrap();
    fs::write(f.c.source.join("app/.chezmoiignore"), "local\n").unwrap();
    let p = operations::plan(&f.c, Action::Capture, &["app".into()], None).unwrap();
    assert_eq!(p.paths, ["app/keep", "app/other"]);
    f.do_action(Action::Capture, &["app"]);
    assert!(!f.c.source.join("app/skip").exists());
    assert!(operations::plan(&f.c, Action::Capture, &["app/skip".into()], None).is_err());
}

#[test]
fn ignore_folder_is_reviewed_recoverable_and_keeps_all_originals() {
    let mut f = Fixture::new();
    f.write("app/cache/file", "original");
    f.write("app/keep", "settings");
    f.do_action(Action::Capture, &["app"]);
    f.commit();
    let p = operations::plan(&f.c, Action::Ignore, &["app/cache".into()], None).unwrap();
    assert_eq!(p.paths, ["app/cache"]);
    assert!(p.preview.contains("+app/cache\n+app/cache/**"));
    assert!(!f.c.source.join(".chezmoiignore").exists());
    assert!(operations::execute(&mut f.c, &f.config, &p, "wrong approval").is_err());
    assert!(
        operations::execute(&mut f.c, &f.config, &p, &p.id)
            .unwrap()
            .failed
            .is_none()
    );
    assert_eq!(
        fs::read_to_string(f.c.home.join("app/cache/file")).unwrap(),
        "original"
    );
    assert!(f.c.source.join("app/cache/file").exists());
    assert!(
        !backend::snapshot(&f.c)
            .unwrap()
            .managed
            .contains_key("app/cache/file")
    );
    assert_eq!(
        operations::plan(&f.c, Action::Capture, &["app".into()], None)
            .unwrap()
            .paths,
        ["app/keep"]
    );
    operations::recover(&f.c, &p.id).unwrap();
    assert!(!f.c.source.join(".chezmoiignore").exists());
    assert!(!f.c.source.join(".gitignore").exists());
    assert!(
        backend::snapshot(&f.c)
            .unwrap()
            .managed
            .contains_key("app/cache/file")
    );
}

#[test]
fn ignore_rules_escape_exact_paths_and_preserve_templates() {
    let mut f = Fixture::new();
    let name = "app/a [b]*? {c}! file";
    f.write(name, "ignored");
    f.write("app/neighbour", "keep");
    fs::write(
        f.c.source.join(".chezmoiignore.tmpl"),
        "# existing\n{{ if eq .chezmoi.os \"none\" }}\nother\n{{ end }}",
    )
    .unwrap();
    f.do_action(Action::Ignore, &[name]);
    let rules = fs::read_to_string(f.c.source.join(".chezmoiignore.tmpl")).unwrap();
    assert!(rules.starts_with("# existing\n{{ if"));
    assert!(!f.c.source.join(".chezmoiignore").exists());
    let p = operations::plan(&f.c, Action::Capture, &["app".into()], None).unwrap();
    assert_eq!(p.paths, ["app/neighbour"]);
    for unsafe_name in ["app/new\nrule", "app/{{ .chezmoi.os }}", "app/x # comment"] {
        f.write(unsafe_name, "untouched");
        assert!(operations::plan(&f.c, Action::Ignore, &[unsafe_name.into()], None).is_err());
    }
    assert!(
        operations::plan(&f.c, Action::Ignore, &[name.into()], None)
            .unwrap_err()
            .to_string()
            .contains("already exist")
    );
}

#[test]
fn one_ignore_action_maps_dotfiles_and_untracks_git_without_deleting() {
    let mut f = Fixture::new();
    f.write(".config/app/one", "original");
    f.do_action(Action::Capture, &[".config"]);
    f.commit();
    let p = operations::plan(&f.c, Action::Ignore, &[".config/app/one".into()], None).unwrap();
    assert!(p.preview.contains("/dot_config/app/one"));
    assert!(p.preview.contains("Stop Git tracking"));
    assert!(
        operations::execute(&mut f.c, &f.config, &p, &p.id)
            .unwrap()
            .failed
            .is_none()
    );
    assert!(
        !git::run(&f.c, &["ls-files"])
            .unwrap()
            .contains("dot_config/app/one")
    );
    assert!(
        !backend::snapshot(&f.c)
            .unwrap()
            .managed
            .contains_key(".config/app/one")
    );
    assert!(f.c.home.join(".config/app/one").exists());
    assert!(f.c.source.join("dot_config/app/one").exists());
    assert!(f.c.source.join(".chezmoiignore").exists());
    assert!(f.c.source.join(".gitignore").exists());
    operations::recover(&f.c, &p.id).unwrap();
    assert!(
        git::run(&f.c, &["ls-files"])
            .unwrap()
            .contains("dot_config/app/one")
    );
    assert!(git::status(&f.c).unwrap().files.is_empty());
}

#[test]
fn ignore_from_git_maps_native_attributes_and_keeps_unrelated_staging() {
    let mut f = Fixture::new();
    fs::create_dir_all(f.c.source.join("private_dot_config")).unwrap();
    fs::write(
        f.c.source.join("private_dot_config/private_secret.tmpl"),
        "original",
    )
    .unwrap();
    fs::write(f.c.source.join("unrelated"), "before").unwrap();
    f.commit();
    fs::write(f.c.source.join("unrelated"), "after").unwrap();
    git::run(&f.c, &["add", "unrelated"]).unwrap();
    let p = operations::plan(
        &f.c,
        Action::Ignore,
        &["private_dot_config/private_secret.tmpl".into()],
        Some("source".into()),
    )
    .unwrap();
    assert_eq!(p.paths, [".config/secret"]);
    assert!(
        p.preview
            .contains("+/private_dot_config/private_secret.tmpl")
    );
    assert!(
        operations::execute(&mut f.c, &f.config, &p, &p.id)
            .unwrap()
            .failed
            .is_none()
    );
    assert!(
        git::run(&f.c, &["diff", "--cached"])
            .unwrap()
            .contains("+after")
    );
    assert!(!git::run(&f.c, &["ls-files"]).unwrap().contains("secret"));
}

#[test]
fn defaults_preserve_existing_rules_and_are_idempotent() {
    let f = Fixture::new();
    fs::write(
        f.c.source.join(".chezmoiignore.tmpl"),
        "# mine\n{{ if eq .chezmoi.os \"linux\" }}\nlinux-only\n{{ end }}\n!keep\n",
    )
    .unwrap();
    fs::write(f.c.source.join(".gitignore"), "# mine\n*.custom\n!keep\n").unwrap();
    dot_cli::ignores::install_defaults(&f.c).unwrap();
    let chez = fs::read_to_string(f.c.source.join(".chezmoiignore.tmpl")).unwrap();
    let git = fs::read_to_string(f.c.source.join(".gitignore")).unwrap();
    assert!(
        chez.ends_with("# mine\n{{ if eq .chezmoi.os \"linux\" }}\nlinux-only\n{{ end }}\n!keep\n")
    );
    assert!(git.ends_with("# mine\n*.custom\n!keep\n"));
    assert!(!f.c.source.join(".chezmoiignore").exists());
    dot_cli::ignores::install_defaults(&f.c).unwrap();
    assert_eq!(
        fs::read_to_string(f.c.source.join(".chezmoiignore.tmpl")).unwrap(),
        chez
    );
    assert_eq!(
        fs::read_to_string(f.c.source.join(".gitignore")).unwrap(),
        git
    );
}

#[test]
fn ignore_rejects_symlink_rule_files_and_stale_plans() {
    let mut f = Fixture::new();
    f.write(".one", "original");
    let p = operations::plan(&f.c, Action::Ignore, &[".one".into()], None).unwrap();
    fs::write(f.c.source.join(".chezmoiignore"), "# external edit\n").unwrap();
    assert!(
        operations::execute(&mut f.c, &f.config, &p, &p.id)
            .unwrap_err()
            .to_string()
            .contains("changed after review")
    );
    fs::remove_file(f.c.source.join(".chezmoiignore")).unwrap();
    std::os::unix::fs::symlink(f.c.home.join(".one"), f.c.source.join(".chezmoiignore")).unwrap();
    assert!(operations::plan(&f.c, Action::Ignore, &[".one".into()], None).is_err());
    assert_eq!(
        fs::read_to_string(f.c.home.join(".one")).unwrap(),
        "original"
    );
}

#[test]
fn ignore_conflicting_exception_rolls_back_both_files_and_keeps_index() {
    let mut f = Fixture::new();
    f.write(".one", "keep");
    f.do_action(Action::Capture, &[".one"]);
    fs::write(f.c.source.join(".chezmoiignore"), "!.one\n").unwrap();
    f.commit();
    let p = operations::plan(&f.c, Action::Ignore, &[".one".into()], None).unwrap();
    let outcome = operations::execute(&mut f.c, &f.config, &p, &p.id).unwrap();
    assert!(outcome.failed.is_some());
    assert!(outcome.message.contains("exception"));
    assert_eq!(
        fs::read_to_string(f.c.source.join(".chezmoiignore")).unwrap(),
        "!.one\n"
    );
    assert!(!f.c.source.join(".gitignore").exists());
    assert!(git::status(&f.c).unwrap().files.is_empty());
    assert!(git::run(&f.c, &["ls-files"]).unwrap().contains("dot_one"));
}

#[test]
fn defaults_and_ignore_respect_relocated_chezmoi_root() {
    let mut f = Fixture::new();
    fs::write(f.c.source.join(".chezmoiroot"), "settings\n").unwrap();
    fs::create_dir_all(f.c.source.join("settings")).unwrap();
    dot_cli::ignores::install_defaults(&f.c).unwrap();
    assert!(f.c.source.join("settings/.chezmoiignore").exists());
    assert!(f.c.source.join(".gitignore").exists());
    f.write(".config/keep", "keep");
    f.write(".config/node_modules/junk", "junk");
    f.do_action(Action::Capture, &[".config"]);
    assert!(f.c.source.join("settings/dot_config/keep").exists());
    assert!(!f.c.source.join("settings/dot_config/node_modules").exists());
    f.commit();
    f.do_action(Action::Ignore, &[".config/keep"]);
    assert!(
        fs::read_to_string(f.c.source.join(".gitignore"))
            .unwrap()
            .contains("/settings/dot_config/keep")
    );
    assert!(
        !git::run(&f.c, &["ls-files"])
            .unwrap()
            .contains("dot_config/keep")
    );
}

#[test]
fn ignore_finds_private_saved_copies_already_hidden_by_defaults() {
    let mut f = Fixture::new();
    f.write(".config/.cache/file", "cached");
    fs::create_dir_all(f.c.source.join("private_dot_config/private_dot_cache")).unwrap();
    fs::write(
        f.c.source
            .join("private_dot_config/private_dot_cache/private_file"),
        "cached",
    )
    .unwrap();
    f.commit();
    dot_cli::ignores::install_defaults(&f.c).unwrap();
    assert!(
        !backend::snapshot(&f.c)
            .unwrap()
            .managed
            .contains_key(".config/.cache/file")
    );
    f.do_action(Action::Ignore, &[".config/.cache"]);
    assert!(
        !git::run(&f.c, &["ls-files"])
            .unwrap()
            .contains("private_file")
    );
    assert!(f.c.home.join(".config/.cache/file").exists());
    assert!(
        f.c.source
            .join("private_dot_config/private_dot_cache/private_file")
            .exists()
    );
}

#[test]
fn defaults_preserve_leading_whitespace_trimming_templates() {
    let f = Fixture::new();
    f.write(".custom", "private");
    f.write(".keep", "settings");
    fs::write(
        f.c.source.join(".chezmoiignore"),
        "{{- if .chezmoi.os -}}\n.custom\n{{- end }}\n",
    )
    .unwrap();
    dot_cli::ignores::install_defaults(&f.c).unwrap();
    assert!(operations::plan(&f.c, Action::Capture, &[".custom".into()], None).is_err());
    assert!(operations::plan(&f.c, Action::Capture, &[".keep".into()], None).is_ok());
}

#[test]
fn machine_defaults_hide_runtime_but_keep_configs_and_skills() {
    let f = Fixture::new();
    let hidden = [
        ".Trash",
        ".trash",
        ".android",
        ".bob",
        ".bundle",
        ".cache",
        ".cargo",
        ".bun",
        ".rustup",
        ".ollama",
        "Library",
        "projects",
        "Downloads",
    ];
    for dir in hidden {
        f.write(&format!("{dir}/runtime"), "generated");
    }
    for path in [
        ".DS_Store",
        ".agents/.DS_Store",
        ".codex/auth.json",
        ".codex/sessions/session.json",
        ".claude/projects/session.json",
        ".config/nvim/.fastembed_cache/data",
        ".ssh/id_ed25519",
        ".local/state/app/data",
    ] {
        f.write(path, "generated");
    }
    let kept = [
        ".zshrc",
        ".gitconfig",
        ".cargo-config-example",
        ".codex/config.toml",
        ".claude/settings.json",
        ".agents/skills/example/SKILL.md",
        ".config/nvim/init.lua",
        ".ssh/config",
        ".local/bin/myscript",
    ];
    for path in kept {
        f.write(path, "settings");
    }
    dot_cli::ignores::install_defaults(&f.c).unwrap();
    let mut snapshot = backend::snapshot(&f.c).unwrap();
    let rows = backend::rows(&f.c, &snapshot, "", backend::Filter::All, "").unwrap();
    for name in hidden {
        assert!(!rows.iter().any(|r| r.path == name), "visible: {name}");
    }
    assert!(!rows.iter().any(|r| r.path == ".DS_Store"));
    backend::inspect_directory(&f.c, &mut snapshot, ".agents").unwrap();
    let rows = backend::rows(&f.c, &snapshot, ".agents", backend::Filter::All, "").unwrap();
    assert_eq!(
        rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
        ["skills"]
    );
    let p = operations::plan(&f.c, Action::Capture, &kept.map(str::to_string), None).unwrap();
    assert_eq!(p.paths.len(), kept.len());
    // The checked-in catalog is exactly what setup writes on a fresh installation.
    if let Ok(dir) = std::env::var("DOT_DEFAULTS_OUTPUT") {
        fs::create_dir_all(&dir).unwrap();
        for name in [".chezmoiignore", ".gitignore"] {
            fs::copy(f.c.source.join(name), Path::new(&dir).join(name)).unwrap();
        }
    } else {
        assert_eq!(
            fs::read_to_string(f.c.source.join(".chezmoiignore")).unwrap(),
            include_str!("../assets/defaults/.chezmoiignore")
        );
        assert_eq!(
            fs::read_to_string(f.c.source.join(".gitignore")).unwrap(),
            include_str!("../assets/defaults/.gitignore")
        );
    }
}

#[test]
fn new_defaults_upgrade_v1_without_overwriting_edited_rules() {
    let f = Fixture::new();
    let custom = "# dot: developer ignore defaults v1\n# user deliberately edited the old block\ncustom-output\n!keep\n";
    for name in [".chezmoiignore", ".gitignore"] {
        fs::write(f.c.source.join(name), custom).unwrap();
    }
    dot_cli::ignores::install_defaults(&f.c).unwrap();
    for name in [".chezmoiignore", ".gitignore"] {
        let rules = fs::read_to_string(f.c.source.join(name)).unwrap();
        assert!(rules.ends_with(custom));
        assert!(rules.contains("# dot: machine ignore defaults v2"));
        assert!(
            !rules.contains("**/node_modules"),
            "edited v1 must not be regenerated"
        );
    }
}
