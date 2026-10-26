//! Command-line interface (spec §9).

use std::io::{BufRead, IsTerminal, Write};
use std::process::ExitCode;

use anyhow::{Result, bail};
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use serde_json::json;

use crate::machine;
use crate::repo::{self, DEFAULT_BRANCH};
use crate::store::Store;
use crate::{doctor, status, tui};

/// Dotfiles, secrets and variables in sync across your machines.
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
    },
    /// Show this machine, the repository and the pending list.
    Status,
    /// Check tool versions, paths, the registered machine and repository health.
    Doctor,
    /// Open the terminal UI (the default when no command is given).
    Tui,
}

pub fn main(bin_name: &'static str) -> ExitCode {
    let matches = Cli::command().name(bin_name).get_matches();
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(c) => c,
        Err(e) => e.exit(),
    };
    match run(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(1)
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    let store = Store::from_process()?;
    let json = cli.json;
    match cli.command.unwrap_or(Command::Tui) {
        Command::Init { name, branch } => {
            let name = resolve_name(name, &[])?;
            cmd_init(&store, &name, &branch, json)?;
        }
        Command::Clone { url, name, adopt } => cmd_clone(&store, &url, name, adopt, json)?,
        Command::Status => {
            store.sync_chezmoi_config()?;
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
                return Ok(ExitCode::from(1));
            }
        }
        Command::Tui => {
            if !store.is_initialised() {
                bail!("not initialised; run `dw init` or `dw clone <url>` first");
            }
            store.sync_chezmoi_config()?;
            tui::run(store)?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Use `--name` when given; otherwise prompt on a terminal, or fail.
fn resolve_name(given: Option<String>, taken: &[String]) -> Result<String> {
    if let Some(n) = given {
        machine::validate_name(&n)?;
        return Ok(n);
    }
    if !std::io::stdin().is_terminal() {
        bail!("no machine name given; pass --name <name> (stdin is not a terminal)");
    }
    prompt_name(taken)
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
            bail!("no machine name given");
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
        let m = &reg.machine;
        println!("{verb} repository at {}", store.repo().display());
        println!(
            "{} machine {} ({}/{}{})",
            if reg.created { "Registered" } else { "Adopted" },
            m.name,
            m.platform,
            m.distribution,
            if m.families.is_empty() {
                String::new()
            } else {
                format!(", family {}", m.families.join(", "))
            }
        );
    }
    Ok(())
}

fn cmd_init(store: &Store, name: &str, branch: &str, json: bool) -> Result<()> {
    repo::init(store, branch)?;
    match repo::register_machine(store, name, false) {
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
    json: bool,
) -> Result<()> {
    if let Some(n) = &name {
        machine::validate_name(n)?;
    } else if !std::io::stdin().is_terminal() {
        bail!("no machine name given; pass --name <name> (stdin is not a terminal)");
    }
    repo::clone(store, url)?;
    let result = (|| {
        let name = match name {
            Some(n) => n,
            None => {
                let taken: Vec<String> = repo::read_registry(&store.repo())?
                    .into_iter()
                    .map(|m| m.name)
                    .collect();
                resolve_name(None, &taken)?
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
