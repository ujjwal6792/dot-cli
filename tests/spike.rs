//! Verified mechanics (docs/spec.md §3): each claim recorded there is proven here
//! against the pinned chezmoi, through the real isolated store.

mod common;

use common::TestEnv;
use std::fs;

use dotwarden::machine::Machine;
use dotwarden::store::Store;

fn machine(name: &str, platform: &str, dist: &str, families: &[&str]) -> Machine {
    Machine {
        name: name.into(),
        platform: platform.into(),
        distribution: dist.into(),
        families: families.iter().map(|s| s.to_string()).collect(),
        retired: false,
    }
}

/// An initialised repository whose registry holds `machines` (no registration of
/// the real computer is needed: the machine under test is chosen by config).
struct Spike {
    t: TestEnv,
    store: Store,
}

impl Spike {
    fn new(machines: &[Machine]) -> Self {
        let t = TestEnv::new();
        let store = t.store();
        dotwarden::repo::init(&store, "main").unwrap();
        dotwarden::repo::write_registry(&store.repo(), machines).unwrap();
        Self { t, store }
    }

    fn home_src(&self, rel: &str, content: &str) {
        let p = self.store.repo().join("home").join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, content).unwrap();
    }

    /// Append to the data file, which holds registry, assignments and variants.
    fn data(&self, extra: &str) {
        let p = self.store.repo().join(dotwarden::repo::DATA_FILE);
        let mut s = fs::read_to_string(&p).unwrap();
        s.push_str(extra);
        fs::write(p, s).unwrap();
    }

    fn become_machine(&self, name: &str) {
        self.store.write_chezmoi_config(name).unwrap();
    }

    fn managed(&self, name: &str) -> Vec<String> {
        self.become_machine(name);
        let out = self
            .store
            .chezmoi(&["managed", "--path-style", "relative"])
            .unwrap();
        out.stdout.lines().map(str::to_string).collect()
    }

    fn rendered(&self, name: &str, target: &str) -> Result<String, String> {
        self.become_machine(name);
        let path = self.t.home.join(target);
        self.store
            .chezmoi(&["cat".as_ref(), path.as_os_str()])
            .map(|o| o.stdout)
            .map_err(|e| format!("{e:#}"))
    }
}

fn fleet() -> Vec<Machine> {
    vec![
        machine("mac", "darwin", "macos", &[]),
        machine("work", "darwin", "macos", &[]),
        machine("ubu", "linux", "ubuntu", &["debian"]),
        machine("arch1", "linux", "arch", &["arch-like"]),
        machine("cachy", "linux", "cachyos", &["arch-like"]),
        machine("cachy2", "linux", "cachyos", &["arch-like"]),
        machine("multi", "linux", "mint", &["debian", "ubuntu-like"]),
    ]
}

#[test]
fn chezmoiroot_makes_home_the_source_state_root() {
    let s = Spike::new(&fleet());
    s.become_machine("mac");
    let repo = s.store.repo();
    // Chezmoi is pointed at the repository root; .chezmoiroot redirects to home/.
    assert_eq!(
        s.store.chezmoi(&["source-path"]).unwrap().stdout.trim(),
        repo.join("home").display().to_string()
    );
    s.home_src("dot_zshrc", "z\n");
    // Files at the repository root are not chezmoi source state.
    fs::write(repo.join("dot_not_a_target"), "x").unwrap();
    fs::write(repo.join("README.md"), "x").unwrap();
    let managed = s.managed("mac");
    assert_eq!(managed, [".zshrc"], "root-level files must not be targets");
}

#[test]
fn leading_dot_entries_under_home_are_not_targets() {
    let s = Spike::new(&fleet());
    s.home_src("dot_zshrc", "z\n");
    s.home_src(".dotwarden/variants/gitconfig/linux", "linux\n");
    s.home_src(".dotwarden/vault/recipient.txt", "age1...\n");
    s.home_src(".dotwarden/vault/secrets/TOKEN.age", "cipher\n");
    s.home_src(".chezmoitemplates/extra", "t\n");
    assert_eq!(s.managed("mac"), [".zshrc"]);
    s.store.chezmoi(&["apply"]).unwrap();
    assert!(s.t.home.join(".zshrc").is_file());
    for leak in [
        ".dotwarden",
        "dotwarden",
        ".chezmoitemplates",
        ".chezmoidata",
        ".chezmoiignore",
    ] {
        assert!(!s.t.home.join(leak).exists(), "{leak} was applied to HOME");
    }
}

