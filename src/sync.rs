//! `dw sync` (spec §6a, ADR 0009): one backup round. Pull (fast-forward only),
//! apply the Incoming files that have no Local edit, stage every Local and New
//! file, commit and push, stopping at the first step that needs a decision.
//! The CLI and the TUI run the same plan and the same steps.

use anyhow::Result;
use serde::Serialize;

use crate::changes::{self, Change, State};
use crate::error::{self, DwError, Kind};
use crate::remote::{self, PullMode};
use crate::store::Store;
use crate::track;

/// How the push step runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushMode {
    /// A human approved the plan: push.
    Run,
    /// Agents, and runs without a terminal and without `--yes`: the push
    /// becomes a pending item (exit 3).
    Pending,
}

#[derive(Debug, Clone, Default)]
pub struct Opts {
    /// `-m` paragraphs; none means the generated message (or the editor).
    pub messages: Vec<String>,
    /// `-e`: write the message in the editor, starting from the generated one.
    pub edit: bool,
    /// Remote to pull from and push to (default: the branch's, else origin).
    pub remote: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Skip {
    pub path: String,
    pub reason: String,
}

/// Why sync stopped, with the exit code of that step (spec §9).
#[derive(Debug, Clone, Serialize)]
pub struct Stop {
    pub kind: &'static str,
    pub code: u8,
    pub message: String,
}

impl Stop {
    fn new(kind: Kind, message: String) -> Self {
        Self {
            kind: kind.name(),
            code: kind.code(),
            message,
        }
    }

    fn from_error(e: &anyhow::Error) -> Self {
        let (kind, message) = error::classify(e);
        Self::new(kind, message)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    pub dry_run: bool,
    pub machine: String,
    pub branch: String,
    /// The remote pulled from and pushed to.
    pub remote: String,
    /// What the pull did (in a dry run: would do, from the last fetch).
    pub pull: String,
    /// Incoming files written to HOME (a snapshot is taken first).
    pub apply: Vec<String>,
    /// Of those, files dotwarden never wrote on this machine.
    pub replaced_existing: Vec<String>,
    /// Local changes and New files that will be staged.
    pub stage: Vec<String>,
    /// Which of `stage` are New files.
    pub new_files: Vec<String>,
    pub skipped: Vec<Skip>,
    /// Changes staged before sync; they are committed too.
    pub staged_before: Vec<String>,
    pub notes: Vec<String>,
    /// The commit message; `None` when nothing will be committed.
    pub message: Option<String>,
    /// The message is written in the editor (starting from `message`).
    pub edit: bool,
    /// Commits the push sends; 0 means no push.
    pub push_commits: usize,
    /// The push becomes a pending item instead of running.
    pub push_pending: bool,
    /// The step that will stop sync, when it is already known.
    pub stop: Option<Stop>,
}

impl Plan {
    /// Nothing to apply, stage, commit or push, and nothing in the way.
    pub fn is_empty(&self) -> bool {
        self.apply.is_empty()
            && self.message.is_none()
            && self.push_commits == 0
            && self.stop.is_none()
    }

