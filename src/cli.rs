//! Command-line interface (spec §9).

use std::io::{BufRead, IsTerminal, Write};
use std::process::ExitCode;

use anyhow::Result;
use clap::error::ErrorKind;
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use serde_json::json;

use crate::error::{self, DwError};
use crate::machine;
use crate::repo::{self, DEFAULT_BRANCH};
use crate::store::Store;
use crate::track::Plan;
use crate::{cmds, doctor, status, tui};

/// Dotfiles, secrets and variables in sync across your machines.
///
/// Exit codes: 0 ok, 1 error, 2 usage, 3 needs approval, 4 vault locked,
/// 5 conflict, 6 refused public remote, 7 stale. With --json, errors are printed
/// on stdout as {"error": {"kind", "code", "message"}}.
#[derive(Debug, Parser)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// Print machine-readable JSON instead of human output.
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create a new Dotwarden repository and register this computer as a machine.
    Init {
        /// Registry name for this machine (a-z, 0-9, '-'); prompted for when omitted.
        #[arg(long)]
        name: Option<String>,
        /// Name of the default branch of the new repository.
        #[arg(long, default_value = DEFAULT_BRANCH)]
        branch: String,
        /// Show exactly what would be created without changing anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Clone an existing Dotwarden repository and register this computer as a machine.
    Clone {
        /// Location of the repository: an https:// or ssh URL, a file:// URL or a path.
        url: String,
        /// Registry name for this machine (a-z, 0-9, '-'); prompted for when omitted.
        #[arg(long)]
        name: Option<String>,
        /// Reuse an existing registry entry with this name (re-installing the same machine).
        #[arg(long)]
        adopt: bool,
        /// Show exactly what would be done without cloning or changing anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Show this machine, the repository and the pending list.
    Status,
    /// Check tool versions, paths, the registered machine and repository health.
    Doctor,
    /// Add tracking of files, symlinks or folders to the pending list.
    ///
    /// Paths are HOME paths: `~/x`, absolute, or relative to the current folder.
    /// A folder tracks its current files and is recorded as a tracked folder: new
    /// files that appear in it later show up as New, never added automatically.
    /// New tracked files apply to all machines. Hidden paths are refused.
    Track {
        /// Files, symlinks or folders to track.
        #[arg(required = true, num_args = 1..)]
        paths: Vec<String>,
        /// Apply immediately instead of leaving the items in the pending list.
        #[arg(long)]
        apply: bool,
        /// Show exactly what would change without adding or applying anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Add untracking of files or folders to the pending list.
    ///
    /// The files leave the repository (staged in git, not committed); every
    /// machine keeps its HOME copy.
    Untrack {
        /// Tracked files or folders to stop tracking.
        #[arg(required = true, num_args = 1..)]
        paths: Vec<String>,
        /// Apply immediately instead of leaving the items in the pending list.
        #[arg(long)]
        apply: bool,
        /// Show exactly what would change without adding or applying anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Assign tracked files or folders to targets (adds one item to the pending list).
    ///
    /// A target is `platform:darwin|linux`, `distribution:<id>`, `family:<id>`
    /// or `machine:<name>`. A file applies when it has no includes or any include
    /// matches, and no exclude matches (excludes win). A folder's assignment
    /// covers everything below it. The preview lists which tracked files stop or
    /// start applying on which registry machines. Without --include, --exclude
    /// or --unset, only --clear changes the assignment (back to all machines).
    Assign {
        /// Tracked files or folders.
        #[arg(required = true, num_args = 1..)]
        paths: Vec<String>,
        /// Add an include (repeatable).
        #[arg(long, value_name = "TARGET")]
        include: Vec<String>,
        /// Add an exclude (repeatable).
        #[arg(long, value_name = "TARGET")]
        exclude: Vec<String>,
        /// Take a target out of both lists (repeatable).
        #[arg(long, value_name = "TARGET")]
        unset: Vec<String>,
        /// Remove the existing assignment first (before --include and --exclude).
        #[arg(long)]
        clear: bool,
        /// Apply immediately instead of leaving the item in the pending list.
        #[arg(long)]
        apply: bool,
        /// Show exactly what would change without adding or applying anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Keep different content of one tracked file per target (platform, distribution, family, machine).
    ///
    /// `add` stores the file's current HOME content as the variant for TARGET;
    /// the content stored so far becomes the default variant. A machine receives
    /// the most specific variant that matches it (machine > distribution >
    /// family > platform > default). Two matches at the same level block that
    /// file only. Each change is one pending item with an exact preview.
    Variant {
        #[command(subcommand)]
        action: VariantCmd,
    },
    /// List, register, rename, retire or preview machines of the registry.
    Machine {
        #[command(subcommand)]
        action: MachineCmd,
    },
    /// List, show, drop or apply pending items (lists when no subcommand is given).
    Pending {
        #[command(subcommand)]
        action: Option<PendingCmd>,
    },
    /// List, add or remove hide rules (lists when no subcommand is given).
    ///
    /// Hide rules use gitignore syntax relative to HOME, are shared by all
    /// machines, keep noise out of the file browser and refuse tracking. Changes
    /// go through the pending list.
    Hide {
        #[command(subcommand)]
        action: Option<HideCmd>,
    },
    /// Show the tracked state of the entries of a HOME folder.
    ///
    /// States: tracked, tracked-folder, contains-tracked, new (untracked file in a
    /// tracked folder), untracked, hidden.
    Ls {
        /// A HOME path (`~/x`, absolute or relative); the current folder inside HOME, else HOME.
        path: Option<String>,
        /// Include entries matched by hide rules.
        #[arg(long, short = 'a')]
        all: bool,
    },
    /// Stage local changes and new files: copy their HOME content into the repository.
    ///
    /// Paths are HOME paths; a folder stages every change below it. Staging a
    /// new file inside a tracked folder starts tracking it. Incoming changes,
    /// conflicts, deleted files and templates are refused.
    Stage {
        /// Files or folders to stage.
        #[arg(num_args = 0..)]
        paths: Vec<String>,
        /// Stage every change that can be staged.
        #[arg(long, conflicts_with = "paths")]
        all: bool,
        /// Show what would be staged without changing anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Unstage changes: put the committed version back in the repository.
    ///
    /// HOME is not touched; the file shows as a local change again.
    Unstage {
        /// Files or folders to unstage.
        #[arg(num_args = 0..)]
        paths: Vec<String>,
        /// Unstage everything.
        #[arg(long, conflicts_with = "paths")]
        all: bool,
        /// Show what would be unstaged without changing anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Commit the staged changes.
    ///
    /// Without -m an editor ($VISUAL or $EDITOR) opens when run in a terminal.
    /// Commits made with DOTWARDEN_AGENT set carry an `Agent: <name>` trailer.
    Commit {
        /// Commit message; repeat for further paragraphs.
        #[arg(short, long = "message")]
        messages: Vec<String>,
        /// Replace the last commit (refused once it is on a remote branch).
        #[arg(long)]
        amend: bool,
        /// Show what would be committed without committing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Show local, new and staged changes as unified diffs.
    ///
    /// In a terminal the diff goes through the bundled delta; --no-pager and
    /// --json (or a pipe) give plain unified diff.
    Diff {
        /// A HOME file or folder; everything when omitted.
        path: Option<String>,
        /// Print plain unified diff even in a terminal.
        #[arg(long)]
        no_pager: bool,
    },
    /// Discard local changes: write the committed version back to HOME.
    ///
    /// A snapshot of the current files is taken first (`dw snapshot list`).
    /// Needs a human: from an agent (DOTWARDEN_AGENT) or without a terminal and
    /// without --yes the request goes to the pending list and the exit code is 3;
    /// approve it with `dw pending apply`.
    Discard {
        /// Files or folders with local changes.
        #[arg(required = true, num_args = 1..)]
        paths: Vec<String>,
        /// Do not ask for confirmation (ignored for agents).
        #[arg(long, short = 'y')]
        yes: bool,
        /// Show what would be overwritten without changing anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// List or restore snapshots of HOME files taken before they were overwritten.
    Snapshot {
        #[command(subcommand)]
        action: SnapshotCmd,
    },
    /// Manage the remotes this repository syncs with.
    ///
    /// Public remotes are refused (exit 6): secret names, variables and the machine
    /// registry would be readable by anyone. Adding or removing a remote needs a
    /// human: from an agent, or without a terminal and without --yes, the request
    /// goes to the pending list (exit 3).
    Remote {
        #[command(subcommand)]
        action: RemoteCmd,
    },
    /// Download new commits from a remote. Never changes HOME or the working tree.
    Fetch {
        /// Fetch every remote instead of only the default one.
        #[arg(long)]
        all: bool,
    },
    /// Fast-forward the branch to the remote's.
    ///
    /// Refuses when this machine has commits the remote lacks. Files whose
    /// committed version changed show as Incoming; `dw apply` writes them to HOME.
    Pull {
        /// Remote to pull from (default: the branch's remote, else origin).
        remote: Option<String>,
    },
    /// Write Incoming files to HOME (a snapshot of them is taken first).
    ///
    /// Only Incoming files are written. Files changed both here and in the
    /// repository (conflicts) are never overwritten: a conflict named explicitly
    /// exits 5, with --all it is skipped and listed.
    Apply {
        /// Files or folders to apply.
        #[arg(num_args = 0..)]
        paths: Vec<String>,
        /// Apply every incoming file.
        #[arg(long, conflicts_with = "paths")]
        all: bool,
        /// Show what would be written without changing anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Push the branch to a remote (the default one unless named).
    ///
    /// Visibility is checked again before every push; a public remote exits 6.
    /// Needs a human: from an agent, or without a terminal and without --yes, the
    /// request goes to the pending list (exit 3) and `dw pending apply` runs it.
    Push {
        /// Remote to push to (default: the branch's remote, else origin).
        #[arg(conflicts_with = "all")]
        remote: Option<String>,
        /// Push to every remote.
        #[arg(long)]
        all: bool,
        /// Do not ask for confirmation (ignored for agents).
        #[arg(long, short = 'y')]
        yes: bool,
        /// Show what would be pushed without changing anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Open the terminal UI (the default when no command is given).
    Tui,
}

#[derive(Debug, Subcommand)]
pub enum VariantCmd {
    /// Store the current HOME content of a tracked file as the variant for a target.
    Add {
        /// A tracked file.
        path: String,
        /// platform:darwin|linux, distribution:<id>, family:<id> or machine:<name>.
        #[arg(long, value_name = "TARGET")]
        target: String,
        /// Apply immediately instead of leaving the item in the pending list.
        #[arg(long)]
        apply: bool,
        /// Show exactly what would change without adding or applying anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// List variant files, their targets and the variant this machine receives.
    List {
        /// Only this tracked file.
        path: Option<String>,
    },
    /// Remove the variant for a target; the last one turns the file back into a plain file.
    Remove {
        /// A tracked file with variants.
        path: String,
        /// The target whose variant is removed.
        #[arg(long, value_name = "TARGET")]
        target: String,
        /// Apply immediately instead of leaving the item in the pending list.
        #[arg(long)]
        apply: bool,
        /// Show exactly what would change without adding or applying anything.
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum MachineCmd {
    /// List the registry: name, platform, distribution, families, retired.
    List,
    /// Register another machine in advance (through the pending list).
    Add {
        /// Registry name ([a-z0-9-]).
        name: String,
        /// darwin or linux.
        #[arg(long)]
        platform: String,
        /// The os-release ID, or macos.
        #[arg(long)]
        distribution: String,
        /// A distribution family such as arch-like or debian (repeatable).
        #[arg(long)]
        family: Vec<String>,
        /// Apply immediately instead of leaving the item in the pending list.
        #[arg(long)]
        apply: bool,
        /// Show exactly what would change without adding or applying anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Rename a machine in the registry, every assignment and every override.
    Rename {
        /// Current name.
        old: String,
        /// New name.
        new: String,
        /// Apply immediately instead of leaving the item in the pending list.
        #[arg(long)]
        apply: bool,
        /// Show exactly what would change without adding or applying anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Retire a machine: remove it from every assignment and mark it retired.
    ///
    /// Needs a human: from an agent, or without a terminal and without --yes,
    /// the request goes to the pending list (exit 3). This machine cannot be retired.
    Retire {
        /// Machine to retire.
        name: String,
        /// Do not ask for confirmation (ignored for agents).
        #[arg(long, short = 'y')]
        yes: bool,
        /// Show what would change without changing anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Point this computer at an existing registry machine (human only).
    ///
    /// For when the machine was renamed on another computer: its entry must
    /// match this computer's platform and distribution.
    Use {
        /// Registry machine this computer is.
        name: String,
    },
    /// List the tracked files a machine would receive.
    ///
    /// Uses a throwaway chezmoi state; nothing in HOME or the real state is written.
    Preview {
        /// Registry machine to preview.
        name: String,
        /// Also show the diff against this machine's HOME.
        #[arg(long)]
        diff: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum RemoteCmd {
    /// Add a remote after checking that it is not public.
    ///
    /// github.com, gitlab.com, codeberg.org and bitbucket.org are probed
    /// anonymously over HTTPS. Other hosts need you to confirm in a terminal that
    /// the repository is private; the answer is remembered for that URL.
    Add {
        /// Name of the remote, for example origin.
        name: String,
        /// Repository URL (https, ssh, scp-style, file or a path).
        url: String,
        /// Do not ask for confirmation (ignored for agents).
        #[arg(long, short = 'y')]
        yes: bool,
        /// Show what would change without changing anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Remove a remote.
    Remove {
        /// Name of the remote.
        name: String,
        /// Do not ask for confirmation (ignored for agents).
        #[arg(long, short = 'y')]
        yes: bool,
        /// Show what would change without changing anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// List the remotes and how their visibility is verified.
    List,
}

#[derive(Debug, Subcommand)]
pub enum SnapshotCmd {
    /// List snapshots, newest first.
    List {
        /// Only snapshots holding this HOME path.
        path: Option<String>,
    },
    /// Put snapshotted files back in HOME; what is there now is snapshotted first.
    Restore {
        /// Snapshot id from `dw snapshot list`.
        id: String,
        /// Only these files or folders (default: everything in the snapshot).
        paths: Vec<String>,
        /// Show what would be restored without changing anything.
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum PendingCmd {
    /// List pending items; stale ones are marked.
    List,
    /// Show everything one item will change.
    Show {
        /// Item number.
        id: u64,
    },
    /// Remove items from the list without applying them.
    Drop {
        /// Item numbers.
        ids: Vec<u64>,
        /// Drop every item.
        #[arg(long)]
        all: bool,
        /// Show what would be dropped without dropping it.
        #[arg(long)]
        dry_run: bool,
    },
    /// Apply items: each is all-or-nothing and staged in git (not committed).
    ///
    /// An item whose files or repository changed since it was added is stale: it
    /// is skipped and kept, and the exit code is 7.
    Apply {
        /// Item numbers.
        ids: Vec<u64>,
        /// Apply every item.
        #[arg(long)]
        all: bool,
        /// Show what would be applied (and which items are stale) without applying.
        #[arg(long)]
        dry_run: bool,
        /// Approve items that need a human without asking (never for agents).
        #[arg(long, short = 'y')]
        yes: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum HideCmd {
    /// List the hide rules.
    List,
    /// Add hide rules (gitignore syntax, e.g. `/.cache/` or `*.log`).
    Add {
        /// Rules to add.
        #[arg(required = true, num_args = 1..)]
        patterns: Vec<String>,
        /// Apply immediately instead of leaving the items in the pending list.
        #[arg(long)]
        apply: bool,
        /// Show exactly what would change without adding or applying anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Remove hide rules (exact text as listed).
    Remove {
        /// Rules to remove.
        #[arg(required = true, num_args = 1..)]
        patterns: Vec<String>,
        /// Apply immediately instead of leaving the items in the pending list.
        #[arg(long)]
        apply: bool,
        /// Show exactly what would change without adding or applying anything.
        #[arg(long)]
        dry_run: bool,
    },
}

pub fn main(bin_name: &'static str) -> ExitCode {
    quiet_broken_pipe();
    let json = std::env::args_os().any(|a| a == "--json");
    let matches = match Cli::command().name(bin_name).try_get_matches() {
        Ok(m) => m,
        Err(e) => {
            let informational =
                matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion);
            if informational || !json {
                e.exit();
            }
            return report(error::Kind::Usage, &e.render().to_string(), true);
        }
    };
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(c) => c,
        Err(e) => e.exit(),
    };
    let json = cli.json;
    match run(cli) {
        Ok(code) => code,
        Err(e) => {
            let (kind, message) = error::classify(&e);
            report(kind, &message, json)
        }
    }
}

/// Rust ignores SIGPIPE, so printing into a closed pipe (`dw status | head`)
/// panics; exit quietly with the conventional 128+SIGPIPE status instead.
fn quiet_broken_pipe() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let text = info
            .payload()
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| info.payload().downcast_ref::<&str>().copied())
            .unwrap_or_default();
        if text.contains("Broken pipe") {
            std::process::exit(141);
        }
        default(info);
    }));
}

fn report(kind: error::Kind, message: &str, json: bool) -> ExitCode {
    if json {
        println!("{}", error::to_json(kind, message));
    } else {
        eprintln!("error: {}", message.trim_end());
    }
    ExitCode::from(kind.code())
}

fn run(cli: Cli) -> Result<ExitCode> {
    let store = Store::from_process()?;
    let json = cli.json;
    match cli.command.unwrap_or(Command::Tui) {
        Command::Init {
            name,
            branch,
            dry_run,
        } => {
            let name = resolve_name(name, &[], dry_run)?;
            cmd_init(&store, name, &branch, dry_run, json)?;
        }
        Command::Clone {
            url,
            name,
            adopt,
            dry_run,
        } => cmd_clone(&store, &url, name, adopt, dry_run, json)?,
        Command::Status => {
            let s = status::collect(&store, None)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&s)?);
            } else {
                print!("{}", s.render_human());
            }
        }
        Command::Doctor => {
            let report = doctor::run(&store);
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print!("{}", report.render_human());
            }
            if !report.ok {
                return Ok(ExitCode::from(error::Kind::Error.code()));
            }
        }
        Command::Track {
            paths,
            apply,
            dry_run,
        } => return cmds::track_cmd(&store, Plan::Track, &paths, apply, dry_run, json),
        Command::Untrack {
            paths,
            apply,
            dry_run,
        } => return cmds::track_cmd(&store, Plan::Untrack, &paths, apply, dry_run, json),
        Command::Assign {
            paths,
            include,
            exclude,
            unset,
            clear,
            apply,
            dry_run,
        } => {
            let edits = cmds::AssignArgs {
                include,
                exclude,
                unset,
                clear,
            };
            return cmds::assign_cmd(&store, &paths, &edits, apply, dry_run, json);
        }
        Command::Variant { action } => {
            return match action {
                VariantCmd::Add {
                    path,
                    target,
                    apply,
                    dry_run,
                } => cmds::variant_edit(&store, true, &path, &target, apply, dry_run, json),
                VariantCmd::Remove {
                    path,
                    target,
                    apply,
                    dry_run,
                } => cmds::variant_edit(&store, false, &path, &target, apply, dry_run, json),
                VariantCmd::List { path } => cmds::variant_list(&store, path.as_deref(), json),
            };
        }
        Command::Machine { action } => {
            return match action {
                MachineCmd::List => cmds::machine_list(&store, json),
                MachineCmd::Add {
                    name,
                    platform,
                    distribution,
                    family,
                    apply,
                    dry_run,
                } => cmds::machine_add(
                    &store,
                    crate::machine::Machine {
                        name,
                        platform,
                        distribution,
                        families: family,
                        retired: false,
                    },
                    apply,
                    dry_run,
                    json,
                ),
                MachineCmd::Rename {
                    old,
                    new,
                    apply,
                    dry_run,
                } => cmds::machine_rename(&store, &old, &new, apply, dry_run, json),
                MachineCmd::Retire { name, yes, dry_run } => {
                    cmds::machine_retire(&store, &name, yes, dry_run, json)
                }
                MachineCmd::Use { name } => cmds::machine_use(&store, &name, json),
                MachineCmd::Preview { name, diff } => {
                    cmds::machine_preview(&store, &name, diff, json)
                }
            };
        }
        Command::Pending { action } => {
            return match action.unwrap_or(PendingCmd::List) {
                PendingCmd::List => cmds::pending_list(&store, json),
                PendingCmd::Show { id } => cmds::pending_show(&store, id, json),
                PendingCmd::Drop { ids, all, dry_run } => {
                    cmds::pending_drop(&store, &ids, all, dry_run, json)
                }
                PendingCmd::Apply {
                    ids,
                    all,
                    dry_run,
                    yes,
                } => cmds::pending_apply(&store, &ids, all, dry_run, yes, json),
            };
        }
        Command::Hide { action } => {
            return match action.unwrap_or(HideCmd::List) {
                HideCmd::List => cmds::hide_list(&store, json),
                HideCmd::Add {
                    patterns,
                    apply,
                    dry_run,
                } => cmds::hide_cmd(&store, true, &patterns, apply, dry_run, json),
                HideCmd::Remove {
                    patterns,
                    apply,
                    dry_run,
                } => cmds::hide_cmd(&store, false, &patterns, apply, dry_run, json),
            };
        }
        Command::Stage {
            paths,
            all,
            dry_run,
        } => return cmds::stage_cmd(&store, true, &paths, all, dry_run, json),
        Command::Unstage {
            paths,
            all,
            dry_run,
        } => return cmds::stage_cmd(&store, false, &paths, all, dry_run, json),
        Command::Commit {
            messages,
            amend,
            dry_run,
        } => return cmds::commit_cmd(&store, messages, amend, dry_run, json),
        Command::Diff { path, no_pager } => {
            return cmds::diff_cmd(&store, path.as_deref(), no_pager, json);
        }
        Command::Discard {
            paths,
            yes,
            dry_run,
        } => return cmds::discard_cmd(&store, &paths, yes, dry_run, json),
        Command::Remote { action } => {
            return match action {
                RemoteCmd::Add {
                    name,
                    url,
                    yes,
                    dry_run,
                } => cmds::remote_add(&store, &name, &url, yes, dry_run, json),
                RemoteCmd::Remove { name, yes, dry_run } => {
                    cmds::remote_remove(&store, &name, yes, dry_run, json)
                }
                RemoteCmd::List => cmds::remote_list(&store, json),
            };
        }
        Command::Fetch { all } => return cmds::fetch_cmd(&store, all, json),
        Command::Pull { remote } => return cmds::pull_cmd(&store, remote.as_deref(), json),
        Command::Apply {
            paths,
            all,
            dry_run,
        } => return cmds::apply_cmd(&store, &paths, all, dry_run, json),
        Command::Push {
            remote,
            all,
            yes,
            dry_run,
        } => return cmds::push_cmd(&store, remote.as_deref(), all, yes, dry_run, json),
        Command::Snapshot { action } => {
            return match action {
                SnapshotCmd::List { path } => cmds::snapshot_list(&store, path.as_deref(), json),
                SnapshotCmd::Restore { id, paths, dry_run } => {
                    cmds::snapshot_restore(&store, &id, &paths, dry_run, json)
                }
            };
        }
        Command::Ls { path, all } => return cmds::ls_cmd(&store, path.as_deref(), all, json),
        Command::Tui => {
            if !store.is_initialised() {
                return Err(DwError::usage(
                    "not initialised; run `dw init` or `dw clone <url>` first",
                )
                .into());
            }
            tui::run(store)?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Use `--name` when given; otherwise prompt on a terminal, or fail. With
/// `dry_run` nothing is prompted: no name stays `None`.
fn resolve_name(given: Option<String>, taken: &[String], dry_run: bool) -> Result<Option<String>> {
    if let Some(n) = given {
        machine::validate_name(&n)?;
        return Ok(Some(n));
    }
    if dry_run {
        return Ok(None);
    }
    if !std::io::stdin().is_terminal() {
        return Err(DwError::usage(
            "no machine name given; pass --name <name> (stdin is not a terminal)",
        )
        .into());
    }
    prompt_name(taken).map(Some)
}

fn prompt_name(taken: &[String]) -> Result<String> {
    let host = std::env::var("HOSTNAME")
        .ok()
        .or_else(|| {
            std::process::Command::new("hostname")
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        })
        .unwrap_or_default();
    let suggestion = machine::suggest_name(&host);
    if !taken.is_empty() {
        eprintln!("Machines already registered: {}", taken.join(", "));
    }
    let stdin = std::io::stdin();
    loop {
        eprint!("Name for this machine [{suggestion}]: ");
        std::io::stderr().flush()?;
        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            return Err(DwError::usage("no machine name given").into());
        }
        let name = match line.trim() {
            "" => suggestion.clone(),
            other => other.to_string(),
        };
        match machine::validate_name(&name) {
            Ok(()) => return Ok(name),
            Err(e) => eprintln!("{e}"),
        }
    }
}

fn describe_machine(m: &machine::Machine) -> String {
    format!(
        "{} ({}/{}{})",
        m.name,
        m.platform,
        m.distribution,
        if m.families.is_empty() {
            String::new()
        } else {
            format!(", family {}", m.families.join(", "))
        }
    )
}

fn report_registration(
    store: &Store,
    reg: &repo::Registration,
    verb: &str,
    json: bool,
) -> Result<()> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "repository": store.repo(),
                "machine": reg.machine,
                "registered": reg.created,
            }))?
        );
    } else {
        println!("{verb} repository at {}", store.repo().display());
        println!(
            "{} machine {}",
            if reg.created { "Registered" } else { "Adopted" },
            describe_machine(&reg.machine)
        );
    }
    Ok(())
}

fn print_plan(plan: &serde_json::Value, human: &[String], json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(plan)?);
    } else {
        println!("Dry run: nothing was changed. Would:");
        for line in human {
            println!("  - {line}");
        }
    }
    Ok(())
}

fn cmd_init(
    store: &Store,
    name: Option<String>,
    branch: &str,
    dry_run: bool,
    json: bool,
) -> Result<()> {
    if dry_run {
        repo::require_empty_repo_location(store)?;
        repo::require_git_identity(store)?;
        let detected = machine::detect()?;
        let files = repo::layout_paths();
        let machine = name.as_deref().map(|n| detected.clone().into_machine(n));
        let plan = json!({
            "dry_run": true,
            "command": "init",
            "repository": store.repo(),
            "branch": branch,
            "files": files,
            "commits": ["Initialise Dotwarden repository", "Register machine"],
            "machine": machine,
            "machine_name": if machine.is_none() { json!("prompted for") } else { json!(null) },
            "writes": [store.dirs.chezmoi_config(), store.dirs.machine_toml()],
        });
        let mut human = vec![format!(
            "create the repository at {} on branch {branch}",
            store.repo().display()
        )];
        human.extend(files.iter().map(|f| format!("write {f}")));
        human.push(match &machine {
            Some(m) => format!("register machine {}", describe_machine(m)),
            None => format!(
                "register this computer ({}/{}) under a name you are prompted for",
                detected.platform, detected.distribution
            ),
        });
        human.push(format!("write {}", store.dirs.chezmoi_config().display()));
        human.push(format!("write {}", store.dirs.machine_toml().display()));
        return print_plan(&plan, &human, json);
    }
    let name = name.expect("a name is always resolved outside dry runs");
    let _lock = store.lock()?;
    repo::init(store, branch)?;
    match repo::register_machine(store, &name, false) {
        Ok(reg) => report_registration(store, &reg, "Initialised", json),
        Err(e) => {
            let _ = std::fs::remove_dir_all(store.repo());
            Err(e)
        }
    }
}

fn cmd_clone(
    store: &Store,
    url: &str,
    name: Option<String>,
    adopt: bool,
    dry_run: bool,
    json: bool,
) -> Result<()> {
    if let Some(n) = &name {
        machine::validate_name(n)?;
    } else if !dry_run && !std::io::stdin().is_terminal() {
        return Err(DwError::usage(
            "no machine name given; pass --name <name> (stdin is not a terminal)",
        )
        .into());
    }
    if dry_run {
        repo::require_empty_repo_location(store)?;
        repo::require_git_identity(store)?;
        let detected = machine::detect()?;
        let machine = name.as_deref().map(|n| detected.clone().into_machine(n));
        let plan = json!({
            "dry_run": true,
            "command": "clone",
            "url": url,
            "repository": store.repo(),
            "adopt": adopt,
            "machine": machine,
            "machine_name": if machine.is_none() { json!("prompted for") } else { json!(null) },
            "commits": ["Register machine"],
            "writes": [store.dirs.chezmoi_config(), store.dirs.machine_toml()],
            "note": "the registry of the remote is only known after cloning, so name conflicts are not checked",
        });
        let human = vec![
            format!("clone {url} into {}", store.repo().display()),
            match &machine {
                Some(m) => format!(
                    "register machine {}{}",
                    describe_machine(m),
                    if adopt {
                        " (adopting an existing entry)"
                    } else {
                        ""
                    }
                ),
                None => "register this computer under a name you are prompted for".to_string(),
            },
            format!("write {}", store.dirs.chezmoi_config().display()),
            format!("write {}", store.dirs.machine_toml().display()),
            "name conflicts with the remote registry are only checked after cloning".to_string(),
        ];
        return print_plan(&plan, &human, json);
    }
    let _lock = store.lock()?;
    repo::clone(store, url)?;
    let result = (|| {
        let name = match name {
            Some(n) => n,
            None => {
                let taken: Vec<String> = repo::read_registry(&store.repo())?
                    .into_iter()
                    .map(|m| m.name)
                    .collect();
                resolve_name(None, &taken, false)?.expect("prompted")
            }
        };
        repo::register_machine(store, &name, adopt)
    })();
    match result {
        Ok(reg) => report_registration(store, &reg, "Cloned", json),
        Err(e) => {
            let _ = std::fs::remove_dir_all(store.repo());
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walk(cmd: &clap::Command, path: String, problems: &mut Vec<String>) {
        for arg in cmd.get_arguments() {
            if arg.get_help().is_none() && arg.get_id() != "help" && arg.get_id() != "version" {
                problems.push(format!(
                    "{path}: argument `{}` has no help text",
                    arg.get_id()
                ));
            }
        }
        for sub in cmd.get_subcommands().filter(|s| s.get_name() != "help") {
            let p = format!("{path} {}", sub.get_name());
            if sub.get_about().is_none() {
                problems.push(format!("{p}: no description"));
            }
            walk(sub, p, problems);
        }
    }

    #[test]
    fn every_command_flag_and_argument_has_help_text() {
        let mut problems = Vec::new();
        walk(&Cli::command(), "dotwarden".into(), &mut problems);
        assert!(problems.is_empty(), "{problems:#?}");
    }

    #[test]
    fn json_is_accepted_after_the_subcommand() {
        let cli = Cli::try_parse_from(["dw", "status", "--json"]).unwrap();
        assert!(cli.json);
    }
}
