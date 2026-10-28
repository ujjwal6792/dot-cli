//! Variants with real git and chezmoi: one tracked path with different content
//! per target, selection by specificity, clashes, staging, discard, rename,
//! untrack and preview-as, including two machines sharing a local bare remote.

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

/// The stored content of a variant file (`default`, `family-debian`, …) of `.conf`.
fn stored(t: &TestEnv, name: &str) -> String {
    fs::read_to_string(repo(t).join("home/.dotwarden/variants/.conf").join(name)).unwrap()
}

fn data(t: &TestEnv) -> String {
    fs::read_to_string(repo(t).join("home/.chezmoidata/dotwarden.toml")).unwrap()
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

fn registered(t: &TestEnv) {
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
}

/// `alpha` (darwin) tracking `.conf`, with `srv` (linux/ubuntu/debian) registered.
fn alpha_with_conf() -> TestEnv {
    let t = TestEnv::new();
    t.dw_ok(&["init", "--name", "alpha"]);
    registered(&t);
    write(&t, ".conf", "base {{ not a template }}\n");
    t.dw_ok(&["track", ".conf", "--apply"]);
    t.dw_ok(&["commit", "-m", "conf"]);
    t
}

/// alpha (darwin/macos) and beta (a second computer registered as linux/ubuntu,
/// family debian) sharing a bare remote, both holding the tracked `.conf`.
fn pair() -> (TestEnv, TestEnv) {
    let a = TestEnv::new();
    a.dw_ok(&["init", "--name", "alpha"]);
    write(&a, ".conf", "base {{ not a template }}\n");
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

#[test]
fn two_machines_receive_different_content_for_one_path_and_the_most_specific_variant_wins() {
    let (a, b) = pair();
    write(&a, ".conf", "mac {{ raw\n");
    a.dw_ok(&[
        "variant",
        "add",
        ".conf",
        "--target",
        "platform:darwin",
        "--apply",
    ]);
    write(&a, ".conf", "debian\n");
    a.dw_ok(&[
        "variant",
        "add",
        ".conf",
        "--target",
        "family:debian",
        "--apply",
    ]);
    write(&a, ".conf", "mac {{ raw\n");
    a.dw_ok(&["commit", "-m", "variants"]);
    assert!(
        !a.dw_ok(&["status"]).contains("Local"),
        "alpha already holds what it receives"
    );
    sync(&a, &b);
    b.dw_ok(&["apply", "--all"]);
    assert_eq!(read(&b, ".conf"), "debian\n", "beta is debian-like linux");
    assert_eq!(
        read(&a, ".conf"),
        "mac {{ raw\n",
        "raw content, no template evaluation"
    );

    // A machine-level variant beats the family and platform variants.
    write(&b, ".conf", "beta only\n");
    b.dw_ok(&[
        "variant",
        "add",
        ".conf",
        "--target",
        "machine:beta",
        "--apply",
    ]);
    b.dw_ok(&["commit", "-m", "beta"]);
    sync(&b, &a);
    let listed = b.dw_ok(&["variant", "list", ".conf"]);
    assert!(listed.contains("machine:beta  <- this machine"), "{listed}");
    let on_alpha = a.dw_ok(&["variant", "list", ".conf"]);
    assert!(
        on_alpha.contains("platform:darwin  <- this machine"),
        "{on_alpha}"
    );
    a.dw_ok(&["apply", "--all"]);
    assert_eq!(read(&a, ".conf"), "mac {{ raw\n", "alpha is unaffected");
}

#[test]
fn a_clash_blocks_only_its_own_file() {
    let (a, b) = pair();
    write(&a, ".other", "o1\n");
    a.dw_ok(&["track", ".other", "--apply"]);
    a.dw_ok(&["commit", "-m", "other"]);
    sync(&a, &b);
    b.dw_ok(&["apply", "--all"]);
    // beta is in two families with a variant each: a same-level clash for .conf.
    let file = repo(&b).join("home/.chezmoidata/dotwarden-machines/beta.toml");
    let text = fs::read_to_string(&file)
        .unwrap()
        .replace("[\"debian\"]", "[\"debian\", \"ubuntu-like\"]");
    fs::write(&file, text).unwrap();
    b.git(&repo(&b), &["add", "-A"]);
    b.dw_ok(&["commit", "-m", "two families"]);
    write(&b, ".conf", "deb\n");
    b.dw_ok(&[
        "variant",
        "add",
        ".conf",
        "--target",
        "family:debian",
        "--apply",
    ]);
    write(&b, ".conf", "ubu\n");
    b.dw_ok(&[
        "variant",
        "add",
        ".conf",
        "--target",
        "family:ubuntu-like",
        "--apply",
    ]);
    b.dw_ok(&["commit", "-m", "clash"]);

    let status = b.dw_ok(&["status"]);
    assert!(
        status.contains("Blocked by variant clashes: 1")
            && status.contains("~/.conf: ambiguous variants at the same specificity"),
        "{status}"
    );
    let doctor = b.dw_ok(&["doctor"]);
    assert!(
        doctor.contains("variants") && doctor.contains("blocked"),
        "{doctor}"
    );
    let r = dw(&b, &["stage", ".conf"]);
    assert!(
        r.code != 0 && r.err.contains("ambiguous variants"),
        "{}",
        r.err
    );

    // Everything else keeps working: stage, commit, discard and apply of other files.
    write(&b, ".other", "o2 edited here\n");
    b.dw_ok(&["stage", ".other"]);
    b.dw_ok(&["commit", "-m", "other edit"]);
    sync(&b, &a);
    write(&a, ".third", "t\n");
    a.dw_ok(&["track", ".third", "--apply"]);
    a.dw_ok(&["commit", "-m", "third"]);
    sync(&a, &b);
    b.dw_ok(&["apply", "--all"]);
    assert_eq!(read(&b, ".third"), "t\n", "other files are applied");
    write(&b, ".third", "scribble\n");
    b.dw_ok(&["discard", ".third", "--yes"]);
    assert_eq!(read(&b, ".third"), "t\n");
    assert!(b.dw_ok(&["status"]).contains("Blocked by variant clashes"));
}

#[test]
fn staging_updates_the_variant_this_machine_receives_and_discard_restores_it() {
    let (a, b) = pair();
    write(&a, ".conf", "mac\n");
    a.dw_ok(&[
        "variant",
        "add",
        ".conf",
        "--target",
        "platform:darwin",
        "--apply",
    ]);
    write(&a, ".conf", "debian v1\n");
    a.dw_ok(&[
        "variant",
        "add",
        ".conf",
        "--target",
        "family:debian",
        "--apply",
    ]);
    write(&a, ".conf", "mac\n");
    a.dw_ok(&["commit", "-m", "variants"]);
    sync(&a, &b);
    b.dw_ok(&["apply", "--all"]);
    assert_eq!(read(&b, ".conf"), "debian v1\n");

    // Discard writes the variant beta receives, not the default.
    write(&b, ".conf", "scribble\n");
    assert!(b.dw_ok(&["status"]).contains("modified  ~/.conf"));
    b.dw_ok(&["discard", ".conf", "--yes"]);
    assert_eq!(read(&b, ".conf"), "debian v1\n");

    // Staging writes the HOME content into beta's variant only.
    write(&b, ".conf", "debian v2\n");
    b.dw_ok(&["stage", ".conf"]);
    assert_eq!(stored(&b, "family-debian"), "debian v2\n");
    assert_eq!(stored(&b, "platform-darwin"), "mac\n");
    assert_eq!(stored(&b, "default"), "base {{ not a template }}\n");
    let diff = b.dw_ok(&["diff", "--no-pager", ".conf"]);
    assert!(
        diff.contains("-debian v1") && diff.contains("+debian v2"),
        "{diff}"
    );
    b.dw_ok(&["commit", "-m", "v2"]);
    assert!(
        !b.dw_ok(&["status"]).contains("Local"),
        "{}",
        b.dw_ok(&["status"])
    );
    sync(&b, &a);
    a.dw_ok(&["apply", "--all"]);
    assert_eq!(
        read(&a, ".conf"),
        "mac\n",
        "alpha still gets its own variant"
    );
}

#[test]
fn a_machine_receiving_the_default_stages_into_the_default() {
    let t = alpha_with_conf();
    write(&t, ".conf", "for srv\n");
    t.dw_ok(&[
        "variant",
        "add",
        ".conf",
        "--target",
        "machine:srv",
        "--apply",
    ]);
    t.dw_ok(&["commit", "-m", "srv variant"]);
    // alpha receives the default, which still has the old content: HOME differs.
    assert!(t.dw_ok(&["status"]).contains("modified  ~/.conf"));
    write(&t, ".conf", "new default\n");
    t.dw_ok(&["stage", ".conf"]);
    assert_eq!(stored(&t, "default"), "new default\n");
    assert_eq!(stored(&t, "machine-srv"), "for srv\n");
}

#[test]
fn adding_and_removing_variants_goes_through_exact_previews_and_back_to_a_plain_file() {
    let t = alpha_with_conf();
    write(&t, ".conf", "srv\n");
    let dry = dw(
        &t,
        &[
            "variant",
            "add",
            ".conf",
            "--target",
            "machine:srv",
            "--dry-run",
        ],
    );
    for line in [
        "- home/dot_conf",
        "+ home/dot_conf.tmpl",
        "+ home/.dotwarden/variants/.conf/default",
        "+ home/.dotwarden/variants/.conf/machine-srv",
        "srv: ~/.conf receives machine:srv instead of default",
    ] {
        assert!(dry.out.contains(line), "{line} missing:\n{}", dry.out);
    }
    assert!(
        repo(&t).join("home/dot_conf").exists(),
        "a dry run changes nothing"
    );
    t.dw_ok(&[
        "variant",
        "add",
        ".conf",
        "--target",
        "machine:srv",
        "--apply",
    ]);
    assert!(
        repo(&t).join("home/dot_conf.tmpl").exists() && !repo(&t).join("home/dot_conf").exists()
    );
    write(&t, ".conf", "debian\n");
    t.dw_ok(&[
        "variant",
        "add",
        ".conf",
        "--target",
        "family:debian",
        "--apply",
    ]);

    let r = dw(
        &t,
        &["variant", "add", ".conf", "--target", "family:debian"],
    );
    assert!(
        r.code != 0 && r.err.contains("already has a variant"),
        "{}",
        r.err
    );
    let r = dw(
        &t,
        &["variant", "add", ".nothing", "--target", "family:debian"],
    );
    assert!(r.code != 0, "untracked files have no variants");
    let r = dw(
        &t,
        &["variant", "add", ".conf", "--target", "machine:ghost"],
    );
    assert!(
        r.code == 2 && r.err.contains("not in the registry"),
        "{}",
        r.err
    );

    t.dw_ok(&[
        "variant",
        "remove",
        ".conf",
        "--target",
        "family:debian",
        "--apply",
    ]);
    assert!(
        repo(&t).join("home/dot_conf.tmpl").exists(),
        "one variant is left"
    );
    let gone = dw(
        &t,
        &[
            "variant",
            "remove",
            ".conf",
            "--target",
            "machine:srv",
            "--dry-run",
        ],
    );
    assert!(
        gone.out.contains("+ home/dot_conf  (file, .conf)")
            && gone.out.contains("plain file with its default content"),
        "{}",
        gone.out
    );
    t.dw_ok(&[
        "variant",
        "remove",
        ".conf",
        "--target",
        "machine:srv",
        "--apply",
    ]);
    let repo = repo(&t);
    assert_eq!(
        fs::read_to_string(repo.join("home/dot_conf")).unwrap(),
        "base {{ not a template }}\n",
        "the default content is the plain file again"
    );
    assert!(!repo.join("home/dot_conf.tmpl").exists());
    assert!(!repo.join("home/.dotwarden/variants/.conf").exists());
    assert!(!data(&t).contains("dotwarden.variants"), "{}", data(&t));
    assert!(t.dw_ok(&["variant", "list"]).contains("No variants"));
}

#[test]
fn agents_may_add_variants_through_the_pending_list() {
    let t = alpha_with_conf();
    write(&t, ".conf", "agent\n");
    let r = run_with(
        &t,
        true,
        &["variant", "add", ".conf", "--target", "machine:srv"],
    );
    assert_eq!(r.code, 0, "{}{}", r.out, r.err);
    let r = run_with(&t, true, &["pending", "apply", "--all"]);
    assert_eq!(r.code, 0, "no human approval needed: {}{}", r.out, r.err);
    assert_eq!(stored(&t, "machine-srv"), "agent\n");
}

#[test]
fn renaming_a_machine_keeps_its_variant_selected() {
    let t = alpha_with_conf();
    write(&t, ".conf", "mine\n");
    t.dw_ok(&[
        "variant",
        "add",
        ".conf",
        "--target",
        "machine:alpha",
        "--apply",
    ]);
    t.dw_ok(&["commit", "-m", "mine"]);
    t.dw_ok(&["machine", "rename", "alpha", "mac", "--apply"]);
    assert!(data(&t).contains("machine:mac") && !data(&t).contains("machine:alpha"));
    let listed = t.dw_ok(&["variant", "list", ".conf"]);
    assert!(listed.contains("machine:mac  <- this machine"), "{listed}");
    assert!(
        !t.dw_ok(&["status"]).contains("modified  ~/.conf"),
        "{}",
        t.dw_ok(&["status"])
    );
    // The old file name is not reused by a later variant for a new machine called alpha.
    t.dw_ok(&[
        "machine",
        "add",
        "alpha",
        "--platform",
        "linux",
        "--distribution",
        "arch",
        "--apply",
    ]);
    t.dw_ok(&[
        "variant",
        "add",
        ".conf",
        "--target",
        "machine:alpha",
        "--apply",
    ]);
    assert_eq!(stored(&t, "machine-alpha"), "mine\n");
    assert!(data(&t).contains("machine-alpha-2"), "{}", data(&t));
}

#[test]
fn retiring_leaves_the_variant_in_place_and_says_so() {
    let t = alpha_with_conf();
    write(&t, ".conf", "srv\n");
    t.dw_ok(&[
        "variant",
        "add",
        ".conf",
        "--target",
        "machine:srv",
        "--apply",
    ]);
    t.dw_ok(&["commit", "-m", "srv"]);
    let r = dw(&t, &["machine", "retire", "srv", "--dry-run"]);
    assert!(
        r.out.contains("variant stays in place but is unused"),
        "{}",
        r.out
    );
    t.dw_ok(&["machine", "retire", "srv", "--yes"]);
    assert_eq!(stored(&t, "machine-srv"), "srv\n");
    assert!(data(&t).contains("machine:srv"));
    // The retired machine's variant can still be removed.
    t.dw_ok(&[
        "variant",
        "remove",
        ".conf",
        "--target",
        "machine:srv",
        "--apply",
    ]);
}

#[test]
fn untracking_removes_every_variant() {
    let t = alpha_with_conf();
    write(&t, ".conf", "srv\n");
    t.dw_ok(&[
        "variant",
        "add",
        ".conf",
        "--target",
        "machine:srv",
        "--apply",
    ]);
    t.dw_ok(&["commit", "-m", "srv"]);
    let dry = dw(&t, &["untrack", ".conf", "--dry-run"]);
    assert!(
        dry.out
            .contains("- home/.dotwarden/variants/.conf/machine-srv")
            && dry.out.contains("- home/.dotwarden/variants/.conf/default"),
        "{}",
        dry.out
    );
    t.dw_ok(&["untrack", ".conf", "--apply"]);
    assert!(!repo(&t).join("home/.dotwarden/variants/.conf").exists());
    assert!(!repo(&t).join("home/dot_conf.tmpl").exists());
    assert!(!data(&t).contains("dotwarden.variants"), "{}", data(&t));
    assert_eq!(read(&t, ".conf"), "srv\n", "HOME is left alone");
}

#[test]
fn preview_as_a_machine_shows_which_variant_it_gets() {
    let t = alpha_with_conf();
    write(&t, ".conf", "srv\n");
    t.dw_ok(&[
        "variant",
        "add",
        ".conf",
        "--target",
        "machine:srv",
        "--apply",
    ]);
    t.dw_ok(&["commit", "-m", "srv"]);
    write(&t, ".conf", "other\n");
    let srv = t.dw_ok(&["machine", "preview", "srv"]);
    assert!(srv.contains("~/.conf  [variant: machine:srv]"), "{srv}");
    let alpha = t.dw_ok(&["machine", "preview", "alpha"]);
    assert!(alpha.contains("~/.conf  [variant: default]"), "{alpha}");
    let diff = t.dw_json(&["machine", "preview", "srv", "--diff"]);
    let d = diff["diff"].as_str().unwrap();
    assert!(d.contains("-other") && d.contains("+srv"), "{d}");
    let v = &diff["variants"][0];
    assert_eq!(v["receives"], "machine:srv");
}

#[test]
fn applying_on_a_fresh_machine_creates_missing_parent_directories() {
    let (a, b) = pair();
    write(&a, ".config/ghostty/themes/config", "deep\n");
    a.dw_ok(&["track", ".config/ghostty/themes/config", "--apply"]);
    a.dw_ok(&["commit", "-m", "deep"]);
    sync(&a, &b);
    assert!(!b.home.join(".config").exists());
    b.dw_ok(&["apply", "--all"]);
    assert_eq!(read(&b, ".config/ghostty/themes/config"), "deep\n");
    // Discard of a file whose folders were removed also recreates them.
    write(&b, ".config/ghostty/themes/config", "edit\n");
    fs::remove_dir_all(b.home.join(".config/ghostty")).unwrap();
    b.dw_ok(&["discard", ".config/ghostty/themes/config", "--yes"]);
    assert_eq!(read(&b, ".config/ghostty/themes/config"), "deep\n");
}

#[test]
fn retiring_notes_assignments_and_a_mode_only_change_of_a_variant_file_is_reported() {
    let t = alpha_with_conf();
    write(&t, ".conf", "srv\n");
    t.dw_ok(&[
        "variant",
        "add",
        ".conf",
        "--target",
        "machine:srv",
        "--apply",
    ]);
    t.dw_ok(&[
        "assign",
        ".conf",
        "--include",
        "machine:srv",
        "--include",
        "machine:alpha",
        "--apply",
    ]);
    let r = dw(&t, &["machine", "retire", "srv", "--dry-run"]);
    assert!(
        !r.out.contains("assignments / ") && r.out.contains("variants / "),
        "assignments are listed under data, not as leftover notes:\n{}",
        r.out
    );
    // alpha receives the default; make HOME equal to it and change only the mode.
    t.dw_ok(&["commit", "-m", "x"]);
    write(&t, ".conf", "base {{ not a template }}\n");
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(t.home.join(".conf"), fs::Permissions::from_mode(0o755)).unwrap();
    let r = dw(&t, &["stage", ".conf"]);
    assert!(
        r.code != 0 && r.err.contains("only its mode differs"),
        "{}",
        r.err
    );
}
