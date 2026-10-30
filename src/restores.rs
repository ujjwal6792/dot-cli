//! Historical candidates contain only commit IDs and fingerprints, never rendered values.
use crate::{
    changes::{self, Change, State},
    git_state::{HomeFile, SourceView},
    store::Store,
    track,
};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub path: String,
    pub commit: String,
    pub guard: String,
    pub home: String,
    pub desired: String,
    pub conflict: bool,
}
fn file(store: &Store) -> std::path::PathBuf {
    store.dirs.state_dir().join("restore-candidates.json")
}
pub fn list(store: &Store) -> Result<Vec<Candidate>> {
    match fs::read(file(store)) {
        Ok(b) => Ok(serde_json::from_slice(&b)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(e.into()),
    }
}
fn save(store: &Store, items: &[Candidate]) -> Result<()> {
    crate::fsutil::write_atomic(&file(store), serde_json::to_vec(items)?)
}
fn selected(paths: Option<&[String]>, path: &str) -> bool {
    paths.is_none_or(|ps| {
        ps.iter()
            .any(|p| p == path || path.starts_with(&format!("{p}/")))
    })
}
pub fn clear(store: &Store, paths: Option<&[String]>, dry: bool) -> Result<Vec<Candidate>> {
    let _lock = store.lock()?;
    let mut all = list(store)?;
    let chosen = all
        .iter()
        .filter(|c| selected(paths, &c.path))
        .cloned()
        .collect();
    all.retain(|c| !selected(paths, &c.path));
    if !dry {
        save(store, &all)?;
    }
    Ok(chosen)
}
pub fn plan(
    store: &Store,
    rev: &str,
    paths: Option<&[String]>,
    dry: bool,
) -> Result<Vec<Candidate>> {
    let _lock = store.lock()?;
    track::ensure_repo_clean(store)?;
    let hash = crate::history::resolve(store, rev)?;
    let view = SourceView::new(store, &hash)?;
    let historical = view.targets()?;
    let current = changes::targets(store)?;
    let normal = changes::scan(store, &track::Ctx::load(store)?)?;
    let guard = crate::git_state::fingerprint(store)?;
    let mut chosen = Vec::new();
    for (path, t) in historical.iter().filter(|(p, _)| selected(paths, p)) {
        if !current.contains_key(path) {
            continue;
        } // current AND historical assignments must receive it
        if t.link {
            bail!("~/{path}: historical symlink restore is unsupported");
        }
        HomeFile::read(store, path)?;
        chosen.push(Candidate {
            path: path.clone(),
            commit: hash.clone(),
            guard: guard.clone(),
            home: track::home_fingerprint(&store.dirs.home.join(path)),
            desired: t.sha(),
            conflict: normal.changes.iter().any(|c| {
                c.path == *path && matches!(c.state, State::Local | State::Staged | State::Conflict)
            }),
        });
    }
    for p in paths.unwrap_or_default() {
        if !chosen
            .iter()
            .any(|c| selected(Some(std::slice::from_ref(p)), &c.path))
        {
            bail!("~/{p} is not received in both historical and current assignments");
        }
    }
    if chosen.is_empty() {
        bail!("no eligible historical files");
    }
    crate::git_state::supported_templates(
        &view.store,
        &chosen.iter().map(|c| c.path.clone()).collect::<Vec<_>>(),
    )?;
    if !dry {
        let mut all = list(store)?;
        all.retain(|c| !chosen.iter().any(|n| n.path == c.path));
        all.extend(chosen.clone());
        save(store, &all)?;
    }
    Ok(chosen)
}
pub fn overlay(store: &Store, changes: &mut Vec<Change>) -> Result<()> {
    let candidates = list(store)?;
    if candidates.is_empty() {
        return Ok(());
    }
    let guard = crate::git_state::fingerprint(store)?;
    for c in candidates {
        let conflict = c.conflict
            || c.guard != guard
            || c.home != track::home_fingerprint(&store.dirs.home.join(&c.path));
        changes.push(Change {
            path: c.path,
            state: if conflict {
                State::Conflict
            } else {
                State::Incoming
            },
            kind: "restore",
        });
    }
    Ok(())
}
pub fn preview(store: &Store, path: &str) -> Result<String> {
    let Some(c) = list(store)?.into_iter().find(|c| c.path == path) else {
        bail!("no restore candidate");
    };
    let view = SourceView::new(store, &c.commit)?;
    let targets = view.targets()?;
    let Some(t) = targets.get(path) else {
        bail!("historical target is unavailable");
    };
    let home = HomeFile::read(store, path)?.text;
    let old = home.as_deref().map(|s| crate::conflicts::masked(store, s));
    let new = crate::conflicts::masked(&view.store, &t.text);
    Ok(format!(
        "Restore from {}\n{}",
        c.commit,
        changes::unified(path, old.as_deref(), Some(&new))
    ))
}
/// Applies only candidates; caller holds the mutation lock. Forward entries remain.
pub fn apply(store: &Store, paths: Option<&[String]>, dry: bool) -> Result<Vec<String>> {
    let all = list(store)?;
    let chosen: Vec<_> = all.iter().filter(|c| selected(paths, &c.path)).collect();
    if chosen.is_empty() {
        return Ok(vec![]);
    }
    track::ensure_repo_clean(store)?;
    let locked = crate::values::locked_files(store)?;
    if chosen.iter().any(|c| locked.contains(&c.path)) {
        return Err(crate::vault::locked());
    }
    let guard = crate::git_state::fingerprint(store)?;
    let current = changes::targets(store)?;
    let mut rendered = BTreeMap::new();
    let mut before = BTreeMap::new();
    for c in &chosen {
        if c.guard != guard || c.home != track::home_fingerprint(&store.dirs.home.join(&c.path)) {
            return Err(crate::error::DwError::new(
                crate::error::Kind::Stale,
                format!(
                    "~/{0}: stale restore candidate; clear or plan it again",
                    c.path
                ),
            )
            .into());
        }
        if c.conflict {
            return Err(crate::error::DwError::new(crate::error::Kind::Conflict, format!("~/{0}: restore collides with local/staged changes; clear candidate, preserve edits, then re-plan",c.path)).into());
        }
        if !current.contains_key(&c.path) {
            bail!("restore is no longer assigned to this machine");
        }
        let historical = SourceView::new(store, &c.commit)?.targets()?;
        let Some(t) = historical.get(&c.path) else {
            bail!("historical assignment changed");
        };
        if t.sha() != c.desired {
            bail!("historical rendering changed; re-plan the restore");
        }
        before.insert(c.path.clone(), HomeFile::read(store, &c.path)?);
        rendered.insert(
            c.path.clone(),
            HomeFile {
                text: Some(t.text.clone()),
                mode: t.perm as u32,
            },
        );
    }
    let affected: Vec<String> = chosen.iter().map(|c| c.path.clone()).collect();
    if dry {
        return Ok(affected);
    }
    crate::git_state::safety_snapshot(store, &affected, "restore-at")?;
    let backup = track::state_backup(store)?;
    let result = (|| -> Result<()> {
        for (p, f) in &rendered {
            f.write(store, p)?;
        }
        changes::restore_last_written(
            store,
            &affected.iter().map(String::as_str).collect::<BTreeSet<_>>(),
        )?;
        let keep: Vec<_> = all
            .iter()
            .filter(|c| !affected.contains(&c.path))
            .cloned()
            .collect();
        save(store, &keep)
    })();
    if let Err(e) = result {
        for (p, f) in before {
            f.write(store, &p)?;
        }
        track::state_restore(store, backup)?;
        return Err(e);
    }
    Ok(affected)
}
