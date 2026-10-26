//! Registry and data-file edits keep comments and ordering, and are atomic.

mod common;

use common::TestEnv;
use dotwarden::machine::{Detected, Machine};
use std::fs;

const HAND_EDITED: &str = r#"# My machines. Keep this comment!
[dotwarden]
format = 1

# assignments I wrote by hand
[dotwarden.assignments.".zshrc"]
include = ["platform:linux"]   # linux only

[dotwarden.variables]
font_size = 14 # large
"#;

fn detected() -> Detected {
    Detected {
        platform: "linux".into(),
        distribution: "ubuntu".into(),
        families: vec!["debian".into()],
    }
}

#[test]
fn registering_a_machine_preserves_comments_and_ordering() {
    let t = TestEnv::new();
    let store = t.store();
    dotwarden::repo::init(&store, "main").unwrap();
    let data = store.repo().join(dotwarden::repo::DATA_FILE);
    fs::write(&data, HAND_EDITED).unwrap();

    dotwarden::repo::register_detected(&store, "alpha", detected(), false).unwrap();
    dotwarden::repo::register_detected(&store, "beta", detected(), false).unwrap();

    let after = fs::read_to_string(&data).unwrap();
    assert!(
        after.starts_with(HAND_EDITED.trim_end_matches('\n')),
        "hand-written part changed:\n{after}"
    );
    for kept in [
        "# My machines. Keep this comment!",
        "# assignments I wrote by hand",
        "# linux only",
        "# large",
    ] {
        assert!(after.contains(kept), "{kept}");
    }
    let names: Vec<_> = dotwarden::repo::read_registry(&store.repo())
        .unwrap()
        .into_iter()
        .map(|m| m.name)
        .collect();
    assert_eq!(names, ["alpha", "beta"]);

    // Updating one entry in place changes only that entry.
    let mut registry = dotwarden::repo::read_registry(&store.repo()).unwrap();
    registry[0].families = vec!["debian".into(), "ubuntu-like".into()];
    dotwarden::repo::write_registry(&store.repo(), &registry).unwrap();
    let again = fs::read_to_string(&data).unwrap();
    assert!(again.contains("# linux only") && again.contains("# large"));
    assert_eq!(
        dotwarden::repo::read_registry(&store.repo()).unwrap()[0].families,
        ["debian", "ubuntu-like"]
    );
    let lines_before: Vec<_> = after.lines().collect();
    let lines_after: Vec<_> = again.lines().collect();
    assert_eq!(
        lines_before.len(),
        lines_after.len(),
        "only a value changed in place"
    );
}

#[test]
fn rewriting_an_unchanged_registry_is_a_no_op_on_the_text() {
    let t = TestEnv::new();
    let store = t.store();
    dotwarden::repo::init(&store, "main").unwrap();
    let data = store.repo().join(dotwarden::repo::DATA_FILE);
    fs::write(&data, HAND_EDITED).unwrap();
    let m = Machine {
        name: "alpha".into(),
        platform: "linux".into(),
        distribution: "ubuntu".into(),
        families: vec!["debian".into()],
        retired: false,
    };
    dotwarden::repo::write_registry(&store.repo(), std::slice::from_ref(&m)).unwrap();
    let once = fs::read_to_string(&data).unwrap();
    dotwarden::repo::write_registry(&store.repo(), &[m]).unwrap();
    assert_eq!(fs::read_to_string(&data).unwrap(), once);
}

#[test]
fn generated_files_are_written_atomically_without_leftovers() {
    let t = TestEnv::new();
    t.dw_ok(&["init", "--name", "atomic"]);
    let dirs = t.dirs();
    for dir in [
        dirs.data_dir(),
        dirs.state_dir(),
        dirs.repo().join("home/.chezmoidata"),
    ] {
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".tmp."))
            .collect();
        assert!(leftovers.is_empty(), "{dir:?}: {leftovers:?}");
    }
    // Registering again (adopt) rewrites the generated config only when it differs.
    let cfg = dirs.chezmoi_config();
    let before = fs::metadata(&cfg).unwrap().modified().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));
    t.store().write_chezmoi_config("atomic").unwrap();
    assert_eq!(fs::metadata(&cfg).unwrap().modified().unwrap(), before);
}
