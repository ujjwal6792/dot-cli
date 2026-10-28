//! `dw status`: machine, repository and pending-list summary.

use anyhow::Result;
use serde::Serialize;

use crate::exec::CancelToken;
use crate::machine::Machine;
use crate::repo;
use crate::store::Store;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RepoStatus {
    pub branch: Option<String>,
    pub head: Option<String>,
    /// Number of paths with uncommitted changes in the repository (should be 0).
    pub changed_paths: usize,
    pub upstream: Option<String>,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    pub remotes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MachineStatus {
    /// Registry name stored in `machine.toml`.
    pub name: String,
    /// Registry entry, if the name is still present in the registry.
    pub registered: Option<Machine>,
}

impl MachineStatus {
    /// What is wrong when this computer's name is not in the registry.
    pub fn problem(&self) -> Option<String> {
        self.registered.is_none().then(|| {
            format!(
                "{} is not in the machine registry (renamed or retired elsewhere?); see `dw machine list`, then `dw machine use NAME`",
                self.name
            )
        })
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Status {
    pub initialised: bool,
    pub repo_path: String,
    pub machine: Option<MachineStatus>,
    pub repo: Option<RepoStatus>,
    pub pending: usize,
    /// Tracked files and recorded tracked folders.
    pub tracked_files: usize,
    pub tracked_folders: usize,
    /// Untracked, unhidden files inside tracked folders (relative to HOME).
    pub new_files: Vec<String>,
    /// Local, new, staged, incoming and conflicting changes.
    pub changes: Vec<crate::changes::Change>,
    pub vault: &'static str,
}

fn trimmed(store: &Store, args: &[&str], cancel: Option<&CancelToken>) -> Option<String> {
    let out = store.git_cancellable(args, cancel).ok()?;
    let s = out.stdout.trim().to_string();
    (!s.is_empty()).then_some(s)
}

pub fn collect(store: &Store, cancel: Option<&CancelToken>) -> Result<Status> {
    let repo_path = store.repo().display().to_string();
    if !store.is_initialised() {
        return Ok(Status {
            initialised: false,
            repo_path,
            machine: None,
            repo: None,
            pending: 0,
            tracked_files: 0,
            tracked_folders: 0,
            new_files: Vec::new(),
            changes: Vec::new(),
            vault: "none",
        });
    }
    let machine = store.local_machine_name()?.map(|name| {
        let registered = repo::read_registry(&store.repo())
            .ok()
            .and_then(|r| r.into_iter().find(|m| m.name == name));
        MachineStatus { name, registered }
    });
    let branch = trimmed(store, &["rev-parse", "--abbrev-ref", "HEAD"], cancel);
    let head = trimmed(store, &["rev-parse", "--short", "HEAD"], cancel);
    let changed_paths = store
        .git_cancellable(&["--no-optional-locks", "status", "--porcelain"], cancel)?
        .stdout
        .lines()
        .count();
    let upstream = crate::remote::tracking_ref(store);
    let (ahead, behind) = match &upstream {
        Some(u) => trimmed(
            store,
            &[
                "rev-list",
                "--left-right",
                "--count",
                &format!("HEAD...{u}"),
            ],
            cancel,
        )
        .and_then(|s| {
            let mut it = s.split_whitespace().map(|n| n.parse::<u32>().ok());
            Some((it.next()??, it.next()??))
        })
        .map_or((None, None), |(a, b)| (Some(a), Some(b))),
        None => (None, None),
    };
    let remotes = store
        .git_cancellable(&["remote"], cancel)?
        .stdout
        .lines()
        .map(str::to_string)
        .collect();
    let pending = crate::pending::load(store)?.items.len();
    let tracked = crate::source::load(&store.repo())?;
    let rules = crate::hide::HideRules::for_store(store)?;
    let new_files = crate::track::new_files(&store.dirs.home, &tracked, &rules, cancel)?;
    // chezmoi needs the generated config (and so a registered machine) to render targets.
    let changes = if machine.as_ref().is_some_and(|m| m.registered.is_some())
        && store.dirs.chezmoi_config().exists()
    {
        crate::changes::classify(store)?
    } else {
        Vec::new()
    };
    Ok(Status {
        initialised: true,
        repo_path,
        machine,
        repo: Some(RepoStatus {
            branch,
            head,
            changed_paths,
            upstream,
            ahead,
            behind,
            remotes,
        }),
        pending,
        tracked_files: tracked.files.len(),
        tracked_folders: tracked.folders.len(),
        new_files,
        changes,
        vault: "none",
    })
}

impl Status {
    pub fn render_human(&self) -> String {
        if !self.initialised {
            return format!(
                "Not initialised. Run `dw init` or `dw clone <url>`.\nRepository: {}\n",
                self.repo_path
            );
        }
        let mut s = String::new();
        match &self.machine {
            Some(m) => match &m.registered {
                Some(r) => s.push_str(&format!(
                    "Machine:    {} ({}/{}{})\n",
                    m.name,
                    r.platform,
                    r.distribution,
                    if r.families.is_empty() {
                        String::new()
                    } else {
                        format!(", family {}", r.families.join(", "))
                    }
                )),
                None => s.push_str(&format!(
                    "Machine:    {}\n",
                    m.problem().unwrap_or_default()
                )),
            },
            None => s.push_str("Machine:    not registered on this computer\n"),
        }
        if let Some(r) = &self.repo {
            s.push_str(&format!(
                "Branch:     {} @ {}\n",
                r.branch.as_deref().unwrap_or("?"),
                r.head.as_deref().unwrap_or("no commits")
            ));
            match (&r.upstream, r.ahead, r.behind) {
                (Some(u), Some(a), Some(b)) => {
                    s.push_str(&format!("Upstream:   {u} (ahead {a}, behind {b})\n"))
                }
                _ => s.push_str("Upstream:   none\n"),
            }
            s.push_str(&format!(
                "Remotes:    {}\n",
                if r.remotes.is_empty() {
                    "none".into()
                } else {
                    r.remotes.join(", ")
                }
            ));
        }
        s.push_str(&format!("Pending:    {}\n", self.pending));
        s.push_str(&format!(
            "Tracked:    {} files, {} folders\n",
            self.tracked_files, self.tracked_folders
        ));
        use crate::changes::State;
        for (state, title) in [
            (State::Staged, "Staged"),
            (State::Local, "Local"),
            (State::New, "New (untracked, in tracked folders)"),
            (State::Incoming, "Incoming"),
            (State::Conflict, "Conflicts"),
        ] {
            let list: Vec<_> = self.changes.iter().filter(|c| c.state == state).collect();
            if list.is_empty() {
                continue;
            }
            s.push_str(&format!("{title}: {}\n", list.len()));
            for c in list.iter().take(20) {
                let shown = if c.kind == "repo" {
                    format!("repo:{}", c.path)
                } else {
                    format!("~/{}", c.path)
                };
                let kind = if state == State::Incoming && c.kind == "new" {
                    "incoming (new file)"
                } else {
                    c.kind
                };
                s.push_str(&format!("  {kind:<9} {shown}\n"));
            }
            if list.len() > 20 {
                s.push_str(&format!("  … and {} more\n", list.len() - 20));
            }
        }
        s.push_str(&format!("Vault:      {}\n", self.vault));
        s.push_str(&format!("Repository: {}\n", self.repo_path));
        s
    }
}
