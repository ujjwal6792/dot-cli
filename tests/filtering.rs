use dot_cli::{
    backend::{self, Filter},
    config::Config,
};
use std::fs;

fn fixture() -> (tempfile::TempDir, Config) {
    let temp = tempfile::tempdir().unwrap();
    let c = Config {
        home: temp.path().join("home"),
        source: temp.path().join("source"),
        state: temp.path().join("state"),
        chezmoi_config: Some(temp.path().join("chezmoi.toml")),
        ..Default::default()
    };
    for path in [&c.home, &c.source, &c.state] {
        fs::create_dir_all(path).unwrap();
    }
    fs::write(c.chezmoi_config.as_ref().unwrap(), "").unwrap();
    (temp, c)
}

#[test]
fn files_rows_hide_native_ignored_desktop_clutter() {
    let (_temp, c) = fixture();
    fs::write(
        c.source.join(".chezmoiignore"),
        "**/.DS_Store\n.Trash\n.Trash/**\n",
    )
    .unwrap();
    fs::write(c.home.join(".DS_Store"), "junk").unwrap();
    fs::create_dir(c.home.join(".Trash")).unwrap();
    fs::write(c.home.join("keep"), "wanted").unwrap();
    let snapshot = backend::snapshot(&c).unwrap();
    let rows = backend::rows(&c, &snapshot, "", Filter::All, "").unwrap();
    assert_eq!(
        rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
        ["keep"]
    );
}

#[test]
fn directory_inspection_preserves_native_templates_nested_rules_and_exceptions() {
    let (_temp, c) = fixture();
    fs::write(c.source.join(".chezmoidata.toml"), "hide = true\n").unwrap();
    fs::write(
        c.source.join(".chezmoiignore.tmpl"),
        "{{ if .hide }}**/*.tmp\n{{ end }}!work/keep.tmp\n",
    )
    .unwrap();
    fs::create_dir(c.source.join("work")).unwrap();
    fs::write(c.source.join("work/.chezmoiignore"), "nested-secret\n").unwrap();
    fs::create_dir(c.home.join("work")).unwrap();
    for name in [
        "hidden.tmp",
        "keep.tmp",
        "nested-secret",
        "visible",
        "line\nbreak",
    ] {
        fs::write(c.home.join("work").join(name), "test").unwrap();
    }
    let mut snapshot = backend::snapshot(&c).unwrap();
    // Navigation waits for background inspection before showing unmanaged files.
    assert!(
        backend::rows(&c, &snapshot, "work", Filter::All, "")
            .unwrap()
            .is_empty()
    );
    backend::inspect_directory(&c, &mut snapshot, "work").unwrap();
    let rows = backend::rows(&c, &snapshot, "work", Filter::All, "").unwrap();
    assert_eq!(
        rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
        ["keep.tmp", "line\nbreak", "visible"]
    );
    assert_eq!(
        backend::rows(&c, &snapshot, "work", Filter::All, "keep")
            .unwrap()
            .len(),
        1
    );
    // A refresh re-evaluates edited rules rather than retaining stale eligibility.
    fs::write(
        c.source.join("work/.chezmoiignore"),
        "nested-secret\nvisible\n",
    )
    .unwrap();
    backend::inspect_directory(&c, &mut snapshot, "work").unwrap();
    assert!(
        !backend::rows(&c, &snapshot, "work", Filter::All, "")
            .unwrap()
            .iter()
            .any(|row| row.name == "visible")
    );
}

#[test]
fn managed_and_missing_targets_survive_filtered_listing() {
    let (_temp, c) = fixture();
    fs::create_dir(c.source.join("dot_config")).unwrap();
    fs::write(c.source.join("dot_config/missing"), "saved").unwrap();
    fs::write(c.source.join("dot_hidden"), "saved").unwrap();
    fs::write(c.source.join(".chezmoiignore"), ".hidden\n").unwrap();
    fs::write(c.home.join(".hidden"), "ignored saved copy").unwrap();
    let snapshot = backend::snapshot(&c).unwrap();
    let rows = backend::rows(&c, &snapshot, "", Filter::All, "").unwrap();
    assert_eq!(
        rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
        [".config"]
    );
    assert!(rows[0].managed);
    assert_eq!(rows[0].children, 1);
    let rows = backend::rows(&c, &snapshot, ".config", Filter::Managed, "").unwrap();
    assert_eq!(rows[0].path, ".config/missing");
}

