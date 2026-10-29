//! The fixed templates Dotwarden writes once into a new repository (ADR 0005).
//! See docs/spec.md §3 "Verified mechanics" for how they are exercised.

/// `home/.chezmoiignore`: evaluates assignments for the current machine.
pub const CHEZMOIIGNORE: &str = include_str!("../assets/home/.chezmoiignore");
/// `home/.chezmoitemplates/dotwarden/targets`: the current machine's targets.
pub const TARGETS: &str = include_str!("../assets/home/.chezmoitemplates/dotwarden/targets");
/// `home/.chezmoitemplates/dotwarden/select`: specificity selection shared by
/// variants and (later) values.
pub const SELECT: &str = include_str!("../assets/home/.chezmoitemplates/dotwarden/select");
/// `home/.chezmoitemplates/dotwarden/variant`: most specific variant selection.
pub const VARIANT: &str = include_str!("../assets/home/.chezmoitemplates/dotwarden/variant");

/// `home/.chezmoitemplates/dotwarden/value`: a variable's value for this machine.
pub const VALUE: &str = include_str!("../assets/home/.chezmoitemplates/dotwarden/value");

/// `home/.chezmoitemplates/dotwarden/file`: an encrypted whole file.
pub const FILE: &str = include_str!("../assets/home/.chezmoitemplates/dotwarden/file");

/// Files of the fixed template set, relative to the repository root.
pub const FILES: &[(&str, &str)] = &[
    ("home/.chezmoiignore", CHEZMOIIGNORE),
    ("home/.chezmoitemplates/dotwarden/targets", TARGETS),
    ("home/.chezmoitemplates/dotwarden/select", SELECT),
    ("home/.chezmoitemplates/dotwarden/variant", VARIANT),
    ("home/.chezmoitemplates/dotwarden/value", VALUE),
    ("home/.chezmoitemplates/dotwarden/file", FILE),
];

/// Version of the fixed template set, recorded as `templates = N` in
/// `dotwarden.toml`. Version 2 added secrets (`dotwarden/value`) and encrypted
/// files (`dotwarden/file`); stored files that reference variables are unchanged.
pub const VERSION: i64 = 2;

/// The recorded template version of a repository (1 when absent).
pub fn recorded(repo: &std::path::Path) -> i64 {
    std::fs::read_to_string(repo.join("dotwarden.toml"))
        .ok()
        .and_then(|t| toml::from_str::<toml::Table>(&t).ok())
        .and_then(|t| t.get("templates")?.as_integer())
        .unwrap_or(1)
}

/// Fixed template files (repository-relative) that are missing or differ from
/// this version's, or all stale ones when the recorded version is older.
pub fn outdated(repo: &std::path::Path) -> Vec<String> {
    FILES
        .iter()
        .filter(|(rel, content)| {
            std::fs::read_to_string(repo.join(rel)).ok().as_deref() != Some(*content)
        })
        .map(|(rel, _)| rel.to_string())
        .collect()
}

pub fn needs_upgrade(repo: &std::path::Path) -> bool {
    recorded(repo) < VERSION || !outdated(repo).is_empty()
}
