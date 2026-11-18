# Dotwarden v2 tickets

Draft bodies for the GitHub parent issue and its children. Each ticket is a
vertical slice: CLI + TUI + tests, verified on macOS only (no Linux, Docker or emulated
runs; the owner tests Linux on real machines later). Section numbers refer to
[spec.md](spec.md).

---

## Parent: Spec — Dotwarden v2

Dotwarden (`dotwarden` / `dw`) replaces dot v1 with a self-contained,
lazygit-style, multi-machine dotfiles tool on bundled chezmoi + git + delta, with
an encrypted vault that agents cannot read. Spec: `docs/spec.md` on branch `v2`.
Glossary: `GLOSSARY.md`. Decisions: `docs/adr/0001`–`0009`.

Children: #T1–#T22 below (order to v1.0.0 in "Road to v1.0.0"). v1 issues are
labelled `v1-retired`.

---

## T1: Walking skeleton (bake-off ticket)

Spec §2, §3, §4 (registration only), §9, §10 (shell), §13.

- Cargo project producing `dotwarden` with a `dw` alias.
- Toolchain resolution of bundled chezmoi/git/delta from the bundle layout, with a
  dev-mode script that downloads and verifies the pinned versions into a local
  `libexec/`. Pins in one file.
- Isolated store (§3): explicit chezmoi flags, scrubbed env, `GIT_CONFIG_NOSYSTEM`,
  identity from global gitconfig.
- `dw init` (new repository with layout, `.chezmoiroot`, fixed `.chezmoiignore`,
  default hide rules) and `dw clone <url>`; machine registration (name prompt,
  platform/distribution/families detection, registry entry).
- **Spike, recorded in spec §3**: verify `.chezmoiroot`, leading-dot ignore,
  `include` path resolution, assignment evaluation in the fixed `.chezmoiignore`,
  and a variant-selection helper against the pinned chezmoi.
- `dw status --json` / `dw doctor`.
- TUI shell: panels 1–5 laid out, Files browser over HOME with hide rules and `.`
  toggle, non-blocking job runner, help overlay, quit/resize.

Acceptance: a user's existing `~/.config/chezmoi` and `~/.local/share/chezmoi`
are untouched (test); init/clone/register work on macOS, with Linux
detection covered by unit tests on sample `/etc/os-release` files; TUI
browses HOME at 80×24 and 45×14; the event loop never blocks on I/O (test with a
slow fake tool).

## T2: Track, untrack, tracked folders, pending list

Spec §5 (track/untrack, tracked folders, symlinks, hide), §8 (pending list).

- `Space` in Files adds track/untrack pending items; `v` range select; pending
  pane with per-item preview; one review applies; drop items.
- Tracked folders record the folder; new files appear as New.
- Symlinks tracked as links. Hidden paths refused.
- CLI: `track`, `untrack`, `pending list|drop`, `hide`.

Blocked by T1.

## T3: Change model — local, staged, commit, discard, snapshots

Spec §5, §11 (snapshots).

- Classification Local/New/Staged from chezmoi status + last-written state, with
  a test for every state, including "no change after stage + commit".
- Stage/unstage whole files, `a` stage all, commit (multi-line), amend.
- Discard with snapshot; snapshot retention.
- Diffs rendered through bundled delta in the main view.
- CLI: `stage`, `unstage`, `commit`, `discard`, `diff`, `snapshot list|restore`.

Blocked by T2.

## T4: Remotes, fetch, pull, incoming, apply, push, public-remote refusal

Spec §5 (incoming/apply), §6, ADR 0006.

- Remote add/remove/list; visibility check on add and before push; one-time
  confirmation for unverifiable hosts stored in `machine.toml`.
- Background fetch; pull (fast-forward); Incoming tab; apply per file/all with
  snapshots; push to one/all remotes.
- CLI: `remote`, `fetch`, `pull`, `apply`, `push`.

Blocked by T3.

## T5: Machines and assignments

Spec §4.

- Registry view; `m` assignment editor with include/exclude targets (platform,
  distribution, family, machine), tri-state for mixed selections.
