//! The repository's source state seen as HOME paths: which target paths are
//! tracked, and which folders are tracked folders.
//!
//! chezmoi cannot answer "is this tracked" for a file that an assignment keeps
//! off this machine (`chezmoi managed` omits it), and tracking such a file again
//! would overwrite the shared version with this machine's content. So the source
//! tree is decoded here instead; `tests/track.rs` cross-checks the decoding
//! against `chezmoi managed`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::repo::DATA_FILE;

/// A tracked file or symlink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceEntry {
    /// Path of the source file relative to the repository root, e.g. `home/dot_zshrc`.
    pub source: String,
    pub symlink: bool,
}

/// What the repository says about one HOME path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrackState {
    /// A tracked file or symlink.
    Tracked,
    /// A folder recorded as a tracked folder.
    TrackedFolder,
    /// A folder holding tracked files that is not itself a tracked folder.
    ContainsTracked,
    /// An untracked file inside a tracked folder, not hidden: a change to stage.
    New,
    Untracked,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tracked {
    /// Target path (relative to HOME) to source entry.
    pub files: BTreeMap<String, SourceEntry>,
    /// Target paths of tracked folders.
    pub folders: BTreeSet<String>,
}

const DIR_PREFIXES: [&str; 3] = ["exact_", "private_", "readonly_"];
const FILE_PREFIXES: [&str; 5] = [
    "encrypted_",
    "private_",
    "readonly_",
    "empty_",
    "executable_",
];

/// Decode one source directory name into its target name.
fn decode_dir(name: &str) -> Option<String> {
    if name.starts_with('.') || name.starts_with("external_") || name.starts_with("remove_") {
        return None;
    }
    let mut s = name;
    while let Some(rest) = DIR_PREFIXES.iter().find_map(|p| s.strip_prefix(p)) {
        s = rest;
    }
    if let Some(rest) = s.strip_prefix("literal_") {
        return Some(rest.to_string());
    }
    Some(match s.strip_prefix("dot_") {
        Some(rest) => format!(".{rest}"),
        None => s.to_string(),
    })
}

/// Decode one source file name into its target name and whether it is a symlink.
/// Scripts and removals have no target.
fn decode_file(name: &str) -> Option<(String, bool)> {
    if name.starts_with('.') || name.starts_with("run_") || name.starts_with("remove_") {
        return None;
    }
    let mut s = name;
    let mut symlink = false;
    if let Some(rest) = s.strip_prefix("symlink_") {
        symlink = true;
        s = rest;
    } else if let Some(rest) = s
        .strip_prefix("create_")
        .or_else(|| s.strip_prefix("modify_"))
    {
        s = rest;
    }
    let mut encrypted = false;
    while let Some(rest) = FILE_PREFIXES.iter().find_map(|p| s.strip_prefix(p)) {
        encrypted |= s.starts_with("encrypted_");
        s = rest;
    }
    let mut target = if let Some(rest) = s.strip_prefix("literal_") {
        rest.to_string()
    } else if let Some(rest) = s.strip_prefix("dot_") {
        format!(".{rest}")
    } else {
        s.to_string()
    };
    if encrypted {
        for ext in [".age", ".asc"] {
            if let Some(t) = target.strip_suffix(ext) {
                target = t.to_string();
                break;
            }
        }
    }
    if let Some(t) = target.strip_suffix(".tmpl") {
        target = t.to_string();
    }
    (!target.is_empty()).then_some((target, symlink))
}

/// Target path and symlink flag of a source file path relative to the
/// repository root, e.g. `home/dot_config/app/dot_x` is `.config/app/.x`.
pub fn decode_source_path(repo: &Path, source: &str) -> Option<(String, bool)> {
    let rest = source.strip_prefix(&source_root(repo))?.strip_prefix('/')?;
    let mut parts: Vec<&str> = rest.split('/').collect();
    let file = parts.pop()?;
    let (name, symlink) = decode_file(file)?;
    let mut out = Vec::new();
    for p in parts {
        out.push(decode_dir(p)?);
    }
    out.push(name);
    Some((out.join("/"), symlink))
}

