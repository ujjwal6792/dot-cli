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

/// Files of the fixed template set, relative to the repository root.
pub const FILES: &[(&str, &str)] = &[
    ("home/.chezmoiignore", CHEZMOIIGNORE),
    ("home/.chezmoitemplates/dotwarden/targets", TARGETS),
    ("home/.chezmoitemplates/dotwarden/select", SELECT),
    ("home/.chezmoitemplates/dotwarden/variant", VARIANT),
];