- Assignments stored as data; fixed `.chezmoiignore` evaluates them.
- Rename and retire with full affected-item review; preview-as machine.
- CLI: `assign`, `machine list|add|rename|retire|preview`.

Blocked by T2.

## T6: Variants

Spec §4 (specificity), §5.

- Create a variant for a target from the current HOME content; select by
  specificity; same-level clash shown and blocks only that file.
- Staging on a machine updates that machine's variant.
- CLI: `variant add|list|remove`.

Blocked by T5, T3.

## T7: Values — variables and references

Spec §7 (variables, references).

- Values screen (`V`), variables with per-target overrides, specificity.
- References inserted by dotwarden; files with references stored as templates;
  reverse mapping when staging.
- CLI: `values list|set-variable|wire`.

Blocked by T6.

## T8: Vault — secrets, encrypted files, unlock, recovery

Spec §7, ADR 0004.

- `vault init` (identity, recipient, passphrase-wrapped and recovery-code-wrapped
  identity), `unlock` (TTY only), `lock`, `passwd`, `recover`.
- Unlock session in Keychain / Secret Service / 0600 runtime fallback with expiry.
- Secrets with per-target values; encrypted whole files (`E`); helper rendering
  with per-run token.
- Detection on track/stage (scanner + known values) producing pending items;
  "secret NAME changed" flow.
- Masking everywhere; a test greps all outputs, logs, pending and activity files
  for planted secret values.

Blocked by T7.

## T9: Conflicts

Spec §5 (conflicts).

- Conflict classification; 3-way hunk picker; `e` opens `$EDITOR` (nvim diff mode
  when nvim); result staged.

Blocked by T4.

## T10: Hunk and line staging

Spec §5.

- `Enter` on a change opens a staging view; stage/unstage hunks and lines; files
  with references and secrets stay masked.

Blocked by T3 (and T8 for masked files).

## T11: Git depth — history, branches, stash, merge/rebase

Spec §6, §11 (point-in-time restore).

- Commits tab with per-commit diff and file history; revert; branches
  create/switch/delete with default-branch warning; stash/pop; merge or rebase on
  pull with in-tool conflict resolution; point-in-time restore through Incoming.
- CLI: `log`, `show`, `branch`, `stash`, `revert`, `restore --at`.

Blocked by T9.

## T12: Agents — approvals, activity, CLI contract

Spec §8, §9, ADR 0007.

- Approval-required actions from agents become pending items; exit code 3;
  `approve` in an interactive terminal; TUI marks "requested by".
- Activity log; `Agent:` commit trailer; `DOTWARDEN_AGENT`.
- JSON output and exit codes for every command; help text for every flag;
  completions and man page; `--dry-run` everywhere; bundled agent skill.

Blocked by T4, T8.

## T13: Recovery — undo and archive

Spec §11.

- `z` undo for stage/unstage, track/untrack, assign, discard, last unpushed commit.
- `archive export|import` (git bundle + registry, encrypted with the vault
  passphrase); offline restore of a fresh machine.

Blocked by T8, T11.

---

## Road to v1.0.0 (macOS)

Agreed order after T13. Each milestone ends with the full suite green.

1. **Backup workflow**: T16, T17, T18, T23. Then the **dogfooding gate**: the owner
   imports their bare repository and uses `dw sync` daily for 1–2 weeks; problems
   found become tickets before step 4 (round 1: T24).
2. **TUI**: T19, T20, T21.
3. **Settings**: T14.
4. **Release**: T22, T15.

Linux x86-64 ships as a prerelease asset from the first rc and becomes stable in
v1.1 after the owner verifies it on a real machine (spec §2).

## T16: Backup workflow fixes

Spec §5 (several paths, apply), §6 (clone, pending pushes, HTTPS).

- `track` with several paths adds the valid ones and lists skipped paths with
  reasons; exit 1 only when nothing was added (also `--json`, `--dry-run`).
- `clone` (human) pushes the registration commit after the privacy check; offline,
  refused, failed or agent: stays local with the `dw push` hint.