/// The source root directory name from `.chezmoiroot` (default `home`).
pub fn source_root(repo: &Path) -> String {
    fs::read_to_string(repo.join(".chezmoiroot"))
        .map(|s| s.trim().to_string())
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "home".to_string())
}

/// Tracked folders recorded in the data file (`dotwarden.tracked_folders`).
pub fn read_folders(repo: &Path) -> Result<BTreeSet<String>> {
    let path = repo.join(DATA_FILE);
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeSet::new()),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let data: toml::Table =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok(data
        .get("dotwarden")
        .and_then(|d| d.get("tracked_folders"))
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect())
}

/// Replace the recorded tracked folders (sorted), leaving the rest of the data
/// file untouched. Removes the key when the list is empty.
pub fn write_folders(repo: &Path, folders: &BTreeSet<String>) -> Result<()> {
    crate::repo::edit_data(repo, |doc| {
        let dw = crate::repo::dotwarden_table(doc)?;
        if folders.is_empty() {
            dw.remove("tracked_folders");
        } else {
            dw["tracked_folders"] = toml_edit::value(
                folders
                    .iter()
                    .map(String::as_str)
                    .collect::<toml_edit::Array>(),
            );
        }
        Ok(())
    })
}

/// Decode the repository checkout at `repo`. A repository without source state
/// yet is simply empty.
pub fn load(repo: &Path) -> Result<Tracked> {
    let mut files = BTreeMap::new();
    let root = source_root(repo);
    walk(&repo.join(&root), &root, "", &mut files)?;
    Ok(Tracked {
        files,
        folders: read_folders(repo)?,
    })
}

fn walk(
    dir: &Path,
    source_prefix: &str,
    target_prefix: &str,
    out: &mut BTreeMap<String, SourceEntry>,
) -> Result<()> {
    let rd = match fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e).with_context(|| format!("reading {}", dir.display())),
    };
    for item in rd.flatten() {
        let Ok(ft) = item.file_type() else { continue };
        let name = item.file_name().to_string_lossy().into_owned();
        let source = format!("{source_prefix}/{name}");
        if ft.is_dir() {
            if let Some(t) = decode_dir(&name) {
                let target = if target_prefix.is_empty() {
                    t
                } else {
                    format!("{target_prefix}/{t}")
                };
                walk(&item.path(), &source, &target, out)?;
            }
        } else if ft.is_file()
            && let Some((t, symlink)) = decode_file(&name)
        {
            let target = if target_prefix.is_empty() {
                t
            } else {
                format!("{target_prefix}/{t}")
            };
            out.insert(target, SourceEntry { source, symlink });
        }
    }
    Ok(())
}

