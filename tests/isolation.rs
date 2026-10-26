//! Spec §3 / ADR 0002: a user's own chezmoi installation is never read or modified.

mod common;

use common::{TestEnv, snapshot};
use std::fs;

/// A HOME that looks like it belongs to a chezmoi user.
fn chezmoi_user() -> TestEnv {
    let mut t = TestEnv::new();
    let cfg = t.home.join(".config/chezmoi");
    let src = t.home.join(".local/share/chezmoi");
    fs::create_dir_all(&cfg).unwrap();
    fs::create_dir_all(src.join("dot_config")).unwrap();
    fs::write(src.join("dot_from_users_chezmoi"), "users own file\n").unwrap();
    fs::write(
        cfg.join("chezmoi.toml"),
        format!(
            "sourceDir = {:?}\n[data]\nsentinel = \"users-config\"\n",
            src.display().to_string()
        ),
    )
    .unwrap();
    fs::write(cfg.join("chezmoistate.boltdb"), b"not a real database").unwrap();
    // Environment that would redirect an unscrubbed chezmoi.
    t.set("CHEZMOI_SOURCE_DIR", src.clone());
    t.set("CHEZMOI_CONFIG_FILE", cfg.join("chezmoi.toml"));
    t.set("CHEZMOI_DESTINATION_DIR", t.root.path().join("elsewhere"));
    t
}

#[test]
fn existing_chezmoi_config_and_source_are_untouched() {
    let t = chezmoi_user();
    let cfg = t.home.join(".config/chezmoi");
    let src = t.home.join(".local/share/chezmoi");
    let before = (snapshot(&cfg), snapshot(&src));

    t.dw_ok(&["init", "--name", "isolated"]);
    t.dw_ok(&["status"]);
    t.dw_ok(&["doctor"]);

    // Exercise chezmoi directly through the store: read and write paths.
    let store = t.store();
    store.chezmoi(&["managed"]).unwrap();
    store.chezmoi(&["status"]).unwrap();
    store.chezmoi(&["verify"]).ok();
    store.chezmoi(&["apply"]).unwrap();
    store.chezmoi(&["data", "--format", "json"]).unwrap();

    assert_eq!(
        (snapshot(&cfg), snapshot(&src)),
        before,
        "user's chezmoi dirs changed"
    );
}

#[test]
fn the_users_chezmoi_data_and_source_are_never_read() {
    let t = chezmoi_user();
    t.dw_ok(&["init", "--name", "isolated"]);
    let store = t.store();

    let data = store.chezmoi(&["data", "--format", "json"]).unwrap().stdout;
    assert!(
        !data.contains("users-config"),
        "user's chezmoi.toml data leaked"
    );
    assert!(data.contains("dotwardenMachine"));

    store.chezmoi(&["apply"]).unwrap();
    assert!(
        !t.home.join(".from_users_chezmoi").exists(),
        "applied from the user's own source directory"
    );
    let managed = store.chezmoi(&["managed"]).unwrap().stdout;
    assert!(!managed.contains("from_users_chezmoi"), "{managed}");
    assert_eq!(
        store.chezmoi(&["source-path"]).unwrap().stdout.trim(),
        t.dirs().repo().join("home").display().to_string()
    );
}

#[test]
fn chezmoi_env_is_scrubbed_and_all_paths_are_explicit() {
    let t = chezmoi_user();
    let cmd = t.store().chezmoi_command();
    assert!(
        cmd.get_envs()
            .all(|(k, _)| !k.to_string_lossy().starts_with("CHEZMOI_")),
        "CHEZMOI_* leaked into the child environment"
    );
    let args: Vec<String> = cmd
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    for flag in [
        "--config",
        "--source",
        "--persistent-state",
        "--cache",
        "--destination",
    ] {
        let i = args
            .iter()
            .position(|a| a == flag)
            .unwrap_or_else(|| panic!("{flag} missing"));
        assert!(
            std::path::Path::new(&args[i + 1]).is_absolute(),
            "{flag} must be an absolute path"
        );
        assert!(
            !args[i + 1].contains(".config/chezmoi")
                && !args[i + 1].ends_with(".local/share/chezmoi"),
            "{flag} points at the user's chezmoi: {}",
            args[i + 1]
        );
    }
}

#[test]
fn a_fresh_home_gets_no_chezmoi_directories() {
    let t = TestEnv::new();
    t.dw_ok(&["init", "--name", "fresh"]);
    let store = t.store();
    fs::write(t.dirs().repo().join("home/dot_probe"), "probe\n").unwrap();
    store.chezmoi(&["apply"]).unwrap();
    store.chezmoi(&["managed"]).unwrap();
    assert_eq!(
        fs::read_to_string(t.home.join(".probe")).unwrap(),
        "probe\n"
    );
    assert!(!t.home.join(".config/chezmoi").exists());
    assert!(!t.home.join(".local/share/chezmoi").exists());
    assert!(!t.home.join(".cache/chezmoi").exists());
    assert!(
        t.home
            .join(".local/state/dotwarden/chezmoistate.boltdb")
            .exists()
    );
}
