//! The bundled (dugite-native) git works through our wrapper.

mod common;

use common::TestEnv;
use std::fs;

#[test]
fn reports_the_pinned_version() {
    let t = TestEnv::new();
    let v = t.git(&t.home, &["--version"]);
    assert!(v.contains(dotwarden::pins::git_version()), "{v}");
}

#[test]
fn does_not_depend_on_a_system_git() {
    // PATH holds no git at all.
    let mut t = TestEnv::new();
    t.set("PATH", "/nonexistent");
    let v = t.git(&t.home, &["--version"]);
    assert!(v.contains("git version"), "{v}");
    t.git(&t.home, &["init", "--quiet", "x"]);
}

#[test]
fn clone_over_file_url_and_plain_path() {
    let t = TestEnv::new();
    let src = t.home.join("src");
    fs::create_dir(&src).unwrap();
    t.git(&src, &["init", "--quiet"]);
    fs::write(src.join("f"), "hello").unwrap();
    t.git(&src, &["add", "f"]);
    t.git(&src, &["commit", "--quiet", "-m", "first"]);

    for (name, url) in [
        ("viafile", format!("file://{}", src.display())),
        ("viapath", src.display().to_string()),
    ] {
        t.git(&t.home, &["clone", "--quiet", &url, name]);
        assert_eq!(
            fs::read_to_string(t.home.join(name).join("f")).unwrap(),
            "hello"
        );
    }
}

/// Needs the network: set DOTWARDEN_NET_TESTS=1.
#[test]
fn clone_over_https() {
    if std::env::var_os("DOTWARDEN_NET_TESTS").is_none() {
        eprintln!("SKIPPED clone_over_https: set DOTWARDEN_NET_TESTS=1 to run");
        return;
    }
    let t = TestEnv::new();
    t.git(
        &t.home,
        &[
            "clone",
            "--quiet",
            "--depth",
            "1",
            "https://github.com/octocat/Hello-World.git",
            "hw",
        ],
    );
    assert!(t.home.join("hw/README").is_file());
}