impl Tracked {
    /// Tracked files at or below `rel` (a file itself, or everything inside a folder).
    pub fn under<'a>(
        &'a self,
        rel: &str,
    ) -> impl Iterator<Item = (&'a String, &'a SourceEntry)> + 'a {
        let exact = self.files.get_key_value(rel);
        let lo = format!("{rel}/");
        let hi = format!("{rel}0"); // '0' follows '/'
        exact
            .into_iter()
            .chain(self.files.range(lo..hi).map(|(k, v)| (k, v)))
    }

    pub fn is_tracked(&self, rel: &str) -> bool {
        self.files.contains_key(rel)
    }

    /// The closest proper ancestor of `rel` that is a recorded tracked folder.
    pub fn folder_above(&self, rel: &str) -> Option<&String> {
        let mut cur = rel;
        while let Some((parent, _)) = cur.rsplit_once('/') {
            if let Some(f) = self.folders.get(parent) {
                return Some(f);
            }
            cur = parent;
        }
        None
    }

    /// Tracked folders at or below `rel`.
    pub fn folders_under<'a>(&'a self, rel: &str) -> impl Iterator<Item = &'a String> + 'a {
        let lo = format!("{rel}/");
        let hi = format!("{rel}0");
        self.folders
            .get(rel)
            .into_iter()
            .chain(self.folders.range(lo..hi))
    }

    /// State of a HOME path (callers decide hidden-ness separately).
    pub fn state_of(&self, rel: &str, is_dir: bool) -> TrackState {
        if is_dir {
            if self.folders.contains(rel) {
                TrackState::TrackedFolder
            } else if self.under(rel).next().is_some() {
                TrackState::ContainsTracked
            } else if self.folder_above(rel).is_some() {
                TrackState::New
            } else {
                TrackState::Untracked
            }
        } else if self.is_tracked(rel) {
            TrackState::Tracked
        } else if self.folder_above(rel).is_some() {
            TrackState::New
        } else {
            TrackState::Untracked
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_attributes_and_names() {
        assert_eq!(decode_file("dot_zshrc"), Some((".zshrc".into(), false)));
        assert_eq!(
            decode_file("private_executable_dot_m700"),
            Some((".m700".into(), false))
        );
        assert_eq!(
            decode_file("empty_dot_empty"),
            Some((".empty".into(), false))
        );
        assert_eq!(decode_file("readonly_dot_ro"), Some((".ro".into(), false)));
        assert_eq!(
            decode_file("literal_dot_literal"),
            Some(("dot_literal".into(), false))
        );
        assert_eq!(decode_file("symlink_link"), Some(("link".into(), true)));
        assert_eq!(decode_file("dot_x.tmpl"), Some((".x".into(), false)));
        assert_eq!(
            decode_file("encrypted_dot_k.age"),
            Some((".k".into(), false))
        );
        assert_eq!(decode_file(".chezmoiignore"), None);
        assert_eq!(decode_file("run_once_setup.sh"), None);
        assert_eq!(decode_dir("private_dot_ssh"), Some(".ssh".into()));
        assert_eq!(decode_dir("exact_dot_config"), Some(".config".into()));
        assert_eq!(decode_dir(".dotwarden"), None);
        assert_eq!(decode_dir(".chezmoitemplates"), None);
    }

    #[test]
    fn range_queries_do_not_leak_into_siblings() {
        let mut t = Tracked::default();
        for p in ["a/b", "a/c/d", "a.txt", "a0", "ab/c"] {
            t.files.insert(
                p.into(),
                SourceEntry {
                    source: format!("home/{p}"),
                    symlink: false,
                },
            );
        }
        let under: Vec<_> = t.under("a").map(|(k, _)| k.as_str()).collect();
        assert_eq!(under, ["a/b", "a/c/d"]);
        assert_eq!(t.state_of("a", true), TrackState::ContainsTracked);
        assert_eq!(t.state_of("a.txt", false), TrackState::Tracked);
        assert_eq!(t.state_of("zz", false), TrackState::Untracked);
    }

    #[test]
    fn new_files_are_those_inside_tracked_folders() {
        let mut t = Tracked::default();
        t.folders.insert(".config/app".into());
        assert_eq!(t.state_of(".config/app", true), TrackState::TrackedFolder);
        assert_eq!(t.state_of(".config/app/new", false), TrackState::New);
        assert_eq!(t.state_of(".config/app/sub/x", false), TrackState::New);
        assert_eq!(t.state_of(".config/other", false), TrackState::Untracked);
        assert_eq!(t.folder_above(".config/app/sub/x").unwrap(), ".config/app");
        assert!(t.folder_above(".config/app").is_none());
    }

    #[test]
    fn folders_roundtrip_through_the_data_file_keeping_other_keys() {
        let d = tempfile::tempdir().unwrap();
        crate::repo::write_layout(d.path(), "main").unwrap();
        let mut set = BTreeSet::new();
        set.insert(".b".to_string());
        set.insert(".a/x".to_string());
        write_folders(d.path(), &set).unwrap();
        assert_eq!(read_folders(d.path()).unwrap(), set);
        let text = fs::read_to_string(d.path().join(DATA_FILE)).unwrap();
        assert!(text.contains("format = 1"), "{text}");
        write_folders(d.path(), &BTreeSet::new()).unwrap();
        assert!(read_folders(d.path()).unwrap().is_empty());
        assert!(
            !fs::read_to_string(d.path().join(DATA_FILE))
                .unwrap()
                .contains("tracked_folders")
        );
    }
}
