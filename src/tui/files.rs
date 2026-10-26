//! The HOME browser model (panel 2).

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use ratatui::widgets::ListState;

use crate::exec::CancelToken;
use crate::hide::HideRules;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: OsString,
    pub is_dir: bool,
    pub is_symlink: bool,
    /// Matched by a hide rule.
    pub hidden: bool,
}

impl Entry {
    pub fn display(&self) -> String {
        self.name.to_string_lossy().into_owned()
    }
}

/// Read one directory. Directories first, then case-insensitive by name.
pub fn list_dir(dir: &Path, rules: &HideRules, cancel: &CancelToken) -> Result<Vec<Entry>, String> {
    let rd = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut out = Vec::new();
    for item in rd.flatten() {
        if cancel.is_cancelled() {
            return Err("cancelled".into());
        }
        let path = item.path();
        let is_symlink = item.file_type().map(|t| t.is_symlink()).unwrap_or(false);
        let is_dir = fs::metadata(&path).map(|m| m.is_dir()).unwrap_or(false);
        out.push(Entry {
            name: item.file_name(),
            is_dir,
            is_symlink,
            hidden: rules.is_hidden(&path, is_dir),
        });
    }
    out.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.display().to_lowercase().cmp(&b.display().to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(out)
}

/// Text for the main view's preview of `path` (read off the event loop).
pub fn preview(path: &Path, cancel: &CancelToken) -> Result<String, String> {
    const LIMIT: usize = 16 * 1024;
    let meta = fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.is_dir() {
        let mut names: Vec<String> = fs::read_dir(path)
            .map_err(|e| e.to_string())?
            .flatten()
            .map(|e| {
                let mut n = e.file_name().to_string_lossy().into_owned();
                if e.path().is_dir() {
                    n.push('/');
                }
                n
            })
            .collect();
        names.sort();
        let total = names.len();
        names.truncate(200);
        let mut s = format!("{total} item(s)\n\n");
        s.push_str(&names.join("\n"));
        return Ok(s);
    }
    if !meta.is_file() {
        return Ok("(special file)".into());
    }
    if cancel.is_cancelled() {
        return Err("cancelled".into());
    }
    use std::io::Read;
    let mut buf = Vec::new();
    fs::File::open(path)
        .and_then(|f| f.take(LIMIT as u64).read_to_end(&mut buf))
        .map_err(|e| e.to_string())?;
    if buf.contains(&0) {
        return Ok(format!("(binary file, {} bytes)", meta.len()));
    }
    let mut text = String::from_utf8_lossy(&buf).into_owned();
    if meta.len() as usize > LIMIT {
        text.push_str("\n… (truncated)");
    }
    Ok(text)
}

#[derive(Debug)]
pub struct FilesState {
    pub root: PathBuf,
    pub cwd: PathBuf,
    all: Vec<Entry>,
    pub selected: usize,
    pub show_hidden: bool,
    pub loading: bool,
    pub error: Option<String>,
    /// Name to select once the next listing arrives.
    reselect: Option<OsString>,
    pub list_state: ListState,
}

impl FilesState {
    pub fn new(root: PathBuf) -> Self {
        Self {
            cwd: root.clone(),
            root,
            all: Vec::new(),
            selected: 0,
            show_hidden: false,
            loading: true,
            error: None,
            reselect: None,
            list_state: ListState::default(),
        }
    }

    pub fn visible(&self) -> Vec<&Entry> {
        self.all
            .iter()
            .filter(|e| self.show_hidden || !e.hidden)
            .collect()
    }

    pub fn selected_entry(&self) -> Option<&Entry> {
        self.visible().get(self.selected).copied()
    }

    /// Path of `e` relative to HOME, `/`-separated.
    pub fn rel_of(&self, e: &Entry) -> String {
        let dir = self.cwd.strip_prefix(&self.root).unwrap_or(Path::new(""));
        let dir = dir.to_string_lossy();
        if dir.is_empty() {
            e.display()
        } else {
            format!("{dir}/{}", e.display())
        }
    }

    pub fn selected_path(&self) -> Option<PathBuf> {
        self.selected_entry().map(|e| self.cwd.join(&e.name))
    }

    pub fn move_by(&mut self, delta: isize) {
        let n = self.visible().len();
        if n == 0 {
            self.selected = 0;
            return;
        }
        self.selected = self.selected.saturating_add_signed(delta).min(n - 1);
    }

    pub fn select_first(&mut self) {
        self.selected = 0;
    }

    pub fn select_last(&mut self) {
        self.selected = self.visible().len().saturating_sub(1);
    }

    /// Enter the selected directory. Returns whether the directory changed.
    pub fn enter(&mut self) -> bool {
        match self.selected_entry() {
            Some(e) if e.is_dir => {
                self.cwd = self.cwd.join(&e.name);
                self.begin_load(None);
                true
            }
            _ => false,
        }
    }

    /// Go to the parent directory (never above the root), reselecting the folder left.
    pub fn leave(&mut self) -> bool {
        if self.cwd == self.root {
            return false;
        }
        let left = self.cwd.file_name().map(OsString::from);
        if let Some(parent) = self.cwd.parent() {
            self.cwd = parent.to_path_buf();
        }
        self.begin_load(left);
        true
    }

    pub fn begin_load(&mut self, reselect: Option<OsString>) {
        self.loading = true;
        self.error = None;
        self.all.clear();
        self.selected = 0;
        self.reselect = reselect;
    }

    /// Toggle showing items matched by hide rules, keeping the selection on the same item.
    pub fn toggle_hidden(&mut self) {
        let name = self.selected_entry().map(|e| e.name.clone());
        self.show_hidden = !self.show_hidden;
        self.reselect = None;
        self.selected = name
            .and_then(|n| self.visible().iter().position(|e| e.name == n))
            .unwrap_or(0)
            .min(self.visible().len().saturating_sub(1));
    }

    pub fn set_listing(&mut self, result: Result<Vec<Entry>, String>) {
        self.loading = false;
        match result {
            Ok(entries) => {
                self.all = entries;
                self.error = None;
                self.selected = self
                    .reselect
                    .take()
                    .and_then(|n| self.visible().iter().position(|e| e.name == n))
                    .unwrap_or(0);
            }
            Err(e) => {
                self.all.clear();
                self.error = Some(e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let p = d.path();
        fs::create_dir_all(p.join(".config/nvim")).unwrap();
        fs::create_dir_all(p.join(".cache/x")).unwrap();
        fs::create_dir_all(p.join("Projects")).unwrap();
        fs::write(p.join(".zshrc"), "export A=1\n").unwrap();
        fs::write(p.join(".zsh_history"), "ls\n").unwrap();
        fs::write(p.join("bin.dat"), [0u8, 1, 2]).unwrap();
        d
    }

    fn load(s: &mut FilesState, rules: &HideRules) {
        let r = list_dir(&s.cwd, rules, &CancelToken::new());
        s.set_listing(r);
    }

    #[test]
    fn hide_rules_filter_and_dot_toggles() {
        let d = tree();
        let rules = HideRules::defaults(d.path()).unwrap();
        let mut s = FilesState::new(d.path().to_path_buf());
        load(&mut s, &rules);
        let names = |s: &FilesState| s.visible().iter().map(|e| e.display()).collect::<Vec<_>>();
        assert_eq!(names(&s), [".config", "Projects", ".zshrc", "bin.dat"]);
        s.toggle_hidden();
        assert!(names(&s).contains(&".cache".to_string()));
        assert!(names(&s).contains(&".zsh_history".to_string()));
        s.toggle_hidden();
        assert_eq!(names(&s).len(), 4);
    }

    #[test]
    fn enter_and_leave_reselect_the_folder() {
        let d = tree();
        let rules = HideRules::defaults(d.path()).unwrap();
        let mut s = FilesState::new(d.path().to_path_buf());
        load(&mut s, &rules);
        assert!(s.enter()); // .config
        assert_eq!(s.cwd, d.path().join(".config"));
        load(&mut s, &rules);
        assert_eq!(s.selected_entry().unwrap().display(), "nvim");
        assert!(s.leave());
        load(&mut s, &rules);
        assert_eq!(s.selected_entry().unwrap().display(), ".config");
        assert!(!s.leave(), "cannot go above the root");
    }

    #[test]
    fn enter_on_a_file_does_nothing() {
        let d = tree();
        let rules = HideRules::defaults(d.path()).unwrap();
        let mut s = FilesState::new(d.path().to_path_buf());
        load(&mut s, &rules);
        s.select_last();
        assert!(!s.enter());
    }

    #[test]
    fn previews() {
        let d = tree();
        let c = CancelToken::new();
        assert!(
            preview(&d.path().join(".zshrc"), &c)
                .unwrap()
                .contains("export A=1")
        );
        assert!(
            preview(&d.path().join("bin.dat"), &c)
                .unwrap()
                .contains("binary")
        );
        assert!(
            preview(&d.path().join(".config"), &c)
                .unwrap()
                .contains("nvim/")
        );
    }
}
