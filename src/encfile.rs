//! Encrypted whole files (spec §7): `dw encrypt PATH` keeps the HOME content
//! of a tracked file in `vault/files/<id>.age` (encrypted to the vault
//! recipient, so no unlock is needed) and stores a stub template in its
//! place that chezmoi renders through the helper. Staging an edited file
//! re-encrypts it; the stub never changes.

use std::collections::BTreeSet;
use std::fs;

use anyhow::{Context, Result};

use crate::pending::{Action, FileChange, Item, Preview};
use crate::render;
use crate::store::Store;
use crate::track::{Ctx, HomePath, new_item, refuse};
use crate::values::RefStage;
use crate::vault;

pub fn age_file(id: &str) -> String {
    vault::rel(&format!("files/{id}.age"))
}

fn tilde(rel: &str) -> String {
    format!("~/{rel}")
}

fn change(action: &str, target: &str, source: &str) -> FileChange {
    FileChange {
        action: action.into(),
        target: target.into(),
        source: source.into(),
        kind: "file".into(),
    }
}

/// Plan storing the tracked file `hp` encrypted.
pub fn plan(store: &Store, ctx: &Ctx, hp: &HomePath, requester: &str) -> Result<Item> {
    vault::recipient(store)?;
    let entry = ctx.tracked.files.get(&hp.rel).ok_or_else(|| {
        refuse(format!(
            "{} is not a tracked file; track it first",
            tilde(&hp.rel)
        ))
    })?;
    if entry.symlink {
        return Err(refuse(format!("{} is a symlink", tilde(&hp.rel))));
    }
    if entry.source.ends_with(".tmpl") {
        return Err(refuse(format!(
            "{} is a template (references or already encrypted)",
            tilde(&hp.rel)
        )));
    }
    if crate::variants::Variants::read(&store.repo())?
        .0
        .contains_key(&hp.rel)
    {
        return Err(refuse(format!("{} has variants", tilde(&hp.rel))));
    }
    let meta = fs::symlink_metadata(&hp.abs)
        .map_err(|_| refuse(format!("{} does not exist", tilde(&hp.rel))))?;
    if !meta.is_file() {
        return Err(refuse(format!("{} is not a regular file", tilde(&hp.rel))));
    }
    let raw: [u8; 8] = rand::random();
    let id: String = raw.iter().map(|b| format!("{b:02x}")).collect();
    let new_source = format!("{}.tmpl", entry.source);
    let mut notes = vec![
        format!(
            "the content of {} is encrypted to the vault recipient (no unlock needed); the repository keeps a stub template",
            tilde(&hp.rel)
        ),
        "viewing or applying it on a machine needs an unlock session".into(),
    ];
    let in_history = store
        .git(&["log", "--oneline", "-1", "--", entry.source.as_str()])
        .is_ok_and(|o| !o.stdout.trim().is_empty());
    if in_history {
        notes.push(format!(
            "warning: earlier commits still contain the plaintext of {}",
            tilde(&hp.rel)
        ));
    }
    let preview = Preview {
        files: vec![
            change("remove", &hp.rel, &entry.source),
            change("add", &hp.rel, &new_source),
            change("add", &hp.rel, &age_file(&id)),
        ],
        notes,
        ..Preview::default()
    };
    let action = Action::Encrypt {
        key: hp.rel.clone(),
        path: hp.rel.clone(),
        file_id: id,
        source: entry.source.clone(),
    };
    new_item(
        store,
        requester,
        action,
        format!("Store {} encrypted", tilde(&hp.rel)),
        preview,
        ctx,
    )
}

/// Make the repository change of an `Encrypt` item; returns the files changed.
pub fn exec(store: &Store, action: &Action) -> Result<BTreeSet<String>> {
    let Action::Encrypt {
        path,
        file_id,
        source,
        ..
    } = action
    else {
        unreachable!("not an encrypt action");
    };
    let repo = store.repo();
    let plain =
        fs::read(store.dirs.home.join(path)).with_context(|| format!("reading ~/{path}"))?;
    let age = age_file(file_id);
    let new_source = format!("{source}.tmpl");
    for (rel, bytes) in [
        (age.clone(), vault::encrypt(store, &plain)?),
        (new_source.clone(), render::file_stub(file_id).into_bytes()),
    ] {
        let p = repo.join(&rel);
        if let Some(d) = p.parent() {
            fs::create_dir_all(d)?;
        }
        fs::write(&p, bytes)?;
    }
    fs::remove_file(repo.join(source))?;
    Ok(BTreeSet::from([age, new_source, source.clone()]))
}

/// Staging an edited encrypted file: the new HOME content, encrypted.
pub fn stage_plan(store: &Store, rel: &str, id: &str) -> Result<RefStage, String> {
    let plain = fs::read(store.dirs.home.join(rel)).map_err(|e| e.to_string())?;
    let age = age_file(id);
    let same = vault::session::identity(store)
        .and_then(|i| {
            let old = fs::read(store.repo().join(&age)).ok()?;
            vault::decrypt(&i, &old).ok()
        })
        .is_some_and(|old| old == plain);
    let bytes = vault::encrypt(store, &plain).map_err(|e| format!("{e:#}"))?;
    Ok(RefStage {
        rel: rel.into(),
        dest: age,
        bytes,
        changes: Vec::new(),
        unchanged: same,
        extra: Vec::new(),
        remove: None,
    })
}
