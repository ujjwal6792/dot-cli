//! Remotes, fetch, pull and push (spec §6, ADR 0006).
//!
//! Public remotes are refused: before a remote is added and before every push,
//! an anonymous `git ls-remote` over HTTPS on a known host decides. Other hosts
//! need a one-time human confirmation stored in `machine.toml`.

use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::error::{DwError, Kind};
use crate::exec::{self, CancelToken};
use crate::fsutil;
use crate::pending::{Action, Item, Preview};
use crate::store::Store;
use crate::track::{self, Ctx};

/// Asks a human whether a remote is private.
pub type Confirm<'a> = &'a dyn Fn(&str) -> Result<bool>;

/// Hosts whose repositories can be public; checked with an anonymous request.
const KNOWN_HOSTS: [&str; 4] = ["github.com", "gitlab.com", "codeberg.org", "bitbucket.org"];
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);
/// Test hook: a command run as `<cmd> <https-url>`; exit 0 = public, 1 = not
/// public, anything else = unreachable. Replaces the network probe.
pub const PROBE_ENV: &str = "DOTWARDEN_VISIBILITY_PROBE";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Host {
    /// A path or `file://` URL: never reachable by the public.
    Local,
    /// One of [`KNOWN_HOSTS`], with the anonymous HTTPS URL to probe.
    Known(String),
    Unknown(String),
}

/// `(host, path)` of an https, ssh, git or scp-style URL.
fn split_url(url: &str) -> Option<(String, String)> {
    if let Some((scheme, rest)) = url.split_once("://") {
        if scheme == "file" {
            return None;
        }
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host = authority.rsplit('@').next()?;
        let host = host.split(':').next()?;
        return Some((host.to_ascii_lowercase(), path.to_string()));
    }
    let (left, path) = url.split_once(':')?;
    if left.contains('/') || left.is_empty() || url.starts_with(['/', '.', '~']) {
        return None;
    }
    let host = left.rsplit('@').next()?;
    Some((
        host.to_ascii_lowercase(),
        path.trim_start_matches('/').to_string(),
    ))
}

