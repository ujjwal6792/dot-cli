//! Local branches. Git protects the index; HOME is never a checkout target.
use crate::{
    pending::{Action, Item, Preview},
    store::Store,
    track,
};
use anyhow::{Result, bail};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Branch {
    pub name: String,
    pub current: bool,
    pub commit: String,
    pub default: bool,
}

pub fn default_name(store: &Store) -> String {
    std::fs::read_to_string(store.repo().join("dotwarden.toml"))
        .ok()
        .and_then(|s| toml::from_str::<toml::Table>(&s).ok())
        .and_then(|t| {
            t.get("default_branch")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "main".into())
}
pub fn list(store: &Store) -> Result<Vec<Branch>> {
    let default = default_name(store);
    let current = crate::remote::current_branch(store)?;
    Ok(store
        .git(&[
            "for-each-ref",
            "--format=%(refname:short) %(objectname)",
            "refs/heads/",
        ])?
        .stdout
        .lines()
        .filter_map(|l| l.split_once(' '))
        .map(|(name, commit)| Branch {
            name: name.into(),
            commit: commit.into(),
            current: name == current,
            default: name == default,
        })
        .collect())
}
fn valid(store: &Store, name: &str) -> Result<()> {
    if name.starts_with('-') {
        bail!("invalid branch name");
    }
    store.git(&["check-ref-format", "--branch", name])?;
    Ok(())
}
pub fn change(store: &Store, name: &str, create: bool, dry: bool) -> Result<Vec<String>> {
    let _lock = store.lock()?;
    track::ensure_repo_clean(store)?;
    valid(store, name)?;
    let branches = list(store)?;
    if create && branches.iter().any(|b| b.name == name) {
        bail!("branch already exists: {name}");
    }
    if !create && !branches.iter().any(|b| b.name == name) {
        bail!("no local branch: {name}");
    }
    let files = if create {
        vec![]
    } else {
        store
            .git(&[
                "diff",
                "--name-only",
                "-z",
                "HEAD",
                &format!("refs/heads/{name}"),
                "--",
            ])?
            .stdout
            .split('\0')
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect()
    };
    // Preview must also reject staged changes that the target could overwrite.
    if !create {
        let staged = store
            .git(&["diff", "--cached", "--name-only", "-z"])?
            .stdout;
        if staged
            .split('\0')
            .filter(|p| !p.is_empty())
            .any(|p| files.iter().any(|f| f == p))
        {
            bail!("switch would overlap staged source edits; commit or stash them first");
        }
    }
    if !dry {
        if create {
            store.git(&["branch", "--", name])?;
        } else {
            store.git(&["switch", "--no-guess", name])?;
        }
    }
    Ok(files)
}
pub fn check_delete(store: &Store, name: &str, force: bool) -> Result<()> {
    valid(store, name)?;
    if crate::remote::current_branch(store)? == name {
        bail!("cannot delete the current branch");
    }
    let hash = crate::history::resolve(store, &format!("refs/heads/{name}"))?;
    if !force
        && store
            .git(&["merge-base", "--is-ancestor", &hash, "HEAD"])
            .is_err()
    {
        bail!("branch {name} is unmerged; explicit --force and human approval are required");
    }
    Ok(())
}
pub fn plan_delete(store: &Store, name: &str, force: bool) -> Result<Item> {
    check_delete(store, name, force)?;
    track::new_item(
        store,
        &crate::pending::requester(store),
        Action::BranchDelete {
            name: name.into(),
            force,
        },
        format!(
            "Delete {}branch {name}",
            if force { "unmerged/forced " } else { "" }
        ),
        Preview {
            notes: vec!["HOME is unchanged; the branch tip is checked again at approval".into()],
            ..Default::default()
        },
        &track::Ctx::load(store)?,
    )
}
pub fn delete_locked(store: &Store, name: &str, force: bool) -> Result<()> {
    track::ensure_repo_clean(store)?;
    check_delete(store, name, force)?;
    store.git(&["branch", if force { "-D" } else { "-d" }, "--", name])?;
    Ok(())
}