- A successful push removes the pending push items it satisfied and says so.
- `apply` lists "replaced existing files" (never written by dotwarden here) with
  the snapshot id, also in `--dry-run` and the TUI Incoming summary.
- HTTPS auth failures name the SSH URL; `remote add` warns on HTTPS to known hosts.

Acceptance: one end-to-end test per bullet with real chezmoi/git; the
two-machine clone test shows the second machine's registration on the remote
without a manual push.

Blocked by T13.

## T17: Sync and backup state

Spec §6 (backup state), §6a, ADR 0009.

- `dw sync [-m|-e] [--yes] [--dry-run] [--remote]`: pull, safe apply, stage,
  commit (generated message), push; plan + `Proceed? [Y/n]` in a terminal; stops
  with the right exit code at conflicts (5), locked vault (4), divergence,
  public remote (6); agents: push becomes a pending item (exit 3).
- Last successful push recorded per machine; `dw status` (+ `--json`) shows
  "last pushed N days ago · M unbacked" and warns past `backup_warn_days`
  (minimal preferences reader with defaults; T14 extends it).
- TUI: Status panel line, warning banner, `S` with the plan dialog.

Acceptance: two-machine test where machine B syncs and receives A's edit and A
then syncs and receives B's; a conflict stops before staging; a flagged secret
is skipped, never committed; an agent sync leaves a pending push; `undo` reverts
a sync commit that is on no remote.

Blocked by T16.

## T18: Import from a bare repository or a path list

Spec §5a, ADR 0008.

- `dw import --bare DIR | --from-file LIST|-` with `--dry-run`, `--apply`,
  `--json`; folder collapsing; skipped list (hidden, secret without vault,
  missing); with a vault, secrets move to it as in `track`.
- Items record the source; sync's generated message and the TUI commit prefill
  start with `Import from <DIR> at <hash>`.
- Bundled skill: how an agent imports.

Acceptance: a test builds a bare repo over an isolated HOME (nvim folder fully
listed, another folder partly listed, a `.cache` path, a token file) and checks
the grouping, skips and that the bare repo is byte-identical afterwards.

Blocked by T16.

## T23: HTTPS remotes through credential helpers

Spec §3 item 7, §6; ADR 0002 (reverses the earlier "SSH only" stance).

- Pass `credential.helper`, `credential.<url>.helper`, `useHttpPath` and
  `<url>.username` from the global git config to every git (order and empty
  values kept); the public probe stays anonymous.
- Auth-failure hint mentions `gh auth setup-git`; `dw remote add` warns only when
  no helper applies; `dw doctor` reports it.

Acceptance: with a helper script in an isolated global config, `git credential
fill` through dotwarden's git calls it and the probe command does not.

Blocked by T16.

## T24: Dogfooding round 1 and backup-workflow gaps

Found importing the owner's `mac-configs` bare repository (2026-11-18), plus the
open gaps of the T16–T18 reports. Spec §3, §5, §5a, §6, §6a.

- A release build run from the checkout (`target/release/dw`) finds the
  checkout's `libexec/` like a debug build does.
- Refusing a hidden path names the `dw hide add '!/…'` command that un-hides it
  (track and import); import skipped three `~/Library` configs with only "hidden".
- A file whose mode chezmoi cannot store (0744, stored as 0755) is no change
  right after tracking; it showed as Incoming and sync chmodded it.
- `XDG_DATA_HOME` set by a wrapper (opencode) pointed dotwarden at an empty
  location: name the existing default repository and refuse to create a second.
- A human adding a remote directly removes the agent's pending request for the
  same remote (it was left stale).
- TUI remote add asks for the name, then the URL (done on
  `fix/tui-remote-add-steps`).
- Gaps: the TUI apply prompt lists replaced existing files up front (T16); a
  sync plan that changed after it was shown is shown again instead of run, in the
  terminal and the TUI (T17); the `Proceed?` prompt and `-e` are tested on a pty
  (T17); import with an unlocked vault is tested (T18); folder grouping stops
  walking at the first unlisted file (T18).

Acceptance: one test per bullet where it can fail; the full suite stays green.

