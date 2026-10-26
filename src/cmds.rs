//! Handlers of the track, untrack, pending, hide and ls commands (spec §9).

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use serde::Serialize;
use serde_json::json;

use crate::error::{DwError, Kind};
use crate::pending::{self, Item};
use crate::store::Store;
use crate::track::{self, Ctx, ItemResult, Outcome, Plan};

pub fn require_initialised(store: &Store) -> Result<()> {
    if !store.is_initialised() {
        return Err(
            DwError::usage("not initialised; run `dw init` or `dw clone <url>` first").into(),
        );
    }
    Ok(())
}

fn cwd() -> Result<PathBuf> {
    Ok(std::env::current_dir()?)
}

#[derive(Serialize)]
struct Listed<'a> {
    #[serde(flatten)]
    item: &'a Item,
    /// Why the item can no longer be applied as previewed, if so.
    stale: Option<String>,
}

fn print_json(v: &impl Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

fn print_results(results: &[ItemResult]) {
    for r in results {
        let id = r.id.map_or(String::new(), |i| format!("#{i} "));
        match r.status {
            Outcome::Applied => println!(
                "applied {id}{} (staged {} path(s); not committed)",
                r.summary,
                r.staged.len()
            ),
            Outcome::Stale => println!(
                "stale   {id}{}: {} — item kept; review it with `dw pending show`",
                r.summary,
                r.message.as_deref().unwrap_or("state changed")
            ),
            Outcome::Failed => println!(
                "failed  {id}{}: {} — nothing was changed; item kept",
                r.summary,
                r.message.as_deref().unwrap_or("error")
            ),
        }
    }
}

fn results_exit(results: &[ItemResult]) -> ExitCode {
    track::result_kind(results).map_or(ExitCode::SUCCESS, |k| ExitCode::from(k.code()))
}

/// Add planned items to the pending list, show them, or apply them at once.
fn submit(
    store: &Store,
    items: Vec<Item>,
    apply: bool,
    dry_run: bool,
    json: bool,
) -> Result<ExitCode> {
    if dry_run {
        let list = pending::load(store)?;
        let marked: Vec<_> = items
            .iter()
            .map(|i| json!({"item": i, "already_pending": list.find_same(&i.action).is_some()}))
            .collect();
        if json {
            print_json(&json!({"dry_run": true, "apply": apply, "items": marked}))?;
        } else {
            println!(
                "Dry run: nothing was changed. Would {}:",
                if apply {
                    "apply immediately"
                } else {
                    "add to the pending list"
                }
            );
            for i in &items {
                println!();
                print!("{}", i.render());
            }
        }
        return Ok(ExitCode::SUCCESS);
    }
    if apply {
        let results = track::apply_now(store, &items)?;
        if json {
            print_json(&json!({"dry_run": false, "applied": results}))?;
        } else {
            print_results(&results);
        }
        return Ok(results_exit(&results));
    }
    // Planning ran without the lock; under it the items must still be fresh.
    let lock = store.lock()?;
    track::recheck(store, &lock, &items)?;
    let ctx = Ctx::load(store)?;
    let stale = track::stale_checker(store, &ctx);
    let added = pending::add(store, &lock, items, &stale)?;
    if json {
        print_json(&json!({"dry_run": false, "added": added}))?;
    } else {
        for a in &added {
            println!(
                "{} {}: {}{}",
                if a.already_pending {
                    "Already pending"
                } else if a.replaced {
                    "Replaced the stale item"
                } else {
                    "Added"
                },
                a.item.label(),
                a.item.summary,
                a.item
                    .preview
                    .applies_to
                    .as_ref()
                    .map_or(String::new(), |t| format!(" (applies to: {t})"))
            );
        }
        println!("Review with `dw pending show <id>`, apply with `dw pending apply --all`.");
    }
    Ok(ExitCode::SUCCESS)
}

pub fn track_cmd(
    store: &Store,
    plan: Plan,
    paths: &[String],
    apply: bool,
    dry_run: bool,
    json: bool,
) -> Result<ExitCode> {
    require_initialised(store)?;
    let requester = pending::requester(store);
    let items = track::plan_paths(store, plan, paths, &cwd()?, &requester)?;
    submit(store, items, apply, dry_run, json)
}

pub fn hide_cmd(
    store: &Store,
    add: bool,
    patterns: &[String],
    apply: bool,
    dry_run: bool,
    json: bool,
) -> Result<ExitCode> {
    require_initialised(store)?;
    let requester = pending::requester(store);
    let ctx = Ctx::load(store)?;
    let mut items = Vec::new();
    for p in patterns {
        let item = track::plan_hide(store, &ctx, add, p, &requester)?;
        if !items
            .iter()
            .any(|i: &Item| i.action.key() == item.action.key())
        {
            items.push(item);
        }
    }
    submit(store, items, apply, dry_run, json)
}

pub fn hide_list(store: &Store, json: bool) -> Result<ExitCode> {
    require_initialised(store)?;
    let rules = crate::hide::rule_lines(&store.repo())?;
    if json {
        print_json(&json!({"rules": rules}))?;
    } else {
        for r in &rules {
            println!("{r}");
        }
    }
    Ok(ExitCode::SUCCESS)
}

// ---- pending ---------------------------------------------------------------

fn no_ids(verb: &str, store: &Store) -> anyhow::Error {
    let n = pending::load(store).map_or(0, |l| l.items.len());
    DwError::usage(format!(
        "no item numbers given. There {} {n} pending item(s) (see `dw pending list`): \
         name the ones to {verb}, or run `dw pending {verb} --all` to {verb} every one of them",
        if n == 1 { "is" } else { "are" },
    ))
    .into()
}

pub fn pending_list(store: &Store, json: bool) -> Result<ExitCode> {
    require_initialised(store)?;
    let list = pending::load(store)?;
    let ctx = Ctx::load(store)?;
    let mut listed = Vec::new();
    for item in &list.items {
        listed.push(Listed {
            stale: track::stale_reason(store, &ctx, item)?,
            item,
        });
    }
    if json {
        print_json(&json!({"items": listed}))?;
    } else if listed.is_empty() {
        println!("No pending items.");
    } else {
        for l in &listed {
            println!(
                "{:<5} {:<12} {}{}",
                l.item.label(),
                l.item.requester,
                l.item.summary,
                if l.stale.is_some() { "  [stale]" } else { "" }
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

pub fn pending_show(store: &Store, id: u64, json: bool) -> Result<ExitCode> {
    require_initialised(store)?;
    let list = pending::load(store)?;
    let item = list.get(id).ok_or_else(|| pending::not_found(id))?;
    let ctx = Ctx::load(store)?;
    let stale = track::stale_reason(store, &ctx, item)?;
    if json {
        print_json(&Listed { item, stale })?;
    } else {
        print!("{}", item.render());
        if let Some(s) = stale {
            println!("STALE: {s}");
        }
    }
    Ok(ExitCode::SUCCESS)
}

pub fn pending_drop(
    store: &Store,
    ids: &[u64],
    all: bool,
    dry_run: bool,
    json: bool,
) -> Result<ExitCode> {
    require_initialised(store)?;
    if ids.is_empty() && !all {
        return Err(no_ids("drop", store));
    }
    let lock = if dry_run { None } else { Some(store.lock()?) };
    let ids: Vec<u64> = if all {
        pending::load(store)?
            .items
            .iter()
            .filter_map(|i| i.id)
            .collect()
    } else {
        ids.to_vec()
    };
    let gone = if dry_run {
        let list = pending::load(store)?;
        ids.iter()
            .map(|i| list.get(*i).cloned().ok_or_else(|| pending::not_found(*i)))
            .collect::<Result<Vec<_>>>()?
    } else {
        pending::drop_items(store, lock.as_ref().expect("held unless dry-run"), &ids)?
    };
    if json {
        print_json(&json!({"dry_run": dry_run, "dropped": gone}))?;
    } else {
        println!(
            "{}",
            if dry_run {
                "Dry run: nothing was changed. Would drop:"
            } else {
                "Dropped:"
            }
        );
        for i in &gone {
            println!("  {} {}", i.label(), i.summary);
        }
    }
    Ok(ExitCode::SUCCESS)
}

pub fn pending_apply(
    store: &Store,
    ids: &[u64],
    all: bool,
    dry_run: bool,
    json: bool,
) -> Result<ExitCode> {
    require_initialised(store)?;
    if ids.is_empty() && !all {
        return Err(no_ids("apply", store));
    }
    if dry_run {
        let list = pending::load(store)?;
        let ctx = Ctx::load(store)?;
        let chosen: Vec<&Item> = if all {
            list.items.iter().collect()
        } else {
            ids.iter()
                .map(|i| list.get(*i).ok_or_else(|| pending::not_found(*i)))
                .collect::<Result<_>>()?
        };
        let mut plan = Vec::new();
        for i in chosen {
            plan.push(Listed {
                stale: track::stale_reason(store, &ctx, i)?,
                item: i,
            });
        }
        if json {
            print_json(&json!({"dry_run": true, "items": plan}))?;
        } else {
            println!("Dry run: nothing was changed. Would apply:");
            for p in &plan {
                println!();
                print!("{}", p.item.render());
                if let Some(s) = &p.stale {
                    println!("STALE (would be skipped, exit 7): {s}");
                }
            }
        }
        let any_stale = plan.iter().any(|p| p.stale.is_some());
        return Ok(if any_stale {
            ExitCode::from(Kind::Stale.code())
        } else {
            ExitCode::SUCCESS
        });
    }
    let results = track::apply_pending(store, if all { None } else { Some(ids) })?;
    if json {
        print_json(&json!({"dry_run": false, "applied": results}))?;
    } else {
        print_results(&results);
    }
    Ok(results_exit(&results))
}

// ---- ls ----------------------------------------------------------------------

pub fn ls_cmd(store: &Store, path: Option<&str>, all: bool, json: bool) -> Result<ExitCode> {
    require_initialised(store)?;
    let cwd = cwd()?;
    let hp = match path {
        Some(p) => track::resolve_home_or_root(store, p, &cwd)?,
        // Without a path: the current folder when it is inside HOME, else HOME.
        None => track::resolve_home_or_root(store, ".", &cwd).unwrap_or(None),
    };
    let entries = track::ls(store, hp.as_ref(), all)?;
    let shown = hp
        .as_ref()
        .map_or("~".to_string(), |h| format!("~/{}", h.rel));
    if json {
        print_json(&json!({"path": shown, "entries": entries}))?;
    } else {
        for e in &entries {
            println!(
                "{:<16} {:<8} {}{}{}",
                e.state,
                e.kind,
                e.name,
                if e.kind == "dir" { "/" } else { "" },
                e.pending
                    .map_or(String::new(), |p| format!("  [pending {p}]"))
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}