    /// The plan as shown before `Proceed?` and in the TUI dialog.
    pub fn lines(&self) -> Vec<String> {
        let mut l = vec![format!(
            "Sync {} ({} with {}):",
            self.machine, self.branch, self.remote
        )];
        l.push(format!("  pull    {}", self.pull));
        for f in &self.apply {
            let replaced = self.replaced_existing.contains(f);
            l.push(format!(
                "  apply   {f}{}",
                if replaced {
                    " (replaces a file dotwarden never wrote here)"
                } else {
                    ""
                }
            ));
        }
        for f in &self.staged_before {
            l.push(format!("  staged  {f} (staged earlier)"));
        }
        for f in &self.stage {
            let new = if self.new_files.contains(f) {
                " (new)"
            } else {
                ""
            };
            l.push(format!("  stage   {f}{new}"));
        }
        for s in &self.skipped {
            l.push(format!(
                "  skip    {}: {}",
                s.path,
                s.reason.replace('\n', " ")
            ));
        }
        for n in &self.notes {
            l.push(format!("  note    {n}"));
        }
        if let Some(m) = &self.message {
            let first = m.lines().next().unwrap_or_default();
            l.push(if self.edit {
                format!("  commit  (message written in the editor, starting from: {first})")
            } else {
                format!("  commit  {first}")
            });
        }
        if self.push_commits > 0 {
            l.push(format!(
                "  push    {} commit(s) of {} to {}{}",
                self.push_commits,
                self.branch,
                self.remote,
                if self.push_pending {
                    " (becomes a pending item: needs a human)"
                } else {
                    ""
                }
            ));
        }
        if let Some(s) = &self.stop {
            l.push(format!("  stop    {}", s.message.replace('\n', " ")));
        }
        if self.is_empty() {
            l.push("  nothing to sync: everything is backed up".into());
        }
        l
    }
}

/// What a run did, up to where it stopped.
#[derive(Debug, Default, Serialize)]
pub struct Done {
    pub applied: Vec<String>,
    pub snapshot: Option<String>,
    pub replaced_existing: Vec<String>,
    pub staged: Vec<String>,
    pub skipped: Vec<Skip>,
    pub commit: Option<String>,
    pub subject: Option<String>,
    pub agent: Option<String>,
    /// `main to origin` once pushed.
    pub pushed: Option<String>,
    pub warnings: Vec<String>,
    /// The pending push item added instead of pushing.
    pub pending_push: Option<String>,
    pub pending_removed: Vec<String>,
    pub stop: Option<Stop>,
}

impl Done {
    pub fn lines(&self) -> Vec<String> {
        let mut l = Vec::new();
        for f in &self.applied {
            l.push(format!("applied {f}"));
        }
        if let Some(id) = &self.snapshot {
            l.push(format!(
                "snapshot {id} holds what apply replaced; undo with `dw snapshot restore {id}`"
            ));
        }
        for f in &self.staged {
            l.push(format!("staged  {f}"));
        }
        for s in &self.skipped {
            l.push(format!(
                "skipped {}: {}",
                s.path,
                s.reason.replace('\n', " ")
            ));
        }
        if let Some(c) = &self.commit {
            l.push(format!(
                "committed {c} {}{}",
                self.subject.as_deref().unwrap_or_default(),
                self.agent
                    .as_ref()
                    .map_or(String::new(), |a| format!(" (Agent: {a})"))
            ));
        }
        if let Some(p) = &self.pushed {
            l.push(format!("pushed  {p}"));
        }
        for w in &self.warnings {
            l.push(format!("warning: {w}"));
        }
        if let Some(p) = &self.pending_push {
            l.push(format!("added pending {p}"));
        }
        l.extend(self.pending_removed.iter().cloned());
        l
    }
}

fn target(store: &Store, opts: &Opts) -> Result<String> {
    match &opts.remote {
        Some(r) => {
            if !remote::names(store)?.contains(r) {
                return Err(DwError::usage(format!("no remote named {r}")).into());
            }
            Ok(r.clone())
        }
        None => remote::default_remote(store),
    }
}

/// Step 1: fetch and fast-forward. Diverged history is an error naming `dw pull`.
pub fn pull(store: &Store, opts: &Opts) -> Result<String> {
    let remote = target(store, opts)?;
    let p = remote::pull(store, Some(&remote), PullMode::FastForward)?;
    Ok(if p.how == "up-to-date" {
        format!("{remote} has nothing new")
    } else {
        format!(
            "pulled {} commit(s) from {remote} ({} file(s) changed)",
            p.commits, p.files
        )
    })
}

/// Step 1 of a dry run, from the last fetch: no network, nothing changes.
fn plan_pull(store: &Store, remote: &str) -> Result<String> {
    if remote::merge_in_progress(store) {
        anyhow::bail!(
            "a merge is in progress: resolve the conflicts, then `dw pull --continue` (or `--abort`)"
        );
    }
    let Ok(p) = remote::plan_pull(store, Some(remote), PullMode::FastForward) else {
        return Ok(format!("nothing fetched from {remote} yet"));
    };
    let reference = format!("{remote}/{}", remote::current_branch(store)?);
    Ok(match (p.ahead, p.behind) {
        (_, 0) => format!("{remote} had nothing new at the last fetch"),
        (0, b) => format!(
            "would pull {b} commit(s) from {remote} ({} file(s) changed; those are not listed below until the pull)",
            p.files
        ),
        (a, b) => return Err(remote::diverged_error(&reference, a, b)),
    })
}

fn shown(c: &Change) -> String {
    if c.kind == "repo" || c.kind == "merge" {
        format!("repo:{}", c.path)
    } else {
        track::tilde(&c.path)
    }
}

/// `sync from <machine>: N changed, M new (<first few paths>)`.
pub fn generated_message(machine: &str, entries: &[(String, bool)]) -> String {
    let new = entries.iter().filter(|(_, n)| *n).count();
    let mut paths: Vec<&str> = entries
        .iter()
        .take(3)
        .map(|(p, _)| p.strip_suffix(" (deleted; untracked)").unwrap_or(p))
        .collect();
    if entries.len() > 3 {
        paths.push("…");
    }
    format!(
        "sync from {machine}: {} changed, {new} new ({})",
        entries.len() - new,
        paths.join(", ")
    )
}

fn machine_name(store: &Store) -> Result<String> {
    Ok(store
        .local_machine_name()?
        .unwrap_or_else(|| "this machine".into()))
}

fn is_nothing_to_stage(e: &anyhow::Error) -> bool {
    format!("{e:#}") == "nothing to stage"
}

/// Plan steps 2–5 as the repository and HOME are now (after step 1, or, in a
/// dry run, before it). Nothing is written.
pub fn plan(store: &Store, opts: &Opts, pulled: Option<String>, push: PushMode) -> Result<Plan> {
    let remote = target(store, opts)?;
    let dry_run = pulled.is_none();
    let mut plan = Plan {
        dry_run,
        machine: machine_name(store)?,
        branch: remote::current_branch(store)?,
        remote: remote.clone(),
        pull: String::new(),
        apply: Vec::new(),
        replaced_existing: Vec::new(),
        stage: Vec::new(),
        new_files: Vec::new(),
        skipped: Vec::new(),
        staged_before: Vec::new(),
        notes: Vec::new(),
        message: None,
        edit: opts.edit,
        push_commits: 0,
        push_pending: push == PushMode::Pending,
        stop: None,
    };
    plan.pull = match pulled {
        Some(p) => p,
        None => match plan_pull(store, &remote) {
            Ok(p) => p,
            Err(e) => {
                plan.pull = "stops here".into();
                plan.stop = Some(Stop::from_error(&e));
                return Ok(plan);
            }
        },
    };
    let found = changes::classify(store)?;
    let a = changes::apply_incoming(store, None, true)?;
    plan.apply = a.applied.iter().map(|f| track::tilde(f)).collect();
    plan.replaced_existing = a
        .replaced_existing
        .iter()
        .map(|f| track::tilde(f))
        .collect();
    if let Some(stop) = apply_stop(&a.conflicts, &a.vault_locked) {
        plan.stop = Some(Stop::from_error(&stop));
        return Ok(plan);
    }
    let new: Vec<String> = found
        .iter()
        .filter(|c| c.state == State::New)
        .map(shown)
        .collect();
    plan.staged_before = found
        .iter()
        .filter(|c| c.state == State::Staged)
        .map(shown)
        .collect();
    match changes::stage(store, None, true) {
        Ok(d) => {
            plan.stage = d.paths;
            plan.skipped = d
                .skipped
                .into_iter()
                .filter(|s| !plan.apply.contains(&s.path))
                .map(|s| Skip {
                    path: s.path,
                    reason: s.reason,
                })
                .collect();
            plan.notes = d.notes;
        }
        Err(e) if is_nothing_to_stage(&e) => {}
        Err(e) => {
            plan.stop = Some(Stop::from_error(&e));
            return Ok(plan);
        }
    }
    plan.new_files = plan
        .stage
        .iter()
        .filter(|p| new.contains(p))
        .cloned()
        .collect();
    let entries: Vec<(String, bool)> = plan
        .staged_before
        .iter()
        .map(|p| (p.clone(), false))
        .chain(
            plan.stage
                .iter()
                .map(|p| (p.clone(), plan.new_files.contains(p))),
        )
        .collect();
    if !entries.is_empty() {
        plan.message = Some(if opts.messages.is_empty() {
            generated_message(&plan.machine, &entries)
        } else {
            opts.messages.join("\n\n")
        });
    }
    plan.push_commits =
        remote::unpushed(store, &remote).unwrap_or(0) + usize::from(plan.message.is_some());
    Ok(plan)
}

/// Conflicts and files that need the locked vault stop sync before staging.
fn apply_stop(conflicts: &[String], locked: &[String]) -> Option<anyhow::Error> {
    let list = |v: &[String]| {
        v.iter()
            .map(|f| track::tilde(f))
            .collect::<Vec<_>>()
            .join(", ")
    };
    if !conflicts.is_empty() {
        return Some(
            DwError::new(
                Kind::Conflict,
                format!(
                    "{} changed here and in the repository; resolve with `dw resolve PATH`, then run `dw sync` again. Nothing was staged",
                    list(conflicts)
                ),
            )
            .into(),
        );
    }
    if !locked.is_empty() {
        return Some(
            DwError::new(
                Kind::VaultLocked,
                format!(
                    "{} need the vault, which is locked; run `dw vault unlock`, then `dw sync` again. Nothing was staged",
                    list(locked)
                ),
            )
            .into(),
        );
    }
    None
}

/// Run steps 2–5 (step 1 ran before the plan). `interactive` allows the
/// editor for `-e`.
pub fn run(store: &Store, opts: &Opts, plan: &Plan, push: PushMode, interactive: bool) -> Done {
    let mut done = Done::default();
    if let Some(stop) = &plan.stop
        && plan.apply.is_empty()
    {
        done.stop = Some(stop.clone());
        return done;
    }
    if let Err(e) = steps(store, opts, plan, push, interactive, &mut done) {
        done.stop = Some(Stop::from_error(&e));
    }
    done
}

fn steps(
    store: &Store,
    opts: &Opts,
    plan: &Plan,
    push: PushMode,
    interactive: bool,
    done: &mut Done,
) -> Result<()> {
    // 2. Apply Incoming files without a Local edit.
    let a = changes::apply_incoming(store, None, false)?;
    done.applied = a.applied.iter().map(|f| track::tilde(f)).collect();
    done.snapshot = a.snapshot.clone();
    done.replaced_existing = a
        .replaced_existing
        .iter()
        .map(|f| track::tilde(f))
        .collect();
    if !a.failed.is_empty() {
        anyhow::bail!(
            "could not apply {}: {}",
            a.failed.join(", "),
            a.error.as_deref().unwrap_or("chezmoi failed")
        );
    }
    if let Some(stop) = apply_stop(&a.conflicts, &a.vault_locked) {
        return Err(stop);
    }
    // 3. Stage every Local and New file that can be.
    match changes::stage(store, None, false) {
        Ok(d) => {
            done.staged = d.paths;
            done.skipped = d
                .skipped
                .into_iter()
                .map(|s| Skip {
                    path: s.path,
                    reason: s.reason,
                })
                .collect();
        }
        Err(e) if is_nothing_to_stage(&e) => {}
        Err(e) => return Err(e),
    }
    // 4. Commit everything staged, including earlier staging.
    let staged: Vec<(String, bool)> = changes::classify(store)?
        .iter()
        .filter(|c| c.state == State::Staged)
        .map(|c| (shown(c), c.kind == "new"))
        .collect();
    if !staged.is_empty() {
        let generated = generated_message(&plan.machine, &staged);
        let (messages, template) = if opts.edit {
            (Vec::new(), Some(generated))
        } else if opts.messages.is_empty() {
            (vec![generated], None)
        } else {
            (opts.messages.clone(), None)
        };
        let c = changes::commit(
            store,
            &changes::CommitOpts {
                messages,
                amend: false,
                dry_run: false,
                interactive: interactive && opts.edit,
                template,
            },
        )?;
        done.commit = c.commit;
        done.subject = Some(c.subject);
        done.agent = c.agent;
    }
    // 5. Push when HEAD is ahead of the remote.
    if remote::unpushed(store, &plan.remote).unwrap_or(0) == 0 {
        return Ok(());
    }
    match push {
        PushMode::Run => {
            let pushed = remote::push(store, std::slice::from_ref(&plan.remote))?;
            done.pending_removed = remote::forget_satisfied(store, &pushed);
            for p in &pushed {
                done.pushed = Some(format!("{} to {}", p.branch, p.remote));
                done.warnings.extend(p.warning.clone());
            }
            Ok(())
        }
        PushMode::Pending => {
            let item = remote::plan_push_item(
                store,
                std::slice::from_ref(&plan.remote),
                &crate::pending::requester(store),
            )?;
            let lock = store.lock()?;
            let ctx = track::Ctx::load(store)?;
            let added =
                crate::pending::add(store, &lock, vec![item], &track::stale_checker(store, &ctx))?;
            if let Some(a) = added.first() {
                done.pending_push = Some(format!("{}: {}", a.item.label(), a.item.summary));
            }
            Err(DwError::new(
                Kind::NeedsApproval,
                "the push needs a human: approve it with `dw pending apply` in a terminal, or in the TUI",
            )
            .into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::generated_message;

    #[test]
    fn generated_message_counts_and_names_the_first_paths() {
        let e = |p: &str, n| (p.to_string(), n);
        assert_eq!(
            generated_message(
                "alpha",
                &[
                    e("~/.a", false),
                    e("~/.b (deleted; untracked)", false),
                    e("~/.c", true),
                    e("~/.d", true)
                ]
            ),
            "sync from alpha: 2 changed, 2 new (~/.a, ~/.b, ~/.c, …)"
        );
    }
}
