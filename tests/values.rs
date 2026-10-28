//! Values with real git and chezmoi: variables with per-target overrides,
//! references in plain files and variants, reverse mapping on stage, detection,
//! with two machines sharing a local bare remote.

mod common;

use std::fs;
use std::path::PathBuf;

use common::TestEnv;

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

fn read(t: &TestEnv, rel: &str) -> String {
    fs::read_to_string(t.home.join(rel)).unwrap()
}

fn repo(t: &TestEnv) -> PathBuf {
    t.dirs().repo()
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

/// alpha (darwin/macos) and beta (a second computer registered as linux/ubuntu,
/// family debian) sharing a bare remote, both holding the tracked `.conf`.
fn pair() -> (TestEnv, TestEnv) {
    let a = TestEnv::new();
    a.dw_ok(&["init", "--name", "alpha"]);
    write(&a, ".conf", "font = Menlo\nsize = 12\n");
    a.dw_ok(&["track", ".conf", "--apply"]);
    a.dw_ok(&["commit", "-m", "conf"]);
    let remote = bare(&a, "remote.git");
    a.dw_ok(&["remote", "add", "origin", remote.to_str().unwrap(), "--yes"]);
    a.dw_ok(&["push", "--yes"]);
    let b = TestEnv::new();
    b.dw_ok(&["clone", remote.to_str().unwrap(), "--name", "beta"]);
    let file = repo(&b).join("home/.chezmoidata/dotwarden-machines/beta.toml");
    let text = fs::read_to_string(&file)
        .unwrap()
        .replace("\"darwin\"", "\"linux\"")
        .replace("\"macos\"", "\"ubuntu\"")
        .replace("families = []", "families = [\"debian\"]");
    fs::write(&file, text).unwrap();
    b.git(&repo(&b), &["add", "-A"]);
    b.dw_ok(&["commit", "-m", "beta is a debian-like linux"]);
    b.dw_ok(&["push", "--yes"]);
    b.dw_ok(&["apply", "--all"]);
    a.dw_ok(&["pull"]);
    (a, b)
}

fn sync(from: &TestEnv, to: &TestEnv) {
    from.dw_ok(&["push", "--yes"]);
    to.dw_ok(&["pull"]);
}

fn stored_conf(t: &TestEnv) -> String {
    fs::read_to_string(repo(t).join("home/dot_conf.tmpl")).unwrap()
}

fn font_ref() -> &'static str {
    "{{ includeTemplate \"dotwarden/value\" (dict \"data\" . \"name\" \"FONT\") }}"
}

/// FONT = Menlo, debian-like machines get Mono; `.conf` wired and committed on
/// alpha, synced to beta.
fn wired_pair() -> (TestEnv, TestEnv) {
    let (a, b) = pair();
    a.dw_ok(&["values", "set-variable", "FONT", "Menlo", "--apply"]);
    a.dw_ok(&[
        "values",
        "set-variable",
        "FONT",
        "Mono",
        "--target",
        "family:debian",
        "--apply",
    ]);
    a.dw_ok(&["values", "wire", ".conf", "FONT", "--apply"]);
    a.dw_ok(&["commit", "-m", "wire"]);
    sync(&a, &b);
    b.dw_ok(&["apply", "--all"]);
    (a, b)
}

#[test]
fn an_override_renders_a_different_value_on_each_machine() {
    let (a, b) = wired_pair();
    assert_eq!(read(&a, ".conf"), "font = Menlo\nsize = 12\n");
    assert_eq!(
        read(&b, ".conf"),
        "font = Mono\nsize = 12\n",
        "beta is debian-like"
    );
    assert!(
        !a.dw_ok(&["status"]).contains("Local"),
        "alpha holds what it renders"
    );
    assert!(!b.dw_ok(&["status"]).contains("Local"));

    // A machine-level override beats the family one, for that machine only.
    b.dw_ok(&[
        "values",
        "set-variable",
        "FONT",
        "Beta",
        "--target",
        "machine:beta",
        "--apply",
    ]);
    b.dw_ok(&["commit", "-m", "beta font"]);
    sync(&b, &a);
    assert!(
        b.dw_ok(&["values", "show", "FONT"])
            .contains("receives Beta (machine:beta)")
    );
    assert!(
        a.dw_ok(&["values", "show", "FONT"])
            .contains("receives Menlo (default)")
    );
    b.dw_ok(&["apply", "--all"]);
    assert_eq!(read(&b, ".conf"), "font = Beta\nsize = 12\n");
    a.dw_ok(&["apply", "--all"]);
    assert_eq!(read(&a, ".conf"), "font = Menlo\nsize = 12\n");
}