pub fn host_of(url: &str) -> Host {
    match split_url(url) {
        None => Host::Local,
        Some((host, path)) if KNOWN_HOSTS.contains(&host.as_str()) => {
            Host::Known(format!("https://{host}/{}", path.trim_start_matches('/')))
        }
        Some((host, _)) => Host::Unknown(host),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Visibility {
    Public,
    NotPublic,
    Unreachable(String),
}

const OFFLINE_SIGNS: [&str; 8] = [
    "could not resolve",
    "failed to connect",
    "timed out",
    "network is unreachable",
    "connection refused",
    "couldn't connect",
    "name or service not known",
    "temporary failure in name resolution",
];

/// Whether `https_url` can be read without credentials.
pub fn probe(store: &Store, https_url: &str) -> Visibility {
    if let Some(fake) = store.env().get(std::ffi::OsStr::new(PROBE_ENV)) {
        return match Command::new(fake).arg(https_url).output() {
            Ok(o) => match o.status.code() {
                Some(0) => Visibility::Public,
                Some(1) => Visibility::NotPublic,
                _ => Visibility::Unreachable("probe command failed".into()),
            },
            Err(e) => Visibility::Unreachable(e.to_string()),
        };
    }
    let mut cmd = store.git_command(&store.dirs.home);
    cmd.args([
        "-c",
        "credential.helper=",
        "-c",
        "core.askPass=",
        "ls-remote",
        "--quiet",
    ])
    .arg(https_url)
    .arg("HEAD")
    .env("GIT_TERMINAL_PROMPT", "0")
    .env("GIT_ASKPASS", "/bin/false")
    .env("SSH_ASKPASS", "/bin/false");
    let cancel = CancelToken::new();
    let (done, wait) = mpsc::channel::<()>();
    let timer = cancel.clone();
    std::thread::spawn(move || {
        if wait.recv_timeout(PROBE_TIMEOUT).is_err() {
            timer.cancel();
        }
    });
    let out = exec::run(cmd, Some(&cancel));
    let _ = done.send(());
    match out {
        Err(e) => Visibility::Unreachable(format!("{e:#}")),
        Ok(o) if o.success() => Visibility::Public,
        Ok(o) => {
            let err = o.stderr.to_lowercase();
            if OFFLINE_SIGNS.iter().any(|s| err.contains(s)) {
                Visibility::Unreachable(o.stderr.trim().lines().last().unwrap_or("").to_string())
            } else {
                Visibility::NotPublic
            }
        }
    }
}

// ---- confirmations in machine.toml --------------------------------------

fn read_machine_table(store: &Store) -> Result<toml::Table> {
    let path = store.dirs.machine_toml();
    match std::fs::read_to_string(&path) {
        Ok(t) => toml::from_str(&t).with_context(|| format!("parsing {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(toml::Table::new()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// Remote URLs a human confirmed to be private.
pub fn confirmed_private(store: &Store) -> Result<Vec<String>> {
    Ok(read_machine_table(store)?
        .get("confirmed_private")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default())
}

pub fn confirm_private(store: &Store, url: &str) -> Result<()> {
    let mut t = read_machine_table(store)?;
    let mut urls = confirmed_private(store)?;
    if !urls.iter().any(|u| u == url) {
        urls.push(url.to_string());
    }
    t.insert(
        "confirmed_private".into(),
        toml::Value::Array(urls.into_iter().map(toml::Value::String).collect()),
    );
    fsutil::write_atomic(&store.dirs.machine_toml(), toml::to_string(&t)?)
}

fn public_refusal(url: &str) -> anyhow::Error {
    DwError::new(
        Kind::PublicRemote,
        format!(
            "{url} is public: secret names, variables and the machine registry would be readable by anyone. \
             Use a private repository"
        ),
    )
    .into()
}

/// Refuse a public remote. `confirm` may ask a human whether an unverifiable
/// host is private (`None`: nobody can be asked). Returns a warning when the
/// check could not run (offline).
pub fn ensure_not_public(
    store: &Store,
    url: &str,
    confirm: Option<Confirm<'_>>,
) -> Result<Option<String>> {
    match host_of(url) {
        Host::Local => Ok(None),
        Host::Known(https) => Ok(match probe(store, &https) {
            Visibility::Public => return Err(public_refusal(url)),
            Visibility::NotPublic => None,
            Visibility::Unreachable(why) => Some(format!(
                "could not check whether {url} is public ({why}); skipped"
            )),
        }),
        Host::Unknown(host) => {
            if confirmed_private(store)?.iter().any(|u| u == url) {
                return Ok(None);
            }
            let needs = || {
                DwError::new(
                    Kind::NeedsApproval,
                    format!(
                        "{host} cannot be checked for visibility: a human must confirm that {url} is private \
                         (run `dw remote add` in a terminal)"
                    ),
                )
            };
            match confirm {
                Some(ask) if ask(url)? => {
                    confirm_private(store, url)?;
                    Ok(None)
                }
                Some(_) => bail!("not confirmed as private; {url} was not added"),
                None => Err(needs().into()),
            }
        }
    }
}

// ---- remotes -------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct RemoteInfo {
    pub name: String,
    pub url: String,
    /// `local`, `known-host` (checked before every push), `confirmed-private` or `unverified`.
    pub visibility: &'static str,
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(['-', '.'])
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// The configured URL as written (git's `get-url` applies `insteadOf` rewrites).
pub fn remote_url(store: &Store, name: &str) -> Result<Option<String>> {
    let key = format!("remote.{name}.url");
    let out = exec::run(
        {
            let mut c = store.git_command(&store.repo());
            c.args(["config", "--get", &key]);
            c
        },
        None,
    )?;
    Ok(out.success().then(|| out.stdout.trim().to_string()))
}

pub fn names(store: &Store) -> Result<Vec<String>> {
    Ok(store
        .git(&["remote"])?
        .stdout
        .lines()
        .map(str::to_string)
        .collect())
}

pub fn list(store: &Store) -> Result<Vec<RemoteInfo>> {
    let confirmed = confirmed_private(store)?;
    let mut out = Vec::new();
    for name in names(store)? {
        let url = remote_url(store, &name)?.unwrap_or_default();
        let visibility = match host_of(&url) {
            Host::Local => "local",
            Host::Known(_) => "known-host",
            Host::Unknown(_) if confirmed.contains(&url) => "confirmed-private",
            Host::Unknown(_) => "unverified",
        };
        out.push(RemoteInfo {
            name,
            url,
            visibility,
        });
    }
    Ok(out)
}

pub fn plan_add(store: &Store, name: &str, url: &str) -> Result<()> {
    if !valid_name(name) {
        return Err(DwError::usage(format!("{name:?} is not a valid remote name")).into());
    }
    if url.is_empty() || url.starts_with('-') {
        return Err(DwError::usage(format!("{url:?} is not a valid remote URL")).into());
    }
    if let Some(old) = remote_url(store, name)? {
        bail!("remote {name} already exists ({old})");
    }
    Ok(())
}

/// Add a remote. The caller holds the mutation lock.
pub fn add_locked(
    store: &Store,
    name: &str,
    url: &str,
    confirm: Option<Confirm<'_>>,
) -> Result<Option<String>> {
    plan_add(store, name, url)?;
    let warning = ensure_not_public(store, url, confirm)?;
    store.git(&["remote", "add", name, url])?;
    Ok(warning)
}

/// Remove a remote. The caller holds the mutation lock.
pub fn remove_locked(store: &Store, name: &str) -> Result<()> {
    if remote_url(store, name)?.is_none() {
        bail!("no remote named {name}");
    }
    store.git(&["remote", "remove", name])?;
    Ok(())
}

/// Ask on the terminal whether an unverifiable remote is private; nobody can
/// answer without one.
pub fn ask_tty(url: &str) -> Result<bool> {
    use std::io::{BufRead, IsTerminal, Write};
    if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
        return Err(DwError::new(
            Kind::NeedsApproval,
            format!("a human must confirm in a terminal that {url} is private"),
        )
        .into());
    }
    print!("{url} cannot be checked for visibility. Is it a private repository? [y/N] ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes"))
}

// ---- pending items -------------------------------------------------------

fn item(
    store: &Store,
    requester: &str,
    action: Action,
    summary: String,
    notes: Vec<String>,
) -> Result<Item> {
    let preview = Preview {
        applies_to: None,
        files: Vec::new(),
        data: Vec::new(),
        rules: Vec::new(),
        notes,
        ..Preview::default()
    };
    track::new_item(
        store,
        requester,
        action,
        summary,
        preview,
        &Ctx::load(store)?,
    )
}

/// Refuse a public known-host remote before anything else happens (no pending item).
fn refuse_public(store: &Store, url: &str) -> Result<()> {
    match host_of(url) {
        Host::Known(https) if probe(store, &https) == Visibility::Public => {
            Err(public_refusal(url))
        }
        _ => Ok(()),
    }
}

pub fn plan_add_item(store: &Store, name: &str, url: &str, requester: &str) -> Result<Item> {
    plan_add(store, name, url)?;
    refuse_public(store, url)?;
    let notes = match host_of(url) {
        Host::Local => vec!["a local path: never public".to_string()],
        Host::Known(_) => {
            vec!["visibility is checked anonymously over HTTPS when this is applied".into()]
        }
        Host::Unknown(h) => vec![format!(
            "{h} cannot be checked: applying needs a human to confirm in a terminal that {url} is private"
        )],
    };
    item(
        store,
        requester,
        Action::RemoteAdd {
            name: name.into(),
            url: url.into(),
        },
        format!("Add remote {name} ({url}) (requested by {requester})"),
        notes,
    )
}

pub fn plan_remove_item(store: &Store, name: &str, requester: &str) -> Result<Item> {
    let url = remote_url(store, name)?.ok_or_else(|| anyhow::anyhow!("no remote named {name}"))?;
    item(
        store,
        requester,
        Action::RemoteRemove { name: name.into() },
        format!("Remove remote {name} ({url}) (requested by {requester})"),
        Vec::new(),
    )
}

pub fn plan_push_item(store: &Store, targets: &[String], requester: &str) -> Result<Item> {
    let chosen = if targets.is_empty() {
        names(store)?
    } else {
        targets.to_vec()
    };
    if chosen.is_empty() {
        bail!("no remotes; add one with `dw remote add NAME URL`");
    }
    let mut notes = Vec::new();
    for r in &chosen {
        let url = remote_url(store, r)?.ok_or_else(|| anyhow::anyhow!("no remote named {r}"))?;
        refuse_public(store, &url)?;
        notes.push(match unpushed(store, r) {
            Some(n) => format!("{r} ({url}): {n} commit(s) to push"),
            None => format!("{r} ({url})"),
        });
    }
    notes.push(
        "visibility is checked again right before pushing; public remotes are refused".into(),
    );
    item(
        store,
        requester,
        Action::Push {
            remotes: chosen.clone(),
        },
        format!(
            "Push {} to {} (requested by {requester})",
            current_branch(store)?,
            chosen.join(", ")
        ),
        notes,
    )
}

/// Run an approved remote or push item (the caller holds the lock).
pub fn exec_action(store: &Store, action: &Action) -> Result<String> {
    match action {
        Action::RemoteAdd { name, url } => {
            let w = add_locked(store, name, url, Some(&ask_tty))?;
            Ok(w.unwrap_or_else(|| format!("added remote {name}")))
        }
        Action::RemoteRemove { name } => {
            remove_locked(store, name)?;
            Ok(format!("removed remote {name}"))
        }
        Action::Push { remotes } => {
            let done = push(store, remotes)?;
            Ok(done
                .iter()
                .map(|p| format!("pushed {} to {}", p.branch, p.remote))
                .collect::<Vec<_>>()
                .join(", "))
        }
        _ => unreachable!("not a remote action"),
    }
}

// ---- git helpers ---------------------------------------------------------

/// Git for network operations: never asks for a password (a prompt would hang
/// the TUI), and ssh runs in batch mode unless the user set their own command.
fn net_git(store: &Store) -> Command {
    let mut cmd = store.git_command(&store.repo());
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    if store
        .env()
        .get(std::ffi::OsStr::new("GIT_SSH_COMMAND"))
        .is_none()
    {
        cmd.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    }
    cmd
}

fn net_run(store: &Store, args: &[&str], cancel: Option<&CancelToken>) -> Result<exec::Output> {
    let mut cmd = net_git(store);
    cmd.args(args);
    exec::run_ok(cmd, cancel)
}

fn text(store: &Store, args: &[&str]) -> Option<String> {
    let out = store.git(args).ok()?;
    let s = out.stdout.trim().to_string();
    (!s.is_empty()).then_some(s)
}

pub fn current_branch(store: &Store) -> Result<String> {
    text(store, &["symbolic-ref", "--short", "HEAD"])
        .ok_or_else(|| anyhow::anyhow!("HEAD is not on a branch"))
}

/// The remote for fetch and pull: the branch's own, else `origin`, else the only one.
pub fn default_remote(store: &Store) -> Result<String> {
    let all = names(store)?;
    if let Ok(b) = current_branch(store)
        && let Some(r) = text(store, &["config", "--get", &format!("branch.{b}.remote")])
        && all.contains(&r)
    {
        return Ok(r);
    }
    if all.iter().any(|n| n == "origin") {
        return Ok("origin".into());
    }
    match all.as_slice() {
        [only] => Ok(only.clone()),
        [] => bail!("no remotes; add one with `dw remote add NAME URL`"),
        _ => bail!("several remotes and none is the default; name one"),
    }
}

/// The ref the branch is compared with: its upstream, else `<default remote>/<branch>`.
pub fn tracking_ref(store: &Store) -> Option<String> {
    if let Some(u) = text(
        store,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
    ) {
        return Some(u);
    }
    let r = format!(
        "{}/{}",
        default_remote(store).ok()?,
        current_branch(store).ok()?
    );
    store
        .git(&[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/remotes/{r}"),
        ])
        .ok()
        .map(|_| r)
}

/// `(ahead, behind)` of HEAD against `reference`.
fn counts(store: &Store, reference: &str) -> Result<(u32, u32)> {
    let out = store.git(&[
        "rev-list",
        "--left-right",
        "--count",
        &format!("HEAD...{reference}"),
    ])?;
    let mut it = out.stdout.split_whitespace().map(|n| n.parse::<u32>());
    match (it.next(), it.next()) {
        (Some(Ok(a)), Some(Ok(b))) => Ok((a, b)),
        _ => bail!("cannot read the commit counts against {reference}"),
    }
}

// ---- fetch ---------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct Fetched {
    pub remotes: Vec<String>,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
}

/// Fetch one remote (the default) or all. Never touches HOME or the working tree.
pub fn fetch(store: &Store, all: bool, cancel: Option<&CancelToken>) -> Result<Fetched> {
    let remotes = if all {
        names(store)?
    } else {
        vec![default_remote(store)?]
    };
    if remotes.is_empty() {
        bail!("no remotes; add one with `dw remote add NAME URL`");
    }
    for r in &remotes {
        net_run(store, &["fetch", "--quiet", "--", r], cancel)
            .with_context(|| format!("fetching {r}"))?;
    }
    let (ahead, behind) = match tracking_ref(store).and_then(|r| counts(store, &r).ok()) {
        Some((a, b)) => (Some(a), Some(b)),
        None => (None, None),
    };
    Ok(Fetched {
        remotes,
        ahead,
        behind,
    })
}

// ---- pull ----------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct Pulled {
    pub remote: String,
    pub commits: u32,
    pub files: usize,
}

/// Fast-forward the branch to the remote's. Diverged history is refused.
pub fn pull(store: &Store, remote: Option<&str>) -> Result<Pulled> {
    let remote = match remote {
        Some(r) => r.to_string(),
        None => default_remote(store)?,
    };
    let branch = current_branch(store)?;
    net_run(store, &["fetch", "--quiet", "--", &remote], None)
        .with_context(|| format!("fetching {remote}"))?;
    let reference = format!("{remote}/{branch}");
    if store
        .git(&[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/remotes/{reference}"),
        ])
        .is_err()
    {
        bail!("{remote} has no branch {branch}; nothing to pull");
    }
    let _lock = store.lock()?;
    crate::track::ensure_repo_clean(store)?;
    let (ahead, behind) = counts(store, &reference)?;
    if behind == 0 {
        return Ok(Pulled {
            remote,
            commits: 0,
            files: 0,
        });
    }
    let base = text(store, &["merge-base", "HEAD", &reference]).unwrap_or_default();
    if ahead > 0 {
        let only_data = store
            .git(&["diff", "--name-only", &format!("{reference}...HEAD")])?
            .stdout
            .lines()
            .all(|p| p.starts_with("home/.chezmoidata/"));
        if !only_data || !rebase_onto(store, &reference) {
            return Err(DwError::new(
                Kind::Error,
                format!(
                    "diverged: this machine has {ahead} commit(s) {reference} lacks and {reference} has {behind} this machine lacks; \
                     merge/rebase arrives later — use `G` lazygit"
                ),
            )
            .into());
        }
    } else {
        store.git(&["merge", "--ff-only", "--quiet", &reference])?;
    }
    let files = store
        .git(&["diff", "--name-only", &format!("{base}..HEAD")])?
        .stdout
        .lines()
        .count();
    Ok(Pulled {
        remote,
        commits: behind,
        files,
    })
}

/// Replay the local commits on `reference`; on any conflict abort so the
/// repository is exactly as before. Only used for commits that touch metadata.
fn rebase_onto(store: &Store, reference: &str) -> bool {
    if store.git(&["rebase", "--quiet", reference]).is_ok() {
        return true;
    }
    let _ = store.git(&["rebase", "--abort"]);
    false
}

// ---- push ----------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct Pushed {
    pub remote: String,
    pub branch: String,
    pub warning: Option<String>,
}

/// Commits `remote` lacks, for previews.
pub fn unpushed(store: &Store, remote: &str) -> Option<usize> {
    let branch = current_branch(store).ok()?;
    let range = if store
        .git(&[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/remotes/{remote}/{branch}"),
        ])
        .is_ok()
    {
        format!("{remote}/{branch}..HEAD")
    } else {
        "HEAD".to_string()
    };
    text(store, &["rev-list", "--count", &range])?.parse().ok()
}

/// Push the branch to `targets` (all remotes when empty). Each remote is
/// re-checked for visibility first; one public remote stops everything before
/// anything is pushed.
pub fn push(store: &Store, targets: &[String]) -> Result<Vec<Pushed>> {
    let all = names(store)?;
    let chosen: Vec<String> = if targets.is_empty() {
        all.clone()
    } else {
        targets.to_vec()
    };
    if chosen.is_empty() {
        bail!("no remotes; add one with `dw remote add NAME URL`");
    }
    let branch = current_branch(store)?;
    let mut warnings = Vec::new();
    for r in &chosen {
        let url = remote_url(store, r)?.ok_or_else(|| anyhow::anyhow!("no remote named {r}"))?;
        warnings.push(ensure_not_public(store, &url, None)?);
    }
    let mut out = Vec::new();
    for (r, warning) in chosen.iter().zip(warnings) {
        let has_upstream = text(
            store,
            &["config", "--get", &format!("branch.{branch}.remote")],
        )
        .is_some();
        let mut args = vec!["push", "--quiet"];
        if !has_upstream {
            args.push("--set-upstream");
        }
        args.extend(["--", r, &branch]);
        net_run(store, &args, None).with_context(|| {
            format!("pushing {branch} to {r} (if it was rejected, `dw pull` first)")
        })?;
        out.push(Pushed {
            remote: r.clone(),
            branch: branch.clone(),
            warning,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_probe_urls_come_from_every_url_form() {
        let want = Host::Known("https://github.com/me/dots.git".into());
        for u in [
            "git@github.com:me/dots.git",
            "ssh://git@github.com/me/dots.git",
            "ssh://git@github.com:22/me/dots.git",
            "https://user:pw@GitHub.com/me/dots.git",
        ] {
            assert_eq!(host_of(u), want, "{u}");
        }
        assert_eq!(
            host_of("git@git.example.org:me/x"),
            Host::Unknown("git.example.org".into())
        );
        assert_eq!(host_of("/srv/git/dots.git"), Host::Local);
        assert_eq!(host_of("file:///srv/dots.git"), Host::Local);
        assert_eq!(host_of("../dots.git"), Host::Local);
    }
}
