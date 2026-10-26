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
Glossary: `GLOSSARY.md`. Decisions: `docs/adr/0001`–`0007`.

Children: #T1–#T15 below. v1 issues are labelled `v1-retired`.

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

## T14: Settings, preferences and integrations

Spec §10, §12.

- Settings screen; preferences file (theme, keymap remapping, intervals,
  retention, unlock timeout); command log `@`; `e` editor and `G` lazygit handoff
  with terminal suspend/restore; `y` copy path; `+`/`_` screen modes.

Blocked by T3.

## T15: Distribution and multi-machine acceptance

Spec §2, §13.

- Release workflow for macOS arm64, Linux x86-64/arm64 bundles with licences and
  checksums; `install.sh`; `self-update` and rollback; `.deb`, AUR `dotwarden-bin`,
  Homebrew tap.
- Multi-machine acceptance on macOS: isolated HOME/state directories registered as
  Ubuntu, CachyOS and macOS machines on a shared bare remote, covering assignments, variants, values, vault, incoming, conflicts,
  retire, archive restore.

Blocked by all others.
