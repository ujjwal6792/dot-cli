//! Reviewed removal of untracked HOME entries through the operating system Trash.
use crate::{backend::Snapshot, config::Config};
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, os::unix::fs::MetadataExt, path::Path};

pub fn prepare(c: &Config, snapshot: &Snapshot, paths: &[String]) -> Result<(Vec<String>, String)> {
    if paths.is_empty() {
        bail!("Select an untracked original file or folder to move to Trash");
    }
    let mut targets = BTreeSet::new();
    for input in paths {
        let relative = c.relative(input)?;
        let path = c.target(&relative)?;
        // Protect both the source/state and any selected folder containing them.
        if [&c.source, &c.state]
            .iter()
            .any(|protected| path.starts_with(protected) || protected.starts_with(&path))
            || Path::new(&relative)
                .components()
                .any(|part| part.as_os_str() == ".git")
        {
            bail!("Cannot move dot source, state, or Git internals to Trash: {relative}");
        }
        let prefix = format!("{relative}/");
        if snapshot
            .managed
            .keys()
            .chain(snapshot.managed_directories.iter())
            .chain(c.enabled.iter())
            .any(|managed| managed == &relative || managed.starts_with(&prefix))
        {
            bail!(
                "{relative} is managed by dot or contains managed files. Move to Trash only supports untracked originals. Use r to remove saved copies, or R to restore originals."
            );
        }
        fs::symlink_metadata(&path)
            .with_context(|| format!("Original no longer exists: {relative}"))?;
        targets.insert(relative);
    }
    // A selected folder already includes its selected descendants.
    let paths: Vec<_> = targets
        .iter()
        .filter(|path| {
            !targets
                .iter()
                .any(|parent| *path != parent && path.starts_with(&format!("{parent}/")))
        })
        .cloned()
        .collect();
    let mut digest = Sha256::new();
    let mut count = 0;
    for relative in &paths {
        for entry in walkdir::WalkDir::new(c.target(relative)?)
            .follow_links(false)
            .follow_root_links(false)
            .sort_by_file_name()
        {
            let entry = entry?;
            count += 1;
            if count > 10000 {
                bail!("Select a smaller folder (more than 10000 entries to review for Trash)");
            }
            let metadata = fs::symlink_metadata(entry.path())?;
            let name = entry
                .path()
                .strip_prefix(&c.home)?
                .as_os_str()
                .as_encoded_bytes();
            digest.update((name.len() as u64).to_le_bytes());
            digest.update(name);
            for number in [
                metadata.dev(),
                metadata.ino(),
                metadata.mode() as u64,
                metadata.len(),
                metadata.mtime() as u64,
                metadata.mtime_nsec() as u64,
                metadata.ctime() as u64,
                metadata.ctime_nsec() as u64,
            ] {
                digest.update(number.to_le_bytes());
            }
            if metadata.file_type().is_symlink() {
                digest.update(fs::read_link(entry.path())?.as_os_str().as_encoded_bytes());
            }
        }
    }
    Ok((paths, format!("{:x}", digest.finalize())))
}

pub fn preview(c: &Config, paths: &[String]) -> Result<String> {
    let mut text = String::from(
        "Move these untracked originals to your system Trash.\nFolders include all contents; symbolic links move only the link.\nSaved copies and Git are unchanged. Restore originals using your system Trash.\n\n",
    );
    for relative in paths {
        let path = c.target(relative)?;
        let meta = fs::symlink_metadata(&path)?;
        let kind = if meta.file_type().is_symlink() {
            "LINK"
        } else if meta.is_dir() {
            "FOLDER"
        } else {
            "FILE"
        };
        text.push_str(&format!("{kind}  {}\n", path.display()));
    }
    Ok(text)
}

pub fn move_to_trash(path: &Path) -> Result<()> {
    trash::delete(path)
        .with_context(|| format!("Could not move {} to system Trash", path.display()))
}
