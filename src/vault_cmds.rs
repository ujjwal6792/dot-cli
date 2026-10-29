//! Handlers of the vault commands, `values add-secret` and `encrypt` (spec §7).
//! Secrets are read from the terminal only; nothing here prints a value.

use std::process::ExitCode;

use anyhow::Result;
use serde_json::json;

use crate::cmds::{print_json, require_initialised};
use crate::error::DwError;
use crate::store::Store;
use crate::vault::{self, Prompt};

fn human_only(store: &Store, what: &str) -> Result<()> {
    if crate::changes::agent(store).is_some() {
        return Err(DwError::usage(format!("{what} is for humans; agents cannot do it")).into());
    }
    Ok(())
}

pub fn init(store: &Store, json: bool) -> Result<ExitCode> {
    require_initialised(store)?;
    human_only(store, "creating the vault")?;
    if vault::exists(store) {
        return Err(crate::track::refuse("the vault already exists".into()));
    }
    let pass = vault::read_new_passphrase(store, "New vault passphrase")?;
    let code = vault::init(store, &pass)?;
    if json {
        print_json(&json!({"vault": "unlocked", "recovery_code": code}))?;
    } else {
        println!("Vault created and unlocked (changes are staged; commit them).");
        println!("Recovery code (shown once; store it somewhere safe, offline):");
        println!("  {code}");
    }
    Ok(ExitCode::SUCCESS)
}

pub fn unlock(store: &Store, json: bool) -> Result<ExitCode> {
    require_initialised(store)?;
    human_only(store, "unlocking the vault")?;
    vault::recipient(store)?;
    let pass = vault::read_secret(store, Prompt::Passphrase, "Vault passphrase")?;
    vault::unlock(store, &pass)?;
    status(store, json)
}

pub fn lock(store: &Store, json: bool) -> Result<ExitCode> {
    vault::session::end(store)?;
    if json {
        print_json(&json!({"vault": "locked"}))?;
    } else {
        println!("Vault locked.");
    }
    Ok(ExitCode::SUCCESS)
}

pub fn status(store: &Store, json: bool) -> Result<ExitCode> {
    let exists = vault::exists(store);
    let s = vault::session::status(store);
    if json {
        print_json(&json!({
            "exists": exists,
            "unlocked": exists && s.unlocked,
            "expires_in_seconds": if exists { s.expires_in_seconds } else { None },
        }))?;
    } else {
        println!("Vault: {}", crate::status::vault_state(store));
    }
    Ok(ExitCode::SUCCESS)
}

pub fn passwd(store: &Store, json: bool) -> Result<ExitCode> {
    require_initialised(store)?;
    human_only(store, "changing the passphrase")?;
    vault::recipient(store)?;
    let old = vault::read_secret(store, Prompt::Passphrase, "Current passphrase")?;
    let new = vault::read_new_passphrase(store, "New passphrase")?;
    vault::passwd(store, &old, &new)?;
    done(
        json,
        "passphrase changed (identity.age rewritten and staged)",
    )
}

pub fn recover(store: &Store, json: bool) -> Result<ExitCode> {
    require_initialised(store)?;
    human_only(store, "recovering the vault")?;
    vault::recipient(store)?;
    let code = vault::read_secret(store, Prompt::RecoveryCode, "Recovery code")?;
    let new = vault::read_new_passphrase(store, "New passphrase")?;
    vault::recover(store, &code, &new)?;
    done(
        json,
        "new passphrase set from the recovery code; vault unlocked",
    )
}

fn done(json: bool, msg: &str) -> Result<ExitCode> {
    if json {
        print_json(&json!({"ok": true, "message": msg}))?;
    } else {
        println!("{msg}");
    }
    Ok(ExitCode::SUCCESS)
}

pub fn add_secret(
    store: &Store,
    name: &str,
    target: Option<&str>,
    placeholder: bool,
    dry_run: bool,
    json: bool,
) -> Result<ExitCode> {
    require_initialised(store)?;
    if placeholder && target.is_some() {
        return Err(DwError::usage("a placeholder has no target").into());
    }
    if dry_run {
        vault::recipient(store)?;
        crate::values::check_name(name)?;
        let what = if placeholder {
            format!("add placeholder {name} (name only)")
        } else {
            format!(
                "set secret {name}{} (value read from the terminal)",
                target.map_or(String::new(), |t| format!(" for {t}"))
            )
        };
        if json {
            print_json(&json!({"dry_run": true, "apply": true, "would": what}))?;
        } else {
            println!("Dry run: nothing was changed. Would {what}.");
        }
        return Ok(ExitCode::SUCCESS);
    }
    if placeholder {
        crate::secrets::add_placeholder(store, name)?;
        return done(
            json,
            &format!(
                "placeholder {name} added (staged); a human sets its value with `dw values add-secret {name}`"
            ),
        );
    }
    human_only(store, "setting a secret value")?;
    vault::recipient(store)?;
    let value = vault::read_secret(store, Prompt::SecretValue, &format!("Value of {name}"))?;
    crate::secrets::set(store, name, target, &value)?;
    done(json, &format!("secret {name} stored in the vault (staged)"))
}

pub fn unset_secret(
    store: &Store,
    name: &str,
    target: &str,
    dry_run: bool,
    json: bool,
) -> Result<ExitCode> {
    require_initialised(store)?;
    if dry_run {
        let meta = crate::secrets::read_meta(&store.repo());
        if !meta
            .get(name)
            .is_some_and(|m| m.targets.iter().any(|t| t == target))
        {
            return Err(crate::track::refuse(format!(
                "secret {name} has no override for {target}"
            )));
        }
        return done(
            json,
            &format!("dry run: would remove the override of {name} for {target}"),
        );
    }
    crate::secrets::unset_target(store, name, target)?;
    done(
        json,
        &format!("override of {name} for {target} removed (staged)"),
    )
}

pub fn upgrade_templates(
    store: &Store,
    apply: bool,
    dry_run: bool,
    json: bool,
) -> Result<ExitCode> {
    require_initialised(store)?;
    let ctx = crate::track::Ctx::load(store)?;
    let item = crate::encfile::plan_upgrade(store, &ctx, &crate::pending::requester(store))?;
    crate::cmds::submit(store, vec![item], apply, dry_run, json)
}

pub fn encrypt(
    store: &Store,
    path: &str,
    apply: bool,
    dry_run: bool,
    json: bool,
) -> Result<ExitCode> {
    require_initialised(store)?;
    let hp = crate::track::resolve_home_path(store, path, &std::env::current_dir()?)?;
    let ctx = crate::track::Ctx::load(store)?;
    let item = crate::encfile::plan(store, &ctx, &hp, &crate::pending::requester(store))?;
    crate::cmds::submit(store, vec![item], apply, dry_run, json)
}

/// Hidden helper `dw __secret NAME TARGETS`: prints the raw value for chezmoi.
pub fn helper_secret(store: &Store, name: &str, targets: &str) -> Result<ExitCode> {
    use std::io::Write;
    let v = crate::render::helper_secret(store, name, targets)?;
    std::io::stdout().write_all(v.as_bytes())?;
    Ok(ExitCode::SUCCESS)
}

/// Hidden helper `dw __file ID`: prints the decrypted file for chezmoi.
pub fn helper_file(store: &Store, id: &str) -> Result<ExitCode> {
    use std::io::Write;
    let v = crate::render::helper_file(store, id)?;
    std::io::stdout().write_all(&v)?;
    Ok(ExitCode::SUCCESS)
}