#[test]
fn wiring_stores_a_template_and_renders_the_same_file() {
    let (a, _b) = pair();
    a.dw_ok(&["values", "set-variable", "FONT", "Menlo", "--apply"]);
    let dry = a.dw_ok(&["values", "wire", ".conf", "FONT", "--dry-run"]);
    assert!(
        dry.contains("1 occurrence(s)") && dry.contains("home/dot_conf.tmpl"),
        "{dry}"
    );
    assert!(
        repo(&a).join("home/dot_conf").is_file(),
        "a dry run changes nothing"
    );
    a.dw_ok(&["values", "wire", ".conf", "FONT", "--apply"]);
    assert_eq!(
        stored_conf(&a),
        format!("font = {}\nsize = 12\n", font_ref())
    );
    assert!(!repo(&a).join("home/dot_conf").exists());
    assert_eq!(
        read(&a, ".conf"),
        "font = Menlo\nsize = 12\n",
        "HOME untouched"
    );
    assert!(
        !a.dw_ok(&["status"]).contains("Local"),
        "no reference-only noise"
    );
    assert!(a.dw_ok(&["values", "list"]).contains("used by ~/.conf"));
    let err = dw(&a, &["values", "wire", ".conf", "FONT"]);
    assert_eq!(err.code, 1, "{}", err.err);
    assert!(err.err.contains("does not occur"), "{}", err.err);
}

#[test]
fn staging_an_unchanged_file_with_references_is_a_no_op() {
    let (a, _b) = wired_pair();
    write(&a, ".conf", "font = Menlo\nsize = 12\n");
    let r = dw(&a, &["stage", "--all"]);
    assert!(
        r.err.contains("nothing to stage") || r.out.contains("nothing"),
        "{} {}",
        r.out,
        r.err
    );
    assert_eq!(
        stored_conf(&a),
        format!("font = {}\nsize = 12\n", font_ref())
    );
    // Editing other text keeps the reference.
    write(&a, ".conf", "font = Menlo\nsize = 13\n");
    a.dw_ok(&["stage", "--all"]);
    assert_eq!(
        stored_conf(&a),
        format!("font = {}\nsize = 13\n", font_ref())
    );
    assert!(read_data_default(&a).contains("Menlo"));
}

fn read_data_default(t: &TestEnv) -> String {
    fs::read_to_string(repo(t).join("home/.chezmoidata/dotwarden-values/FONT.toml")).unwrap()
}

#[test]
fn a_changed_value_in_home_is_reported_and_staging_updates_the_variable() {
    let (a, b) = pair();
    a.dw_ok(&["values", "set-variable", "FONT", "Plain", "--apply"]);
    a.dw_ok(&[
        "values",
        "set-variable",
        "FONT",
        "Menlo",
        "--target",
        "platform:darwin",
        "--apply",
    ]);
    a.dw_ok(&["values", "wire", ".conf", "FONT", "--apply"]);
    a.dw_ok(&["commit", "-m", "wire"]);
    sync(&a, &b);
    write(&a, ".conf", "font = Monaco\nsize = 12\n");
    let status = a.dw_ok(&["status"]);
    assert!(
        status.contains("Variable FONT changed: Menlo → Monaco")
            && status.contains("platform:darwin override"),
        "{status}"
    );
    let diff = a.dw_ok(&["diff"]);
    assert!(
        diff.contains("variable FONT changed") && diff.contains("+font = Monaco"),
        "{diff}"
    );
    assert!(
        !diff.contains("template"),
        "no reference text in the working diff: {diff}"
    );
    let staged = a.dw_ok(&["stage", "--all"]);
    assert!(
        staged.contains("staging updates the platform:darwin override"),
        "{staged}"
    );
    let data = read_data_default(&a);
    assert!(data.contains("Monaco") && data.contains("Plain"), "{data}");
    assert_eq!(
        stored_conf(&a),
        format!("font = {}\nsize = 12\n", font_ref())
    );
    assert!(
        !a.dw_ok(&["status"]).contains("Variable FONT"),
        "nothing left to report"
    );
}