## T19: Onboarding

Spec §10 (onboarding and `init` questions).

- TUI without a repository opens the onboarding screen: New / Clone / Import,
  each running the CLI code paths; vault offered (default yes) with recovery
  code confirmation; `gh repo create --private` offered when `gh` is available
  and authenticated, else an SSH URL prompt; first-track suggestions.
- `dw init` in a terminal asks the vault and remote questions (`--no-vault`,
  `--no-remote`); agents never get them.

Acceptance: TUI tests (test backend) for each path with a fake `gh` on `PATH`;
nothing is written before the final confirmation of each step.

Blocked by T17, T18.

## T20: TUI navigation and discoverability

Spec §10 (discoverability and navigation), §5 (hide reload).

- Main-view scrolling (keys and mouse wheel) for diffs, previews and logs.
- Mouse: click focus/select, double-click Enter. `v` range select in Changes.
- Per-panel footer key hints fitting 80 columns.
- Hide rules reloaded in a running TUI after `i` or `dw hide`.

Acceptance: TUI tests at 80×24 and 45×14 for scrolling a 500-line diff, a range
stage, and a hide added from another process disappearing from Files.

Blocked by T13.

## T21: TUI flows that were CLI-only

Spec §10 (flows), §4 (variants), §7 (values).

- Variant remove (`D`), values wire (`w`, path picker) and unset (`-`).
- `@` log with Commands and Activity tabs (values masked, filter by actor).

Acceptance: one TUI test per flow; the secret-masking test covers both log tabs.

Blocked by T20.

## T14: Settings, preferences and integrations

Spec §10, §12.

- Preferences module (all keys, defaults, `toml_edit` writes keeping comments,
  live reload, `dw doctor` reporting); `dw settings list|get|set|unset|keys`.
- Repository-wide settings through pending items; agents cannot change identity
  or signing.
- Settings screen (`,`); keymap remapping by action name with conflict
  reporting; themes `dark|light|terminal` + `[theme.colors]` overrides, delta
  following light/dark.
- `G` lazygit handoff with terminal suspend/restore; `+`/`_` screen modes.

Acceptance: a remapped key works and its old key does not; a duplicate binding
falls back and is reported; a preference edited on disk while the TUI runs takes
effect; repository-wide edits appear as pending items.

Blocked by T21.

## T22: Documentation

Spec §2 (bundle contents).

- README: what it is, install, 5-minute first backup, new-machine restore,
  `dw sync`, Intel Mac/Linux status.
- Guides: "Migrating from a bare git repo" (import, vault, sync) and "Secrets and
  the vault" (passphrase, recovery code, encrypted files, honest boundary).
- Man pages and completions generated at build time into the bundle layout.

Blocked by T14.

## T15: Distribution and multi-machine acceptance

Spec §2, §13.

- GitHub Actions on `v*` tags: fmt, clippy, tests on macOS and Ubuntu; bundles
  for macOS arm64 (stable + rc) and Linux x86-64 (rc only until v1.1) with
  licences, man pages, completions and the SHA256 manifest; GitHub Release with
  `install.sh`.
- `install.sh` (verify, versioned install, `current` symlink, PATH advice,
  refuses unpublished targets on stable); `dw self-update [--check|--pre|
  --rollback|--dry-run]`, disabled for source builds.
- Version 1.0.0 in `Cargo.toml`; first tag `v1.0.0-rc.1`.
- Multi-machine acceptance on macOS: isolated HOME/state directories registered as
  Ubuntu, CachyOS and macOS machines on a shared bare remote, covering
  assignments, variants, values, vault, incoming, conflicts, retire, archive
  restore, import and sync.
- Going public (owner checklist): the history audit is done (no secrets or
  dotfiles; the v1 commits' personal e-mail is accepted); skim PR/issue text and
  Actions logs; tag v1's tip `v1-final`; make `v2` the new `main`; make
  `dot-cli` public; tag `v1.0.0` after the dogfooding gate.
- Packages (`.deb`, AUR, Homebrew) are out of scope for v1.0.

Blocked by all others.
