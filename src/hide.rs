//! Hide rules: shared gitignore-syntax patterns that keep noise out of the file
//! browser and prevent tracking (GLOSSARY: Hide rule).

use std::path::{Path, PathBuf};

use anyhow::Result;
use ignore::gitignore::{Gitignore, GitignoreBuilder};

/// Hide rules written to a new repository's `hide` file.
pub const DEFAULT_RULES: &str = include_str!("../assets/hide.default");

pub struct HideRules {
    root: PathBuf,
    matcher: Gitignore,
}

impl HideRules {
    /// Build from rule text, with patterns relative to `root` (HOME).
    pub fn parse(root: &Path, text: &str) -> Result<Self> {
        let mut b = GitignoreBuilder::new(root);
        for line in text.lines() {
            b.add_line(None, line)?;
        }
        Ok(Self {
            root: root.to_path_buf(),
            matcher: b.build()?,
        })
    }

    pub fn defaults(root: &Path) -> Result<Self> {
        Self::parse(root, DEFAULT_RULES)
    }

    /// Load the repository's `hide` file, falling back to the defaults when the
    /// repository does not exist yet.
    pub fn load(root: &Path, repo: &Path) -> Result<Self> {
        match std::fs::read_to_string(repo.join("hide")) {
            Ok(text) => Self::parse(root, &text),
            Err(_) => Self::defaults(root),
        }
    }

    /// Whether `path` (absolute, under the root) or any parent is hidden.
    pub fn is_hidden(&self, path: &Path, is_dir: bool) -> bool {
        if !path.starts_with(&self.root) || path == self.root {
            return false;
        }
        self.matcher
            .matched_path_or_any_parents(path, is_dir)
            .is_ignore()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(text: &str) -> HideRules {
        HideRules::parse(Path::new("/h"), text).unwrap()
    }

    #[test]
    fn defaults_hide_caches_and_histories_but_not_dotfiles() {
        let r = HideRules::defaults(Path::new("/h")).unwrap();
        assert!(r.is_hidden(Path::new("/h/.cache"), true));
        assert!(r.is_hidden(Path::new("/h/.cache/x/y"), false));
        assert!(r.is_hidden(Path::new("/h/projects/app/node_modules"), true));
        assert!(r.is_hidden(Path::new("/h/.zsh_history"), false));
        assert!(r.is_hidden(Path::new("/h/Library/Caches"), true));
        assert!(!r.is_hidden(Path::new("/h/.zshrc"), false));
        assert!(!r.is_hidden(Path::new("/h/.config/nvim/init.lua"), false));
        assert!(!r.is_hidden(Path::new("/h/Library"), true));
    }

    #[test]
    fn negation_unhides() {
        let r = rules("/Library/*\n!/Library/Preferences\n");
        assert!(r.is_hidden(Path::new("/h/Library/Caches"), true));
        assert!(!r.is_hidden(Path::new("/h/Library/Preferences"), true));
    }

    #[test]
    fn root_and_outside_paths_are_never_hidden() {
        let r = rules("*\n");
        assert!(!r.is_hidden(Path::new("/h"), true));
        assert!(!r.is_hidden(Path::new("/elsewhere/x"), false));
    }
}
