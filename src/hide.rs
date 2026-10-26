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
    /// Literal paths (relative to the root) named by `!` rules. A path hidden by
    /// a parent rule is still shown when it is one of these, below one, or a
    /// folder leading to one: gitignore itself cannot re-include below an
    /// ignored folder.
    unhidden: Vec<PathBuf>,
    /// Absolute paths that are always hidden (Dotwarden's own directories,
    /// wherever XDG puts them).
    always_hidden: Vec<PathBuf>,
}

/// The literal path of a `!pattern` rule, when it names one (anchored, no wildcards).
fn literal_negation(line: &str) -> Option<PathBuf> {
    let rest = line.trim_end().strip_prefix('!')?;
    let anchored = rest.starts_with('/') || rest.trim_end_matches('/').contains('/');
    let path = rest.trim_start_matches('/').trim_end_matches('/');
    let literal = !path.is_empty() && !path.contains(['*', '?', '[', '\\']);
    (anchored && literal).then(|| PathBuf::from(path))
}

impl HideRules {
    /// Build from rule text, with patterns relative to `root` (HOME).
    pub fn parse(root: &Path, text: &str) -> Result<Self> {
        let mut b = GitignoreBuilder::new(root);
        let mut unhidden = Vec::new();
        for line in text.lines() {
            b.add_line(None, line)?;
            unhidden.extend(literal_negation(line));
        }
        Ok(Self {
            root: root.to_path_buf(),
            matcher: b.build()?,
            unhidden,
            always_hidden: Vec::new(),
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

    /// The rules of the store: the repository's `hide` file plus Dotwarden's
    /// own directories.
    pub fn for_store(store: &crate::store::Store) -> Result<Self> {
        Ok(Self::load(&store.dirs.home, &store.repo())?.always_hide(store.dirs.own_dirs()))
    }

    /// Why `path` is hidden, for error messages; `None` when it is not hidden.
    pub fn why_hidden(&self, path: &Path, is_dir: bool) -> Option<String> {
        if !self.is_hidden(path, is_dir) {
            return None;
        }
        if self.always_hidden.iter().any(|h| path.starts_with(h)) {
            return Some("it is inside one of dotwarden's own directories".to_string());
        }
        let m = self.matcher.matched_path_or_any_parents(path, is_dir);
        Some(match m.inner() {
            Some(g) => format!("it matches the hide rule `{}`", g.original()),
            None => "it is hidden by a hide rule".to_string(),
        })
    }

    /// Always hide these absolute paths and everything below them.
    pub fn always_hide(mut self, paths: impl IntoIterator<Item = PathBuf>) -> Self {
        self.always_hidden.extend(paths);
        self
    }

    /// Whether `path` (absolute, under the root) or any parent is hidden.
    pub fn is_hidden(&self, path: &Path, is_dir: bool) -> bool {
        if !path.starts_with(&self.root) || path == self.root {
            return false;
        }
        if self.always_hidden.iter().any(|h| path.starts_with(h)) {
            return true;
        }
        if !self
            .matcher
            .matched_path_or_any_parents(path, is_dir)
            .is_ignore()
        {
            return false;
        }
        let Ok(rel) = path.strip_prefix(&self.root) else {
            return true;
        };
        for n in &self.unhidden {
            if (rel == n || n.starts_with(rel)) && self.root.join(n).exists() {
                return false;
            }
            if let Ok(below) = rel.strip_prefix(n) {
                // Under an un-hidden path: hidden only by a rule matching the
                // path itself or a folder between the un-hidden path and it.
                let mut cur = self.root.join(n);
                let parts: Vec<_> = below.components().collect();
                for (i, c) in parts.iter().enumerate() {
                    cur.push(c);
                    let dir = i + 1 < parts.len() || is_dir;
                    if self.matcher.matched(&cur, dir).is_ignore() {
                        return true;
                    }
                }
                return false;
            }
        }
        true
    }
}

/// The `hide` file, relative to the repository root.
pub const HIDE_FILE: &str = "hide";

/// A rule must be one non-empty, non-comment gitignore line.
pub fn validate_pattern(pattern: &str) -> Result<()> {
    let bad = |why: &str| -> Result<()> {
        Err(crate::error::DwError::usage(format!("invalid hide rule {pattern:?}: {why}")).into())
    };
    if pattern.trim().is_empty() {
        return bad("it is empty");
    }
    if pattern.starts_with('#') {
        return bad("a leading # starts a comment; escape it as \\#");
    }
    if pattern.chars().any(char::is_control) {
        return bad("it contains a newline or another control character");
    }
    if pattern != pattern.trim() {
        return bad("leading or trailing whitespace is not significant in hide rules");
    }
    let mut b = GitignoreBuilder::new("/");
    if let Err(e) = b.add_line(None, pattern) {
        return bad(&e.to_string());
    }
    Ok(())
}

/// The rule lines of the repository's `hide` file (comments and blanks excluded).
pub fn rule_lines(repo: &Path) -> Result<Vec<String>> {
    let text = match std::fs::read_to_string(repo.join(HIDE_FILE)) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    Ok(text
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect())
}

pub fn has_rule(repo: &Path, pattern: &str) -> Result<bool> {
    Ok(rule_lines(repo)?.iter().any(|l| l == pattern))
}

/// Append `pattern` to the `hide` file (atomic).
pub fn add_rule(repo: &Path, pattern: &str) -> Result<()> {
    validate_pattern(pattern)?;
    let path = repo.join(HIDE_FILE);
    let mut text = std::fs::read_to_string(&path).unwrap_or_default();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(pattern);
    text.push('\n');
    crate::fsutil::write_atomic(&path, text)
}

/// Remove every line equal to `pattern` from the `hide` file (atomic).
pub fn remove_rule(repo: &Path, pattern: &str) -> Result<()> {
    let path = repo.join(HIDE_FILE);
    let text = std::fs::read_to_string(&path)?;
    let mut out: String = text
        .lines()
        .filter(|l| *l != pattern)
        .collect::<Vec<_>>()
        .join("\n");
    out.push('\n');
    crate::fsutil::write_atomic(&path, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_editing_keeps_comments_and_other_rules() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join(HIDE_FILE), "# c\n.cache/\nfoo").unwrap();
        add_rule(d.path(), "bar/").unwrap();
        assert_eq!(rule_lines(d.path()).unwrap(), [".cache/", "foo", "bar/"]);
        remove_rule(d.path(), "foo").unwrap();
        let text = std::fs::read_to_string(d.path().join(HIDE_FILE)).unwrap();
        assert_eq!(text, "# c\n.cache/\nbar/\n");
        assert!(has_rule(d.path(), "bar/").unwrap());
        for bad in ["", "  ", "#x", "a\nb", " a", "a "] {
            assert!(validate_pattern(bad).is_err(), "{bad:?}");
        }
        validate_pattern("\\#x").unwrap();
        validate_pattern("!/Library/Application Support/Foo/").unwrap();
    }

    #[test]
    fn why_hidden_names_the_rule() {
        let r = HideRules::defaults(Path::new("/h")).unwrap();
        let why = r.why_hidden(Path::new("/h/.cache/x"), false).unwrap();
        assert!(why.contains(".cache/"), "{why}");
        assert!(r.why_hidden(Path::new("/h/.zshrc"), false).is_none());
        let r = r.always_hide([PathBuf::from("/h/own")]);
        assert!(
            r.why_hidden(Path::new("/h/own/x"), false)
                .unwrap()
                .contains("own directories")
        );
    }

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
    }

    #[test]
    fn library_is_hidden_except_explicitly_unhidden_paths() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        for p in [
            "Library/Caches",
            "Library/Preferences",
            "Library/Application Support/Code/User",
            "Library/Application Support/Slack",
        ] {
            std::fs::create_dir_all(root.join(p)).unwrap();
        }
        let h = |r: &HideRules, p: &str, dir| r.is_hidden(&root.join(p), dir);
        let r = HideRules::defaults(root).unwrap();
        // The folders leading to an un-hidden app are reachable ...
        assert!(!h(&r, "Library", true));
        assert!(!h(&r, "Library/Application Support", true));
        assert!(!h(&r, "Library/Application Support/Code", true));
        assert!(!h(&r, "Library/Application Support/Code/User", true));
        // ... and nothing beside them is.
        assert!(h(&r, "Library/Caches", true));
        assert!(h(&r, "Library/Preferences", true));
        assert!(h(&r, "Library/Application Support/Slack", true));
        assert!(h(&r, "Library/Application Support/Slack/x", false));
        // Without any existing un-hidden path, the whole of Library is hidden.
        let bare = HideRules::parse(root, "/Library/\n").unwrap();
        assert!(h(&bare, "Library", true));
        assert!(h(&bare, "Library/Application Support/Code", true));
        let none =
            HideRules::parse(root, "/Library/\n!/Library/Application Support/Missing/\n").unwrap();
        assert!(h(&none, "Library", true));
    }