#[test]
fn include_resolves_relative_to_the_chezmoiroot_not_the_repository_root() {
    let s = Spike::new(&fleet());
    s.home_src(".dotwarden/variants/note", "variant body\n");
    s.home_src("dot_ok.tmpl", "{{ include \".dotwarden/variants/note\" }}");
    s.home_src(
        "dot_bad.tmpl",
        "{{ include \"home/.dotwarden/variants/note\" }}",
    );
    assert_eq!(s.rendered("mac", ".ok").unwrap(), "variant body\n");
    let err = s.rendered("mac", ".bad").unwrap_err();
    assert!(
        err.contains("no such file") || err.contains("not exist"),
        "{err}"
    );
}

#[test]
fn assignments_union_of_includes_excludes_win_and_all_four_target_kinds() {
    let s = Spike::new(&fleet());
    for f in [
        "dot_everywhere",
        "dot_linux_only",
        "dot_arch_only",
        "dot_ubuntu_only",
        "dot_cachy_machine",
        "dot_union",
        "dot_not_work",
        "dot_exclude_wins",
        "dot_exclude_family",
        "dot_config/app/dot_nested",
        "dot_config/other/kept",
    ] {
        s.home_src(f, "x\n");
    }
    // `.config/app/.nested` is written as dot_config/app/dot_nested -> target `.config/app/.nested`.
    s.data(
        r#"
[dotwarden.assignments.".linux_only"]
include = ["platform:linux"]
[dotwarden.assignments.".arch_only"]
include = ["family:arch-like"]
[dotwarden.assignments.".ubuntu_only"]
include = ["distribution:ubuntu"]
[dotwarden.assignments.".cachy_machine"]
include = ["machine:cachy"]
[dotwarden.assignments.".union"]
include = ["machine:mac", "distribution:ubuntu", "family:arch-like"]
[dotwarden.assignments.".not_work"]
exclude = ["machine:work"]
[dotwarden.assignments.".exclude_wins"]
include = ["platform:darwin", "platform:linux"]
exclude = ["distribution:cachyos", "machine:work"]
[dotwarden.assignments.".exclude_family"]
include = ["platform:linux"]
exclude = ["family:arch-like"]
[dotwarden.assignments.".config/app/.nested"]
include = ["platform:darwin"]
"#,
    );
    let expect = |name: &str, want: &[&str]| {
        let mut got = s.managed(name);
        got.retain(|p| !p.ends_with("/") && !p.is_empty());
        let mut want: Vec<String> = want.iter().map(|s| s.to_string()).collect();
        want.sort();
        got.sort();
        assert_eq!(got, want, "managed set of {name}");
    };
    // Finding: ignoring every file of a directory leaves the (empty) directory
    // itself managed, so `.config/app` is in every machine's set.
    let common = [
        ".everywhere",
        ".config",
        ".config/app",
        ".config/other",
        ".config/other/kept",
    ];
    let with = |extra: &[&'static str]| -> Vec<&'static str> {
        common
            .iter()
            .copied()
            .chain(extra.iter().copied())
            .collect()
    };
    expect(
        "mac",
        &with(&[
            ".union",
            ".not_work",
            ".exclude_wins",
            ".config/app/.nested",
        ]),
    );
    // work: excluded by machine name from .not_work and .exclude_wins; .union only via machine:mac.
    expect("work", &with(&[".config/app/.nested"]));
    expect(
        "ubu",
        &with(&[
            ".linux_only",
            ".ubuntu_only",
            ".union",
            ".not_work",
            ".exclude_wins",
            ".exclude_family",
        ]),
    );
    expect(
        "arch1",
        &with(&[
            ".linux_only",
            ".arch_only",
            ".union",
            ".not_work",
            ".exclude_wins",
        ]),
    );
    // cachy: family include matches but distribution exclude wins for .exclude_wins.
    expect(
        "cachy",
        &with(&[
            ".linux_only",
            ".arch_only",
            ".cachy_machine",
            ".union",
            ".not_work",
        ]),
    );
    expect(
        "cachy2",
        &with(&[".linux_only", ".arch_only", ".union", ".not_work"]),
    );
    expect(
        "multi",
        &with(&[
            ".linux_only",
            ".not_work",
            ".exclude_wins",
            ".exclude_family",
        ]),
    );
}

#[test]
fn assignments_are_enforced_by_plain_apply() {
    let s = Spike::new(&fleet());
    s.home_src("dot_only_mac", "m\n");
    s.home_src("dot_all", "a\n");
    s.data("[dotwarden.assignments.\".only_mac\"]\ninclude = [\"machine:mac\"]\n");
    s.become_machine("ubu");
    s.store.chezmoi(&["apply"]).unwrap();
    assert!(s.t.home.join(".all").is_file());
    assert!(!s.t.home.join(".only_mac").exists());
    s.become_machine("mac");
    s.store.chezmoi(&["apply"]).unwrap();
    assert!(s.t.home.join(".only_mac").is_file());
}

#[test]
fn assigning_changes_the_data_file_and_the_managed_set_but_never_chezmoiignore() {
    let s = Spike::new(&fleet());
    s.home_src("dot_x", "x\n");
    let repo = s.store.repo();
    let commit = |msg: &str| {
        s.store.git(&["add", "-A"]).unwrap();
        s.store.git(&["commit", "--quiet", "-m", msg]).unwrap();
    };
    commit("baseline");
    assert!(s.managed("mac").contains(&".x".to_string()));

    dotwarden::assign::set_assignment(&repo, ".x", &["machine:ubu".into()], &[]).unwrap();

    // Behaviour changed for the machines concerned ...
    assert!(!s.managed("mac").contains(&".x".to_string()));
    assert!(s.managed("ubu").contains(&".x".to_string()));
    // ... and the only thing that changed in the repository is the data file.
    let changed = s.store.git(&["diff", "--name-only"]).unwrap().stdout;
    assert_eq!(changed.trim(), dotwarden::repo::DATA_FILE);

    // Removing the assignment restores "everywhere", again by data alone.
    dotwarden::assign::set_assignment(&repo, ".x", &[], &[]).unwrap();
    assert!(s.managed("mac").contains(&".x".to_string()));
    assert_eq!(
        s.store
            .git(&["status", "--porcelain"])
            .unwrap()
            .stdout
            .trim(),
        ""
    );
}

#[test]
fn unrepresentable_assignment_paths_are_refused_before_anything_is_written() {
    let s = Spike::new(&fleet());
    let data = s.store.repo().join(dotwarden::repo::DATA_FILE);
    let before = fs::read_to_string(&data).unwrap();
    for bad in ["a\nb", "a\rb", "/abs", "a/../b", "a\tb", ""] {
        let e =
            dotwarden::assign::set_assignment(&s.store.repo(), bad, &["machine:mac".into()], &[])
                .unwrap_err();
        assert_eq!(
            dotwarden::error::classify(&e).0,
            dotwarden::error::Kind::Usage,
            "{bad:?}"
        );
    }
    let e = dotwarden::assign::set_assignment(&s.store.repo(), "ok", &["bogus:thing".into()], &[])
        .unwrap_err();
    assert!(format!("{e:#}").contains("invalid target"));
    assert_eq!(fs::read_to_string(&data).unwrap(), before);
}

/// `name` is excluded on `mac` by an assignment; `sibling` must stay managed.
/// Each name is a hazard in `.chezmoiignore` syntax if written unescaped.
fn exclusion_matches_literally(name: &str, sibling: &str) {
    let s = Spike::new(&fleet());
    s.home_src(name, "x\n");
    s.home_src(sibling, "y\n");
    s.home_src("zzz", "z\n");
    dotwarden::assign::set_assignment(&s.store.repo(), name, &[], &["machine:mac".into()]).unwrap();
    let mut got = s.managed("mac");
    got.sort();
    let mut want = vec![sibling.to_string(), "zzz".to_string()];
    want.sort();
    assert_eq!(got, want, "excluding {name:?}");
    // Other machines still get it.
    assert!(
        s.managed("ubu").contains(&name.to_string()),
        "{name:?} on ubu"
    );
}

#[test]
fn ignore_escaping_leading_hash_is_not_a_comment() {
    exclusion_matches_literally("#x", "x");
}

#[test]
fn ignore_escaping_leading_bang_is_not_a_negation() {
    exclusion_matches_literally("!x", "x");
}

#[test]
fn ignore_escaping_trailing_and_leading_spaces_survive_trimming() {
    exclusion_matches_literally("a b ", "a b");
    exclusion_matches_literally(" lead", "lead");
    exclusion_matches_literally("a\u{a0}", "a");
}

#[test]
fn ignore_escaping_brackets_star_and_question_mark_match_literally() {
    exclusion_matches_literally("[1]", "1");
    exclusion_matches_literally("*", "star");
    exclusion_matches_literally("?", "q");
    exclusion_matches_literally("{a,b}", "a");
    exclusion_matches_literally("back\\slash", "backslash");
}

#[test]
fn variant_selection_picks_the_most_specific() {
    let s = Spike::new(&fleet());
    for (id, body) in [
        ("platform-linux", "platform\n"),
        ("family-arch", "family\n"),
        ("dist-cachyos", "distribution\n"),
        ("machine-cachy2", "machine\n"),
    ] {
        s.home_src(&format!(".dotwarden/variants/gitconfig/{id}"), body);
    }
    s.home_src(
        "dot_gitconfig.tmpl",
        "{{- $v := includeTemplate \"dotwarden/variant\" (dict \"data\" . \"path\" \".gitconfig\") | trim -}}\n\
         {{- if $v }}{{ include (printf \".dotwarden/variants/gitconfig/%s\" $v) }}{{ else }}default\n{{ end -}}",
    );
    s.data(
        r#"
[[dotwarden.variants.".gitconfig"]]
target = "platform:linux"
file = "platform-linux"
[[dotwarden.variants.".gitconfig"]]
target = "family:arch-like"
file = "family-arch"
[[dotwarden.variants.".gitconfig"]]
target = "distribution:cachyos"
file = "dist-cachyos"
[[dotwarden.variants.".gitconfig"]]
target = "machine:cachy2"
file = "machine-cachy2"
"#,
    );
    for (name, want) in [
        ("mac", "default\n"),
        ("ubu", "platform\n"),
        ("arch1", "family\n"),
        ("cachy", "distribution\n"),
        ("cachy2", "machine\n"),
    ] {
        assert_eq!(
            s.rendered(name, ".gitconfig").unwrap(),
            want,
            "variant for {name}"
        );
    }
    // And through apply.
    s.become_machine("cachy");
    s.store.chezmoi(&["apply"]).unwrap();
    assert_eq!(
        fs::read_to_string(s.t.home.join(".gitconfig")).unwrap(),
        "distribution\n"
    );
}

#[test]
fn variants_at_the_same_specificity_are_an_error_for_that_file_only() {
    let s = Spike::new(&fleet());
    s.home_src(".dotwarden/variants/a", "a\n");
    s.home_src(".dotwarden/variants/b", "b\n");
    let tmpl = |path: &str| {
        format!(
            "{{{{- $v := includeTemplate \"dotwarden/variant\" (dict \"data\" . \"path\" \"{path}\") | trim -}}}}\
             {{{{- if $v }}}}{{{{ include (printf \".dotwarden/variants/%s\" $v) }}}}{{{{ else }}}}default\n{{{{ end -}}}}"
        )
    };
    s.home_src("dot_clash.tmpl", &tmpl(".clash"));
    s.home_src("dot_fine.tmpl", &tmpl(".fine"));
    s.data(
        r#"
[[dotwarden.variants.".clash"]]
target = "family:debian"
file = "a"
[[dotwarden.variants.".clash"]]
target = "family:ubuntu-like"
file = "b"
[[dotwarden.variants.".fine"]]
target = "family:debian"
file = "a"
"#,
    );
    let err = s.rendered("multi", ".clash").unwrap_err();
    assert!(err.contains("ambiguous variants for .clash"), "{err}");
    assert!(
        err.contains("family:debian") && err.contains("family:ubuntu-like"),
        "{err}"
    );
    // A different file on the same machine is unaffected.
    assert_eq!(s.rendered("multi", ".fine").unwrap(), "a\n");
    // A machine matching only one of the two is fine.
    assert_eq!(s.rendered("ubu", ".clash").unwrap(), "a\n");
}

#[test]
fn templates_fail_clearly_for_unknown_machines_and_without_dotwarden_config() {
    let s = Spike::new(&fleet());
    s.home_src("dot_x", "x\n");
    s.become_machine("ghost");
    let err = format!("{:#}", s.store.chezmoi(&["managed"]).unwrap_err());
    assert!(
        err.contains("machine \"ghost\" is not in the machine registry"),
        "{err}"
    );

    // Plain chezmoi without the generated config: explicit message, not a crash.
    let cfg = s.store.dirs.chezmoi_config();
    fs::write(&cfg, "# empty\n").unwrap();
    let err = format!("{:#}", s.store.chezmoi(&["managed"]).unwrap_err());
    assert!(err.contains("dotwardenMachine"), "{err}");
}

#[test]
fn real_registration_works_with_the_fixed_ignore_template() {
    // End to end: `dw init` + a tracked file with an assignment for this machine.
    let t = TestEnv::new();
    t.dw_ok(&["init", "--name", "me"]);
    let repo = t.dirs().repo();
    fs::write(repo.join("home/dot_here"), "h\n").unwrap();
    fs::write(repo.join("home/dot_elsewhere"), "e\n").unwrap();
    let mut data = fs::read_to_string(repo.join(dotwarden::repo::DATA_FILE)).unwrap();
    data.push_str("[dotwarden.assignments.\".elsewhere\"]\nexclude = [\"machine:me\"]\n");
    fs::write(repo.join(dotwarden::repo::DATA_FILE), data).unwrap();
    let out = t
        .store()
        .chezmoi(&["managed", "--path-style", "relative"])
        .unwrap()
        .stdout;
    assert_eq!(out.trim(), ".here");
}

#[test]
fn assignment_paths_with_glob_characters_and_spaces_match_literally() {
    let s = Spike::new(&fleet());
    for f in [
        "dot_a[1]b",
        "dot_a1b",
        "dot_sp ace",
        "dot_star*",
        "dot_starx",
    ] {
        s.home_src(f, "x\n");
    }
    s.data(
        r#"
[dotwarden.assignments.".a[1]b"]
include = ["machine:nobody"]
[dotwarden.assignments.".sp ace"]
include = ["machine:nobody"]
[dotwarden.assignments.".star*"]
include = ["machine:nobody"]
"#,
    );
    let mut got = s.managed("mac");
    got.sort();
    assert_eq!(got, [".a1b", ".starx"]);
}

#[test]
fn an_ambiguous_variant_blocks_only_that_file_with_keep_going() {
    let s = Spike::new(&fleet());
    s.home_src(".dotwarden/variants/a", "a\n");
    s.home_src(".dotwarden/variants/b", "b\n");
    let tmpl = |path: &str| {
        format!(
            "{{{{- $v := includeTemplate \"dotwarden/variant\" (dict \"data\" . \"path\" \"{path}\") | trim -}}}}\
             {{{{- if $v }}}}{{{{ include (printf \".dotwarden/variants/%s\" $v) }}}}{{{{ end -}}}}"
        )
    };
    s.home_src("dot_clash.tmpl", &tmpl(".clash"));
    s.home_src("dot_fine.tmpl", &tmpl(".fine"));
    s.home_src("dot_plain", "plain\n");
    s.data(
        r#"
[[dotwarden.variants.".clash"]]
target = "family:debian"
file = "a"
[[dotwarden.variants.".clash"]]
target = "family:ubuntu-like"
file = "b"
[[dotwarden.variants.".fine"]]
target = "family:debian"
file = "a"
"#,
    );
    s.become_machine("multi");
    // Plain apply aborts as a whole...
    assert!(s.store.chezmoi(&["apply"]).is_err());
    // ...while --keep-going applies everything else and reports the one failure.
    let err = s.store.chezmoi(&["apply", "--keep-going"]).unwrap_err();
    assert!(format!("{err:#}").contains("ambiguous variants for .clash"));
    assert_eq!(
        fs::read_to_string(s.t.home.join(".plain")).unwrap(),
        "plain\n"
    );
    assert_eq!(fs::read_to_string(s.t.home.join(".fine")).unwrap(), "a\n");
    assert!(!s.t.home.join(".clash").exists());
}

#[test]
fn a_more_specific_match_beats_a_lower_level_tie() {
    // `multi` is a mint machine in two families. The two family variants tie at
    // family level, but the distribution variant is more specific and wins.
    let s = Spike::new(&fleet());
    for (id, body) in [
        ("a", "family-debian\n"),
        ("b", "family-ubuntu\n"),
        ("c", "dist-mint\n"),
        ("d", "machine\n"),
    ] {
        s.home_src(&format!(".dotwarden/variants/{id}"), body);
    }
    s.home_src("dot_pick.tmpl", &variant_template(".pick"));
    s.data(
        r#"
[[dotwarden.variants.".pick"]]
target = "family:debian"
file = "a"
[[dotwarden.variants.".pick"]]
target = "family:ubuntu-like"
file = "b"
[[dotwarden.variants.".pick"]]
target = "distribution:mint"
file = "c"
"#,
    );
    assert_eq!(s.rendered("multi", ".pick").unwrap(), "dist-mint\n");
    // Without the more specific entry the tie is an error again.
    let s2 = Spike::new(&fleet());
    s2.home_src(".dotwarden/variants/a", "x\n");
    s2.home_src("dot_pick.tmpl", &variant_template(".pick"));
    s2.data(
        r#"
[[dotwarden.variants.".pick"]]
target = "family:debian"
file = "a"
[[dotwarden.variants.".pick"]]
target = "family:ubuntu-like"
file = "a"
"#,
    );
    assert!(
        s2.rendered("multi", ".pick")
            .unwrap_err()
            .contains("ambiguous variants for .pick")
    );
    // A tie at the most specific level is still an error even with a lower winner.
    let s3 = Spike::new(&fleet());
    s3.home_src(".dotwarden/variants/a", "x\n");
    s3.home_src("dot_pick.tmpl", &variant_template(".pick"));
    s3.data(
        r#"
[[dotwarden.variants.".pick"]]
target = "platform:linux"
file = "a"
[[dotwarden.variants.".pick"]]
target = "machine:multi"
file = "a"
[[dotwarden.variants.".pick"]]
target = "machine:multi"
file = "a"
"#,
    );
    assert!(
        s3.rendered("multi", ".pick")
            .unwrap_err()
            .contains("ambiguous variants")
    );
}

fn variant_template(path: &str) -> String {
    format!(
        "{{{{- $v := includeTemplate \"dotwarden/variant\" (dict \"data\" . \"path\" \"{path}\") | trim -}}}}\
         {{{{- if $v }}}}{{{{ include (printf \".dotwarden/variants/%s\" $v) }}}}{{{{ else }}}}default\n{{{{ end -}}}}"
    )
}

#[test]
fn an_unknown_target_kind_in_one_entry_breaks_nothing_and_is_reported_by_doctor() {
    let s = Spike::new(&fleet());
    s.home_src(".dotwarden/variants/a", "known\n");
    s.home_src(".dotwarden/variants/b", "weird\n");
    s.home_src("dot_one.tmpl", &variant_template(".one"));
    s.home_src("dot_two.tmpl", &variant_template(".two"));
    s.home_src("dot_assigned", "x\n");
    s.home_src("dot_other", "x\n");
    s.data(
        r#"
[[dotwarden.variants.".one"]]
target = "weird:thing"
file = "b"
[[dotwarden.variants.".one"]]
target = "platform:linux"
file = "a"
[[dotwarden.variants.".two"]]
target = "planet:mars"
file = "b"
[dotwarden.assignments.".assigned"]
include = ["bogus:x", "machine:ubu"]
[dotwarden.assignments.".other"]
exclude = ["bogus:y"]
"#,
    );
    // The unknown entry is ignored: the valid one still selects, the other file is plain.
    assert_eq!(s.rendered("ubu", ".one").unwrap(), "known\n");
    assert_eq!(s.rendered("ubu", ".two").unwrap(), "default\n");
    assert_eq!(s.rendered("mac", ".one").unwrap(), "default\n");
    // An unknown include never matches; an unknown exclude never excludes.
    assert!(s.managed("ubu").contains(&".assigned".to_string()));
    assert!(!s.managed("mac").contains(&".assigned".to_string()));
    assert!(s.managed("mac").contains(&".other".to_string()));

    // `dw doctor` surfaces them as warnings without failing.
    s.store.write_local_machine("ubu").unwrap();
    s.store.write_chezmoi_config("ubu").unwrap();
    let report = dotwarden::doctor::run(&s.store);
    let warn = report
        .checks
        .iter()
        .find(|c| c.name == "targets")
        .expect("a targets finding");
    assert!(warn.warning && warn.ok);
    for t in ["weird:thing", "planet:mars", "bogus:x", "bogus:y"] {
        assert!(warn.detail.contains(t), "{t} not in {}", warn.detail);
    }
}

#[test]
fn retired_machines_have_no_targets() {
    let mut fleet = fleet();
    fleet.push(Machine {
        retired: true,
        ..machine("gone", "linux", "ubuntu", &["debian"])
    });
    let s = Spike::new(&fleet);
    s.home_src("dot_x", "x\n");
    s.become_machine("gone");
    let err = format!("{:#}", s.store.chezmoi(&["managed"]).unwrap_err());
    assert!(err.contains("machine \"gone\" is retired"), "{err}");
    // Live machines are unaffected, including one that reuses nothing of it.
    assert_eq!(s.managed("ubu"), [".x"]);
}