#[test]
fn startup_rows_do_not_require_native_commands() {
    let (_temp, mut c) = fixture();
    c.chezmoi = "/not/a/command".into();
    fs::write(c.home.join("visible"), "test").unwrap();
    let rows = backend::rows(&c, &Default::default(), "", Filter::All, "").unwrap();
    assert_eq!(rows[0].name, "visible");
}

#[test]
fn inspection_batches_only_immediate_unmanaged_targets_and_rows_never_spawn() {
    use std::os::unix::fs::PermissionsExt;
    let (temp, mut c) = fixture();
    fs::create_dir(c.source.join("managed")).unwrap();
    fs::create_dir(c.home.join("managed")).unwrap();
    fs::write(c.source.join("managed/tracked"), "saved").unwrap();
    fs::write(c.home.join("managed/tracked"), "saved").unwrap();
    fs::write(c.home.join("managed/uninspected"), "deep").unwrap();
    let mut snapshot = backend::snapshot(&c).unwrap();
    for i in 0..260 {
        fs::write(c.home.join(format!("item-{i}")), "test").unwrap();
    }
    let executable = std::process::Command::new("which")
        .arg(&c.chezmoi)
        .output()
        .unwrap();
    let native = String::from_utf8(executable.stdout).unwrap();
    assert!(executable.status.success());
    let log = temp.path().join("calls");
    let wrapper = temp.path().join("chezmoi-wrapper");
    // All shell inputs are temporary fixture paths or a discovered executable.
    let quote = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
    fs::write(&wrapper, format!("#!/bin/sh\nprintf '%s\\n' \"$@\" >> {}\nprintf '%s\\n' CALL_END >> {}\nexec {} \"$@\"\n", quote(&log.to_string_lossy()), quote(&log.to_string_lossy()), quote(native.trim()))).unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    c.chezmoi = wrapper.to_string_lossy().into_owned();
    backend::inspect_directory(&c, &mut snapshot, "").unwrap();
    let calls = fs::read_to_string(&log).unwrap();
    assert_eq!(calls.matches("CALL_END").count(), 3);
    assert!(
        !calls
            .lines()
            .any(|line| line == c.home.join("managed").to_string_lossy())
    );
    assert!(!calls.contains("uninspected"));
    assert_eq!(
        backend::rows(&c, &snapshot, "", Filter::All, "")
            .unwrap()
            .len(),
        261
    );
    backend::rows(&c, &snapshot, "", Filter::All, "item-2").unwrap();
    assert_eq!(fs::read_to_string(&log).unwrap(), calls);
}

#[test]
fn entering_ignored_parent_during_startup_never_exposes_children() {
    let (_temp, c) = fixture();
    fs::write(c.source.join(".chezmoiignore"), "hidden/\n").unwrap();
    fs::create_dir_all(c.home.join("hidden/deep")).unwrap();
    fs::write(c.home.join("hidden/deep/secret"), "test").unwrap();
    let mut snapshot = backend::snapshot(&c).unwrap();
    backend::inspect_directory(&c, &mut snapshot, "hidden/deep").unwrap();
    assert!(
        backend::rows(&c, &snapshot, "hidden/deep", Filter::All, "")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn nested_unmanaged_directories_stay_browseable() {
    let (_temp, c) = fixture();
    fs::create_dir_all(c.home.join("config/editor/themes")).unwrap();
    fs::write(c.home.join("config/editor/themes/theme"), "settings").unwrap();
    let mut snapshot = backend::snapshot(&c).unwrap();
    backend::inspect_directory(&c, &mut snapshot, "config/editor/themes").unwrap();
    let rows = backend::rows(&c, &snapshot, "config/editor/themes", Filter::All, "").unwrap();
    assert_eq!(
        rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
        ["theme"]
    );
}
