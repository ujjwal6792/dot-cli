//! Handlers of the track, untrack, pending, hide and ls commands (spec §9).

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use serde::Serialize;
use serde_json::json;

use crate::changes::{self, CommitOpts};
use crate::error::{DwError, Kind};
use crate::pending::{self, Item};
use crate::snapshot;
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
            Outcome::Applied if r.kind == "discard" => {
                println!("applied {id}{} (snapshot taken first)", r.summary)
            }
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

fn home_selectors(
    store: &Store,
    paths: &[String],
    all: bool,
    verb: &str,
) -> Result<Option<Vec<String>>> {
    require_initialised(store)?;
    if all {
        return Ok(None);
    }
    if paths.is_empty() {
        return Err(DwError::usage(format!("name the paths to {verb}, or pass --all")).into());
    }
    let cwd = cwd()?;
    paths
        .iter()
        .map(|p| Ok(track::resolve_home_path(store, p, &cwd)?.rel))
        .collect::<Result<_>>()
        .map(Some)
}

pub fn stage_cmd(
    store: &Store,
    stage: bool,
    paths: &[String],
    all: bool,
    dry_run: bool,
    json: bool,
) -> Result<ExitCode> {
    let verb = if stage { "stage" } else { "unstage" };
    let sel = home_selectors(store, paths, all, verb)?;
    let done = if stage {
        changes::stage(store, sel.as_deref(), dry_run)?
    } else {
        changes::unstage(store, sel.as_deref(), dry_run)?
    };
    if json {
        print_json(&done)?;
    } else {
        let tense = match (dry_run, stage) {
            (true, true) => "would stage",
            (true, false) => "would unstage",
            (false, true) => "staged",
            (false, false) => "unstaged",
        };
        for p in &done.paths {
            println!("{tense} {p}");
        }
        for s in &done.skipped {
            println!("skipped {}: {}", s.path, s.reason);
        }
    }
    Ok(ExitCode::SUCCESS)
}

pub fn commit_cmd(
    store: &Store,
    messages: Vec<String>,
    amend: bool,
    dry_run: bool,
    json: bool,
) -> Result<ExitCode> {
    require_initialised(store)?;
    use std::io::IsTerminal;
    let interactive = !json && std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let done = changes::commit(
        store,
        &CommitOpts {
            messages,
            amend,
            dry_run,
            interactive,
        },
    )?;
    if json {
        print_json(&done)?;
    } else {
        println!(
            "{} {} ({} file(s)){}",
            if dry_run {
                "would commit"
            } else if amend {
                "amended"
            } else {
                "committed"
            },
            done.commit.as_deref().unwrap_or(&done.subject),
            done.files,
            done.agent
                .as_ref()
                .map_or(String::new(), |a| format!(", Agent: {a}"))
        );
    }
    Ok(ExitCode::SUCCESS)
}

pub fn diff_cmd(store: &Store, path: Option<&str>, no_pager: bool, json: bool) -> Result<ExitCode> {
    require_initialised(store)?;
    let sel = match path {
        Some(p) => Some(vec![track::resolve_home_path(store, p, &cwd()?)?.rel]),
        None => None,
    };
    let diffs = changes::diff(store, sel.as_deref())?;
    if json {
        return print_json(&diffs).map(|_| ExitCode::SUCCESS);
    }
    let text: String = diffs.iter().map(|d| d.diff.as_str()).collect();
    use std::io::{IsTerminal, Write};
    if !no_pager && !text.is_empty() && std::io::stdout().is_terminal() {
        let mut child = std::process::Command::new(store.tools.delta())
            .stdin(std::process::Stdio::piped())
            .spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        child.wait()?;
    } else {
        print!("{text}");
    }
    Ok(ExitCode::SUCCESS)
}

pub fn discard_cmd(
    store: &Store,
    paths: &[String],
    yes: bool,
    dry_run: bool,
    json: bool,
) -> Result<ExitCode> {
    require_initialised(store)?;
    use std::io::IsTerminal;
    let agent = changes::agent(store);
    let requester = agent.clone().unwrap_or_else(|| "human".into());
    let cwd = cwd()?;
    let mut items = Vec::new();
    for p in paths {
        let hp = track::resolve_home_path(store, p, &cwd)?;
        items.push(changes::plan_discard(store, &hp.rel, &requester)?);
    }
    if dry_run {
        return submit(store, items, false, true, json);
    }
    let interactive = !json && std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let approved = agent.is_none() && (yes || (interactive && confirm(&items)?));
    if !approved {
        if agent.is_none() && interactive {
            return Err(anyhow::anyhow!("cancelled; nothing was discarded"));
        }
        submit(store, items, false, false, json)?;
        if !json {
            println!("Discarding needs a human: approve it with `dw pending apply`.");
        }
        return Ok(ExitCode::from(Kind::NeedsApproval.code()));
    }
    let results = track::apply_now(store, &items)?;
    if json {
        print_json(&json!({"dry_run": false, "applied": results}))?;
    } else {
        print_results(&results);
    }
    Ok(results_exit(&results))
}

fn confirm(items: &[Item]) -> Result<bool> {
    use std::io::{BufRead, Write};
    for i in items {
        println!("{}", i.summary);
    }
    print!("Overwrite them with the committed version? A snapshot is taken first. [y/N] ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes"))
}

pub fn snapshot_list(store: &Store, path: Option<&str>, json: bool) -> Result<ExitCode> {
    require_initialised(store)?;
    let snaps = match path {
        Some(p) => {
            let hp = track::resolve_home_path(store, p, &cwd()?)?;
            snapshot::list_for(store, &hp.rel)?
        }
        None => snapshot::list(store)?,
    };
    if json {
        print_json(&snaps)?;
    } else {
        for s in &snaps {
            let names: Vec<_> = s.files.iter().map(|f| format!("~/{}", f.path)).collect();
            println!(
                "{}  {}  {:<8} {:<8} {}",
                s.id,
                s.created,
                s.reason,
                s.actor,
                names.join(" ")
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

pub fn snapshot_restore(
    store: &Store,
    id: &str,
    paths: &[String],
    dry_run: bool,
    json: bool,
) -> Result<ExitCode> {
    require_initialised(store)?;
    let cwd = cwd()?;
    let rels: Vec<String> = paths
        .iter()
        .map(|p| Ok(track::resolve_home_path(store, p, &cwd)?.rel))
        .collect::<Result<_>>()?;
    if dry_run {
        let plan = snapshot::plan_restore(store, &snapshot::find(store, id)?, &rels)?;
        if json {
            print_json(&json!({"dry_run": true, "files": plan}))?;
        } else {
            for p in &plan {
                println!("would restore ~/{}", p.path);
            }
        }
        return Ok(ExitCode::SUCCESS);
    }
    let _lock = store.lock()?;
    let actor = changes::agent(store).unwrap_or_else(|| "human".into());
    let (restored, safety) = snapshot::restore(store, id, &rels, &actor)?;
    if json {
        print_json(&json!({"restored": restored, "safety_snapshot": safety}))?;
    } else {
        for r in &restored {
            println!("restored ~/{r}");
        }
        if let Some(s) = safety {
            println!("what was there before is in snapshot {s}");
        }
    }
    Ok(ExitCode::SUCCESS)
}