    #[test]
    fn rules_inside_an_unhidden_path_still_apply() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        std::fs::create_dir_all(root.join("Library/App/Code/User")).unwrap();
        let r = HideRules::parse(
            root,
            "/Library/\n!/Library/App/Code/\n/Library/App/Code/Cache/\n",
        )
        .unwrap();
        assert!(!r.is_hidden(&root.join("Library/App/Code/User"), true));
        assert!(r.is_hidden(&root.join("Library/App/Code/Cache"), true));
        assert!(r.is_hidden(&root.join("Library/App/Code/Cache/x"), false));
    }

    #[test]
    fn always_hidden_paths_win_wherever_xdg_puts_them() {
        let r = rules("").always_hide([PathBuf::from("/h/xdg/data/dotwarden")]);
        assert!(r.is_hidden(Path::new("/h/xdg/data/dotwarden"), true));
        assert!(r.is_hidden(Path::new("/h/xdg/data/dotwarden/repo/x"), false));
        assert!(!r.is_hidden(Path::new("/h/xdg/data"), true));
        assert!(!r.is_hidden(Path::new("/h/xdg/data/other"), true));
    }

    #[test]
    fn literal_negations() {
        assert_eq!(literal_negation("!/a/b/"), Some(PathBuf::from("a/b")));
        assert_eq!(literal_negation("!a/b"), Some(PathBuf::from("a/b")));
        assert_eq!(literal_negation("!keep.log"), None);
        assert_eq!(literal_negation("!/a/*"), None);
        assert_eq!(literal_negation("/a/b"), None);
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
