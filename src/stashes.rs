//! Dotwarden stashes: exact source index plus independent tracked HOME payload.
use crate::{
    changes,
    git_state::{HomeFile, SourceView},
    store::Store,
    track,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    os::unix::fs::PermissionsExt,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stash {
    pub id: String,
    pub base: String,
    pub index_commit: String,
    pub paths: Vec<String>,
    pub index_paths: Vec<String>,
    pub encrypted: bool,
    pub clean: BTreeMap<String, String>,
}
#[derive(Serialize, Deserialize)]
struct Payload {
    home: BTreeMap<String, HomeFile>,
}
fn root(store: &Store) -> std::path::PathBuf {
    store.dirs.state_dir().join("stashes")
}
fn dir(store: &Store, id: &str) -> Result<std::path::PathBuf> {
    if id.is_empty() || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') {
        bail!("invalid stash id");
    }
    Ok(root(store).join(id))
}
pub fn list(store: &Store) -> Result<Vec<Stash>> {
    if !root(store).exists() {
        return Ok(vec![]);
    }
    let mut items = Vec::new();
    for e in fs::read_dir(root(store))? {
        let path = e?.path().join("meta.json");
        if path.exists() {
            items.push(serde_json::from_slice::<Stash>(&fs::read(path)?)?);
        }
    }
    items.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(items)
}
pub fn get(store: &Store, id: Option<&str>) -> Result<Stash> {
    let all = list(store)?;
    all.into_iter()
        .find(|s| id.is_none_or(|id| s.id == id))
        .context("stash not found")
}
pub fn push(store: &Store, dry: bool) -> Result<Stash> {
    let _lock = store.lock()?;
    track::ensure_repo_clean(store)?;
    if !crate::restores::list(store)?.is_empty() {
        bail!("review or clear restore candidates before stashing");
    }
    let changes = changes::classify(store)?;
    if changes
        .iter()
        .any(|c| matches!(c.state, changes::State::Incoming | changes::State::Conflict))
    {
        bail!("apply or resolve Incoming/Conflicts before stashing");
    }
    let mut paths: Vec<String> = changes
        .iter()
        .filter(|c| {
            c.kind != "repo" && matches!(c.state, changes::State::Local | changes::State::Staged)
        })
        .map(|c| c.path.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let index_paths: Vec<String> = store
        .git(&["diff", "--cached", "--name-only", "-z"])?
        .stdout
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect();
    if paths.is_empty() && index_paths.is_empty() {
        bail!("nothing to stash (untracked HOME files are excluded)");
    }
    let base = crate::history::resolve(store, "HEAD")?;
    let view = SourceView::new(store, &base)?;
    let baseline = view.targets()?;
    let current = changes::targets(store)?;
    for (p, t) in &current {
        if baseline
            .get(p)
            .is_some_and(|b| b.text != t.text || b.perm != t.perm || b.link != t.link)
            && !paths.contains(p)
        {
            paths.push(p.clone());
        }
    }
    let encrypted = crate::vault::exists(store);
    if index_paths
        .iter()
        .any(|p| p == &crate::vault::rel("recipient.txt"))
    {
        bail!(
            "stash does not support a new or changed vault identity; commit vault initialization/rotation first"
        );
    }
    crate::git_state::supported_templates(store, &paths)?;
    crate::git_state::supported_templates(&view.store, &paths)?;
    if encrypted {
        crate::vault::session::require(store)?;
    }
    let mut home = BTreeMap::new();
    let mut clean_home = BTreeMap::new();
    for path in &paths {
        let f = HomeFile::read(store, path)?;
        if !encrypted && let Some(t) = &f.text {
            crate::git_state::safe_plain(store, t)?;
        }
        if baseline.get(path).is_some_and(|t| t.link) || current.get(path).is_some_and(|t| t.link) {
            bail!("~/{path}: stash requires regular files");
        }
        clean_home.insert(
            path.clone(),
            baseline.get(path).map_or(
                HomeFile {
                    text: None,
                    mode: 0o644,
                },
                |t| HomeFile {
                    text: Some(t.text.clone()),
                    mode: t.perm as u32,
                },
            ),
        );
        home.insert(path.clone(), f);
    }
    // Source edits stay in stored form; never capture rendered HOME in Git.
    for path in &index_paths {
        if let Ok(t) = fs::read_to_string(store.repo().join(path)) {
            crate::git_state::safe_plain(store, &t)
                .context("unsafe plaintext source edit; wire/encrypt it before stashing")?;
        }
    }
    let mut stash = Stash {
        id: format!(
            "{}-{:016x}",
            crate::snapshot::now_unix(),
            rand::random::<u64>()
        ),
        base,
        index_commit: String::new(),
        paths,
        index_paths,
        encrypted,
        clean: BTreeMap::new(),
    };
    if dry {
        return Ok(stash);
    }
    let tree = track::index_tree(store)?;
    let backup = track::state_backup(store)?;
    crate::git_state::safety_snapshot(store, &stash.paths, "stash-push")?;
    let d = dir(store, &stash.id)?;
    fs::create_dir_all(&d)?;
    fs::set_permissions(&d, fs::Permissions::from_mode(0o700))?;
    let bytes = serde_json::to_vec(&Payload { home: home.clone() })?;
    let bytes = if encrypted {
        crate::vault::encrypt(store, &bytes)?
    } else {
        bytes
    };
    crate::fsutil::write_atomic(&d.join("payload"), bytes)?;
    fs::set_permissions(d.join("payload"), fs::Permissions::from_mode(0o600))?;
    stash.index_commit = store
        .git(&[
            "commit-tree",
            &tree,
            "-p",
            &stash.base,
            "-m",
            "Dotwarden stash index",
        ])?
        .stdout
        .trim()
        .into();
    let reference = format!("refs/dotwarden/stashes/{}", stash.id);
    store.git(&["update-ref", &reference, &stash.index_commit])?;
    // Write recovery metadata before changing either side.
    crate::fsutil::write_atomic(&d.join("meta.json"), serde_json::to_vec(&stash)?)?;
    let result = (|| -> Result<()> {
        store.git(&["read-tree", "--reset", "-u", "HEAD"])?;
        for (p, f) in &clean_home {
            f.write(store, p)?;
        }
        changes::restore_last_written(store, &stash.paths.iter().map(String::as_str).collect())?;
        for p in &stash.paths {
            stash
                .clean
                .insert(p.clone(), track::home_fingerprint(&store.dirs.home.join(p)));
        }
        crate::fsutil::write_atomic(&d.join("meta.json"), serde_json::to_vec(&stash)?)?;
        Ok(())
    })();
    if let Err(e) = result {
        track::rollback(store, &tree)?;
        for (p, f) in home {
            f.write(store, &p)?;
        }
        track::state_restore(store, backup)?;
        return Err(e.context("stash retained for recovery; push rolled back"));
    }
    Ok(stash)
}
pub fn pop(store: &Store, id: Option<&str>, dry: bool) -> Result<Stash> {
    let _lock = store.lock()?;
    track::ensure_repo_clean(store)?;
    let stash = get(store, id)?;
    if crate::history::resolve(store, "HEAD")? != stash.base
        || store
            .git(&[
                "--no-optional-locks",
                "diff",
                "--cached",
                "--quiet",
                "HEAD",
                "--",
            ])
            .is_err()
    {
        bail!(
            "stash conflicts with the current HEAD/index; stash retained (return to its base with a clean index)"
        );
    }
    for p in &stash.paths {
        if stash.clean.get(p) != Some(&track::home_fingerprint(&store.dirs.home.join(p))) {
            return Err(crate::error::DwError::new(
                crate::error::Kind::Conflict,
                format!("~/{p} changed since stash push; stash retained"),
            )
            .into());
        }
    }
    let d = dir(store, &stash.id)?;
    let mut payload = fs::read(d.join("payload"))?;
    if stash.encrypted {
        payload = crate::vault::decrypt(&crate::vault::session::require(store)?, &payload)?;
    }
    let saved: Payload = serde_json::from_slice(&payload)?;
    for p in saved.home.keys() {
        crate::git_state::checked_path(store, p)?;
    }
    if dry {
        return Ok(stash);
    }
    let before: BTreeMap<String, HomeFile> = stash
        .paths
        .iter()
        .map(|p| Ok((p.clone(), HomeFile::read(store, p)?)))
        .collect::<Result<_>>()?;
    crate::git_state::safety_snapshot(store, &stash.paths, "stash-pop")?;
    let result = (|| -> Result<()> {
        store.git(&["read-tree", "--reset", "-u", &stash.index_commit])?;
        for (p, f) in &saved.home {
            f.write(store, p)?;
        }
        Ok(())
    })();
    if let Err(e) = result {
        store.git(&["read-tree", "--reset", "-u", "HEAD"])?;
        for (p, f) in before {
            f.write(store, &p)?;
        }
        return Err(e.context("pop rolled back; stash retained"));
    }
    drop_locked(store, &stash.id)?;
    Ok(stash)
}
pub fn drop_locked(store: &Store, id: &str) -> Result<()> {
    let stash = get(store, Some(id))?;
    store.git(&[
        "update-ref",
        "-d",
        &format!("refs/dotwarden/stashes/{id}"),
        &stash.index_commit,
    ])?;
    fs::remove_dir_all(dir(store, id)?)?;
    Ok(())
}
pub fn plan_drop(store: &Store, id: Option<&str>) -> Result<crate::pending::Item> {
    let stash = get(store, id)?;
    track::new_item(
        store,
        &crate::pending::requester(store),
        crate::pending::Action::StashDrop {
            id: stash.id.clone(),
        },
        format!(
            "Drop stash {} (HOME: {}; index: {})",
            stash.id,
            stash.paths.join(", "),
            stash.index_paths.join(", ")
        ),
        crate::pending::Preview::default(),
        &track::Ctx::load(store)?,
    )
}