#[test]
fn literal_braces_stay_intact_through_wire_stage_and_apply() {
    let (a, b) = pair();
    let text = "x = {{ not a template }}\nopen {{{ font = Menlo {\n}} {{\"{{\"}} {\n";
    write(&a, ".braces", text);
    a.dw_ok(&["track", ".braces", "--apply"]);
    a.dw_ok(&["values", "set-variable", "FONT", "Menlo", "--apply"]);
    a.dw_ok(&["values", "wire", ".braces", "FONT", "--apply"]);
    a.dw_ok(&["commit", "-m", "braces"]);
    assert_eq!(
        read(&a, ".braces"),
        text,
        "chezmoi renders the braces literally"
    );
    assert!(!a.dw_ok(&["status"]).contains("Local"));
    // Edit a line with braces and stage: still the same text after a re-render.
    let edited = text.replace("x = ", "y = ");
    write(&a, ".braces", &edited);
    a.dw_ok(&["stage", "--all"]);
    a.dw_ok(&["commit", "-m", "edit"]);
    assert!(!a.dw_ok(&["status"]).contains("Local"));
    sync(&a, &b);
    b.dw_ok(&["apply", "--all"]);
    assert_eq!(read(&b, ".braces"), edited);
}

#[test]
fn references_work_inside_a_variant_file() {
    let (a, b) = pair();
    // darwin gets its own content; debian-like machines keep the default.
    write(&a, ".conf", "mac font = Menlo {{raw}}\n");
    a.dw_ok(&[
        "variant",
        "add",
        ".conf",
        "--target",
        "platform:darwin",
        "--apply",
    ]);
    a.dw_ok(&["values", "set-variable", "FONT", "Menlo", "--apply"]);
    a.dw_ok(&["values", "wire", ".conf", "FONT", "--apply"]);
    a.dw_ok(&["commit", "-m", "variant wired"]);
    let vdir = repo(&a).join("home/.dotwarden/variants/.conf");
    let mac = fs::read_to_string(vdir.join("platform-darwin")).unwrap();
    assert!(
        mac.contains(font_ref()) && mac.contains("{{ \"{{\" }}raw}}"),
        "{mac}"
    );
    assert_eq!(
        fs::read_to_string(vdir.join("default")).unwrap(),
        "font = Menlo\nsize = 12\n",
        "the default variant stays raw"
    );
    assert_eq!(read(&a, ".conf"), "mac font = Menlo {{raw}}\n");
    assert!(!a.dw_ok(&["status"]).contains("Local"));

    // A new value renders into the variant on alpha only.
    a.dw_ok(&["values", "set-variable", "FONT", "Monaco", "--apply"]);
    a.dw_ok(&["commit", "-m", "monaco"]);
    sync(&a, &b);
    a.dw_ok(&["apply", "--all"]);
    assert_eq!(read(&a, ".conf"), "mac font = Monaco {{raw}}\n");
    b.dw_ok(&["apply", "--all"]);
    assert_eq!(
        read(&b, ".conf"),
        "font = Menlo\nsize = 12\n",
        "beta's default is untouched"
    );

    // HOME edit of the value stages into the variant and updates the variable.
    write(&a, ".conf", "mac font = Courier {{raw}}\n");
    let staged = a.dw_ok(&["stage", "--all"]);
    assert!(
        staged.contains("variable FONT changed: Monaco → Courier"),
        "{staged}"
    );
    let mac = fs::read_to_string(vdir.join("platform-darwin")).unwrap();
    assert!(
        mac.contains(font_ref()) && !mac.contains("Courier"),
        "{mac}"
    );
    assert!(read_data_default(&a).contains("Courier"));
}

