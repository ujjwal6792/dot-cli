//! Locating the bundled chezmoi, git and delta (spec §2, ADR 0002).
//!
//! Bundle layout (`<prefix>` is the directory above the one holding the binary):
//!
//! ```text
//! <prefix>/bin/dotwarden
//! <prefix>/libexec/chezmoi
//! <prefix>/libexec/git/{bin/git,libexec/git-core,share/git-core/templates,...}
//! <prefix>/libexec/delta
//! ```

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use crate::paths::Env;

/// Overrides the libexec directory (development and tests).
pub const LIBEXEC_ENV: &str = "DOTWARDEN_LIBEXEC";

#[derive(Debug, Clone)]
pub struct Toolchain {
    libexec: PathBuf,
}

impl Toolchain {
    pub fn at(libexec: impl Into<PathBuf>) -> Self {
        Self {
            libexec: libexec.into(),
        }
    }

    /// Find libexec: `$DOTWARDEN_LIBEXEC`, then `<exe>/../libexec` (bundle),
    /// then the source checkout's git-ignored `libexec/` (development builds).
    pub fn resolve(env: &Env) -> Result<Self> {
        let mut tried = Vec::new();
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Some(v) = env.get(std::ffi::OsStr::new(LIBEXEC_ENV)) {
            candidates.push(PathBuf::from(v));
        }
        if let Ok(exe) = std::env::current_exe() {
            let exe = exe.canonicalize().unwrap_or(exe);
            if let Some(prefix) = exe.parent().and_then(Path::parent) {
                candidates.push(prefix.join("libexec"));
            }
        }
        if cfg!(debug_assertions) {
            candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("libexec"));
        }
        for c in candidates {
            let t = Self::at(&c);
            if t.chezmoi().is_file() && t.git_bin().is_file() {
                return Ok(t);
            }
            tried.push(c.display().to_string());
        }
        bail!(
            "bundled tools not found (looked in: {}); \
             run scripts/fetch-tools in a source checkout or reinstall dotwarden",
            tried.join(", ")
        )
    }

    pub fn libexec(&self) -> &Path {
        &self.libexec
    }
    pub fn chezmoi(&self) -> PathBuf {
        self.libexec.join("chezmoi")
    }
    pub fn git_root(&self) -> PathBuf {
        self.libexec.join("git")
    }
    pub fn git_bin(&self) -> PathBuf {
        self.git_root().join("bin").join("git")
    }
    pub fn delta(&self) -> PathBuf {
        self.libexec.join("delta")
    }

    /// Environment variables dugite's portable git needs, plus the isolation
    /// settings from spec §3. Applied on top of the scrubbed environment.
    pub fn git_env(&self) -> Vec<(&'static str, PathBuf)> {
        let root = self.git_root();
        let mut v = vec![
            ("GIT_EXEC_PATH", root.join("libexec").join("git-core")),
            (
                "GIT_TEMPLATE_DIR",
                root.join("share").join("git-core").join("templates"),
            ),
        ];
        let cert = root.join("ssl").join("cert.pem");
        if cert.is_file() {
            v.push(("GIT_SSL_CAINFO", cert));
        }
        v
    }
}

/// `GIT_*` variables that are passed through to git; everything else with that
/// prefix could redirect git away from the Dotwarden repository, so it is dropped.
const GIT_PASSTHROUGH: &[&str] = &[
    "GIT_SSH",
    "GIT_SSH_COMMAND",
    "GIT_ASKPASS",
    "GIT_TERMINAL_PROMPT",
    "GIT_SSL_CAINFO",
    "GIT_SSL_NO_VERIFY",
    "GIT_CURL_VERBOSE",
    "GIT_TRACE",
    "GIT_TRACE_CURL",
];

/// Whether `key` must be removed from the environment of a git invocation.
pub fn is_scrubbed_for_git(key: &str) -> bool {
    key.starts_with("GIT_") && !GIT_PASSTHROUGH.contains(&key)
}

/// Whether `key` must be removed from the environment of a chezmoi invocation.
pub fn is_scrubbed_for_chezmoi(key: &str) -> bool {
    key.starts_with("CHEZMOI_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_paths() {
        let t = Toolchain::at("/p/libexec");
        assert_eq!(t.chezmoi(), Path::new("/p/libexec/chezmoi"));
        assert_eq!(t.git_bin(), Path::new("/p/libexec/git/bin/git"));
        assert_eq!(t.delta(), Path::new("/p/libexec/delta"));
    }

    #[test]
    fn git_env_points_into_the_bundle() {
        let t = Toolchain::at("/p/libexec");
        let env = t.git_env();
        assert!(env.iter().any(
            |(k, v)| *k == "GIT_EXEC_PATH" && v == Path::new("/p/libexec/git/libexec/git-core")
        ));
    }

    #[test]
    fn libexec_override_wins_and_missing_tools_are_explained() {
        let dir = tempfile::tempdir().unwrap();
        let env_with = |p: &Path| -> Env { [(LIBEXEC_ENV.into(), p.into())].into_iter().collect() };
        std::fs::create_dir_all(dir.path().join("git/bin")).unwrap();
        std::fs::write(dir.path().join("chezmoi"), "").unwrap();
        std::fs::write(dir.path().join("git/bin/git"), "").unwrap();
        let t = Toolchain::resolve(&env_with(dir.path())).unwrap();
        assert_eq!(t.libexec(), dir.path());

        // An override that is incomplete is skipped, and the error names where we looked.
        let empty = tempfile::tempdir().unwrap();
        if let Err(e) = Toolchain::resolve(&env_with(empty.path())) {
            let msg = e.to_string();
            assert!(
                msg.contains(&empty.path().display().to_string()) && msg.contains("fetch-tools")
            );
        }
    }

    #[test]
    fn scrubbing_rules() {
        assert!(is_scrubbed_for_git("GIT_DIR"));
        assert!(is_scrubbed_for_git("GIT_CONFIG_GLOBAL"));
        assert!(!is_scrubbed_for_git("GIT_SSH_COMMAND"));
        assert!(!is_scrubbed_for_git("PATH"));
        assert!(is_scrubbed_for_chezmoi("CHEZMOI_SOURCE_DIR"));
        assert!(!is_scrubbed_for_chezmoi("HOME"));
    }
}