#[test]
fn detection_proposes_wiring_on_track_and_on_stage_but_never_applies() {
    let (a, _b) = pair();
    a.dw_ok(&["values", "set-variable", "FONT", "JetBrainsMono", "--apply"]);
    write(&a, ".term", "font = JetBrainsMono\n");
    a.dw_ok(&["track", ".term", "--apply"]);
    let pending = a.dw_ok(&["pending", "list"]);
    assert!(pending.contains("Wire FONT into ~/.term"), "{pending}");
    assert!(
        repo(&a).join("home/dot_term").is_file(),
        "not wired by itself"
    );
    // Dropping it, then a stage of new content that holds the value proposes again.
    a.dw_ok(&["pending", "drop", "1"]);
    write(&a, ".term", "font = JetBrainsMono\nbold = JetBrainsMono\n");
    let staged = a.dw_ok(&["stage", "--all"]);
    assert!(
        staged.contains("proposed") && staged.contains("Wire FONT into ~/.term"),
        "{staged}"
    );
    assert!(
        a.dw_ok(&["pending", "list"])
            .contains("Wire FONT into ~/.term")
    );
    assert!(repo(&a).join("home/dot_term").is_file());
    // Applying the proposal wires both occurrences.
    a.dw_ok(&["commit", "-m", "term"]);
    a.dw_ok(&["pending", "apply", "--all"]);
    let t = fs::read_to_string(repo(&a).join("home/dot_term.tmpl")).unwrap();
    assert_eq!(t.matches("dotwarden/value").count(), 2, "{t}");
}

#[test]
fn an_agent_can_set_and_wire_through_pending_items() {
    let (a, _b) = pair();
    let r = run_with(
        &a,
        true,
        &["values", "set-variable", "FONT", "Menlo", "--apply"],
    );
    assert_eq!(r.code, 0, "{}", r.err);
    let r = run_with(&a, true, &["values", "wire", ".conf", "FONT"]);
    assert_eq!(r.code, 0, "{}", r.err);
    let list = a.dw_ok(&["pending", "list"]);
    assert!(
        list.contains("Wire FONT into ~/.conf") && list.contains("claude"),
        "{list}"
    );
}

#[test]
fn removing_and_renaming_keep_the_data_consistent() {
    let (a, _b) = wired_pair();
    let r = dw(&a, &["values", "unset-variable", "FONT", "--apply"]);
    assert_eq!(r.code, 1);
    assert!(
        r.err.contains("still referenced") && r.err.contains("~/.conf"),
        "{}",
        r.err
    );
    let r = dw(
        &a,
        &[
            "values",
            "set-variable",
            "SIZE",
            "1",
            "--target",
            "machine:alpha",
        ],
    );
    assert!(r.err.contains("no default yet"), "{}", r.err);
    let r = dw(&a, &["values", "set-variable", "font", "x"]);
    assert!(r.err.contains("differ only by case"), "{}", r.err);

    a.dw_ok(&[
        "values",
        "set-variable",
        "FONT",
        "Alpha",
        "--target",
        "machine:alpha",
        "--apply",
    ]);
    a.dw_ok(&["machine", "rename", "alpha", "first", "--apply"]);
    assert!(read_data_default(&a).contains("machine:first"));
    assert!(
        a.dw_ok(&["values", "show", "FONT"])
            .contains("receives Alpha (machine:first)")
    );
    a.dw_ok(&[
        "values",
        "unset-variable",
        "FONT",
        "--target",
        "machine:first",
        "--apply",
    ]);
    assert!(!read_data_default(&a).contains("machine:first"));
}

#[test]
fn one_variable_with_different_text_at_two_positions_is_refused() {
    let (a, _b) = pair();
    write(&a, ".conf", "a = Menlo\nb = Menlo\n");
    a.dw_ok(&["stage", "--all"]);
    a.dw_ok(&["commit", "-m", "two"]);
    a.dw_ok(&["values", "set-variable", "FONT", "Menlo", "--apply"]);
    a.dw_ok(&["values", "wire", ".conf", "FONT", "--apply"]);
    a.dw_ok(&["commit", "-m", "wired"]);
    write(&a, ".conf", "a = One\nb = Two\n");
    let r = dw(&a, &["stage", ".conf"]);
    assert_ne!(r.code, 0);
    assert!(
        r.err.contains("different text at different positions"),
        "{} {}",
        r.out,
        r.err
    );
    assert!(
        stored_conf(&a).matches("dotwarden/value").count() == 2,
        "stored file unchanged"
    );
}
