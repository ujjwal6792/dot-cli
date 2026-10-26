# Dotwarden v2 specification

Dotwarden (`dotwarden`, alias `dw`) keeps one person's dotfiles, secrets and
variables in sync across their machines. It is a lazygit-style terminal UI and an
agent-friendly CLI on top of a bundled chezmoi and a bundled git. Vocabulary is
defined in [GLOSSARY.md](../GLOSSARY.md); decisions are recorded in [ADRs](adr/).

This replaces the `dot` v1 application entirely (ADR 0001). Nothing from v1 is
migrated: users start a fresh repository and agents re-track files from HOME.

## 1. Goals and non-goals

Goals:

- One tool replaces the nvim + lazygit + file-manager + `.gitignore` juggling of a
  bare-repo-in-HOME setup, for macOS, Arch/CachyOS and Ubuntu machines.
- Everything in HOME is browsable; tracking is a keypress; every change is visible
  before it is committed, pushed, discarded or applied.
- Per-machine differences (assignments, variants, values) are first-class and
  visible, and are enforced by chezmoi so behaviour is standard and inspectable.
- Secrets live encrypted in the repository and are restored with a passphrase.
  Agents can operate dotwarden without ever reading secret values from it.
- Install with one `curl` command; no system chezmoi, git or delta required.

Non-goals: Windows; importing existing dotfile setups through the UI (agents do
that with ordinary CLI commands); background sync daemons; machine status
reporting; raw chezmoi passthrough; hosted services.

## 2. Distribution and runtime (ADR 0002)

- Release bundle per target: macOS arm64, Linux x86-64, Linux arm64. Contents:
  `bin/dotwarden`, `bin/dw` (symlink), `libexec/chezmoi` (v2.73.3 at time of
  writing), `libexec/git/` (portable git from desktop/dugite-native, v2.56.0-2 at
  time of writing), `libexec/delta` (0.20.1), licenses (git is GPLv2: include its
  license and a source offer/link), and a SHA256 manifest. Pins are recorded in one
  file in the repo and verified at build time.
- `curl -fsSL <url>/install.sh | sh`: detects OS/arch, downloads and verifies the
  bundle, installs to `~/.local/share/dotwarden/releases/<version>/`, points
  `~/.local/share/dotwarden/current` at it, and links `dotwarden` and `dw` into
  `~/.local/bin`. No sudo. Prints PATH advice if needed.
- `dw self-update` verifies checksums and switches `current`; keeps the previous
  version for `dw self-update --rollback`. Disabled (with a message) when installed
  by a package manager.
- Packages: `.deb` (Ubuntu), AUR `dotwarden-bin`, Homebrew tap. Packages install
  the same bundle layout under the package manager's prefix.
- Linux git from dugite-native is built on Ubuntu (glibc). Verify it runs on Arch
  and CachyOS; document the minimum glibc.

## 3. Storage layout

Machine-local (never committed):

| Path | Contents |
|---|---|
| `$XDG_DATA_HOME/dotwarden/repo/` | The git repository (see below) |
| `$XDG_DATA_HOME/dotwarden/chezmoi.toml` | Generated chezmoi config; regenerated, never hand-edited |
| `$XDG_STATE_HOME/dotwarden/chezmoistate.boltdb` | chezmoi persistent state |
| `$XDG_STATE_HOME/dotwarden/snapshots/` | Snapshots (0700) |
| `$XDG_STATE_HOME/dotwarden/pending.json` | Pending list, shared by CLI and TUI |
| `$XDG_STATE_HOME/dotwarden/activity.jsonl` | Local activity log (no values) |
| `$XDG_STATE_HOME/dotwarden/machine.toml` | This machine's registry name and remote-privacy confirmations |
| `~/.config/dotwarden/config.toml` | Preferences: an ordinary, trackable dotfile |

Every chezmoi invocation passes explicit `--config`, `--source`,
`--persistent-state`, `--cache` and `--destination`, and an environment with
`CHEZMOI_*` variables removed, so a user's own chezmoi installation is neither read
nor modified. Every git invocation uses the bundled git with
`GIT_CONFIG_NOSYSTEM=1`; the user's global config is read for `user.name`,
`user.email` and signing settings, and the system `ssh` with the user's keys and
agent handles SSH remotes. Settings can override identity per repository.

Repository layout:

```
repo/
  .chezmoiroot                 contains "home"
  dotwarden.toml               format version, default branch
  hide                         hide rules (gitignore syntax)
  home/                        chezmoi source state root
    .chezmoiignore             fixed template written by dotwarden (assignment logic)
    .chezmoidata/dotwarden.toml  registry, assignments, variables, tracked folders, variant index
    .chezmoitemplates/dotwarden/ helper templates (variant selection, references)
    .dotwarden/variants/…      variant contents (leading dot: ignored as targets)
    .dotwarden/vault/          recipient.txt, identity.age, recovery.age,
                               secrets/<NAME>.age, files/<id>.age
    dot_zshrc …                ordinary chezmoi source entries
```

#### Verified mechanics

Verified in ticket 1 against the pinned chezmoi v2.73.3 and proven by
`tests/spike.rs` (every item below has a test of the same topic) and
`tests/isolation.rs`.

1. **`.chezmoiroot`.** Dotwarden passes the *repository root* as `--source`;
   chezmoi reads `.chezmoiroot` (`home`) and uses `repo/home` as the source state
   root (`chezmoi source-path` prints `repo/home`). Files at the repository root
   (`dotwarden.toml`, `hide`, `README.md`, even `dot_x`) are not source state and
   never become targets.
2. **Leading-dot entries under `home/` are not targets.** `.dotwarden/…`
   (variants, vault), `.chezmoitemplates/`, `.chezmoidata/` and `.chezmoiignore`
   are never listed by `chezmoi managed` and never written to HOME by `apply`.
3. **`include` paths are relative to the `.chezmoiroot` directory (`home/`)**, not
   the repository root: `{{ include ".dotwarden/variants/x" }}` works;
   `{{ include "home/.dotwarden/variants/x" }}` fails with "no such file".
4. **Machine data.** The generated chezmoi config exposes only the registry name:
   `[data.dotwardenMachine] name = "<registry name>"`. Everything else (platform,
   distribution, families) is looked up in the registry
   (`.chezmoidata/dotwarden.toml`, key `dotwarden.machines`) by the `targets`
   helper, so every plain-chezmoi evaluation agrees with the registry. An unknown
   name fails with `machine "<name>" is not in the machine registry`; running
   chezmoi without the generated config fails with a message naming
   `dotwardenMachine`. chezmoi runs templates with `missingkey=error`, so
   templates must read optional keys with `dig`/`get`/`hasKey`.
5. **Assignments** live in `.chezmoidata/dotwarden.toml`, keyed by *target path*
   (the HOME-relative path, e.g. `.config/app/.nested`, not the `dot_` source
   name):
   ```toml
   [dotwarden.assignments.".zshrc"]
   include = ["platform:linux", "machine:mac"]   # optional; union; absent = everywhere
   exclude = ["distribution:cachyos"]            # optional; always wins
   ```
   The fixed `.chezmoiignore` (below) evaluates them for the current machine.
   Verified: include union across all four target kinds, excludes win over
   includes, no includes means every machine, and plain `chezmoi apply`
   enforces the result. Assigning edits only data; `.chezmoiignore` is
   byte-identical to the template shipped with dotwarden.
   *Caveats found:* (a) `.chezmoiignore` lines are globs, so paths containing
   `* ? [ ] { } \` must be escaped, which the template does
   (`regexReplaceAll`); without that, an assignment for `.a[1]b` ignored
   `.a1b` instead. (b) Ignoring every file of a directory leaves the empty
   directory itself managed; to keep a directory off a machine, assign the
   directory path.
6. **Variant selection.** Variants are data, keyed by target path:
   ```toml
   [[dotwarden.variants.".gitconfig"]]
   target = "family:arch-like"
   file = "gitconfig/arch"          # under home/.dotwarden/variants/
   ```
   The `dotwarden/variant` helper prints the `file` of the most specific
   matching variant (machine > distribution > family > platform) or nothing when
   the default content applies. Two matches at the same specificity (for
   example a machine in two families, each with a variant) fail with
   `ambiguous variants for <path> at the same specificity: …`. A tracked file
   using variants is a template such as `home/dot_gitconfig.tmpl`:
   ```
   {{- $v := includeTemplate "dotwarden/variant" (dict "data" . "path" ".gitconfig") | trim -}}
   {{- if $v }}{{ include (printf ".dotwarden/variants/%s" $v) }}{{ else }}…default content…{{ end -}}
   ```
   An ambiguity is an error for that file only: `chezmoi apply` stops at the
   first error, but `chezmoi apply --keep-going` writes every other file and
   reports the failing one, so dotwarden must apply with `--keep-going` to
   "block apply of that file only".
7. **Isolation.** With explicit `--config/--source/--persistent-state/--cache/
   --destination` and `CHEZMOI_*` stripped, a pre-existing `~/.config/chezmoi`
   and `~/.local/share/chezmoi` are neither read (their data and source never
   appear) nor modified (byte, size and mtime identical after init, status,
   doctor, managed, status, apply, data); on a fresh HOME neither directory is
   created.

Exact template code (written once into every repository; the files in
`assets/home/` are the source of truth and are embedded in the binary):

`home/.chezmoiignore`
```
{{- /* Written once by dotwarden; never regenerated or hand-edited (ADR 0005).
       Evaluates the assignments in .chezmoidata/dotwarden.toml for this machine:
       a file applies when (no includes OR any include matches) AND no exclude
       matches. Everything printed here is a target path that is NOT applied;
       glob characters in paths are escaped so they match literally. */ -}}
{{- $targets := includeTemplate "dotwarden/targets" . | fromJson -}}
{{- range $path, $a := (dig "dotwarden" "assignments" (dict) .) -}}
{{- $include := get $a "include" | default (list) -}}
{{- $exclude := get $a "exclude" | default (list) -}}
{{- $applies := not $include -}}
{{- range $include }}{{ if has . $targets }}{{ $applies = true }}{{ end }}{{ end -}}
{{- range $exclude }}{{ if has . $targets }}{{ $applies = false }}{{ end }}{{ end -}}
{{- if not $applies }}
{{ regexReplaceAll `([\\*?\[\]{}])` $path `\$1` }}
{{- end }}
{{ end -}}
```

`home/.chezmoitemplates/dotwarden/targets`
```
{{- /* JSON list of the targets the current machine matches, e.g.
       ["platform:linux","distribution:cachyos","family:arch-like","machine:laptop"].
       The machine's registry name comes from the generated chezmoi config
       (data.dotwardenMachine.name); everything else from the registry. */ -}}
{{- if not (hasKey . "dotwardenMachine") -}}
{{- fail "dotwarden: chezmoi data 'dotwardenMachine' is missing; run chezmoi through dotwarden" -}}
{{- end -}}
{{- $name := .dotwardenMachine.name -}}
{{- $machine := dict -}}
{{- range (dig "dotwarden" "machines" (list) .) }}{{ if eq .name $name }}{{ $machine = . }}{{ end }}{{ end -}}
{{- if not $machine -}}
{{- fail (printf "dotwarden: machine %q is not in the machine registry" $name) -}}
{{- end -}}
{{- $targets := list (printf "platform:%s" $machine.platform) (printf "distribution:%s" $machine.distribution) (printf "machine:%s" $machine.name) -}}
{{- range $machine.families }}{{ $targets = append $targets (printf "family:%s" .) }}{{ end -}}
{{- toJson $targets -}}
```

`home/.chezmoitemplates/dotwarden/variant`
```
{{- /* Variant selection. Call with (dict "data" . "path" "<target path>").
       Prints the `file` of the most specific matching variant of that target
       (machine > distribution > family > platform), or nothing when no variant
       matches and the default content applies. Two matches at the same
       specificity are an error. */ -}}
{{- $targets := includeTemplate "dotwarden/targets" .data | fromJson -}}
{{- $best := 0 -}}
{{- $file := "" -}}
{{- $ties := list -}}
{{- range (dig "dotwarden" "variants" .path (list) .data) -}}
{{- $parts := splitList ":" .target -}}
{{- $level := 0 -}}
{{- if eq (index $parts 0) "machine" }}{{ $level = 4 }}
{{- else if eq (index $parts 0) "distribution" }}{{ $level = 3 }}
{{- else if eq (index $parts 0) "family" }}{{ $level = 2 }}
{{- else if eq (index $parts 0) "platform" }}{{ $level = 1 }}
{{- else }}{{ fail (printf "dotwarden: unknown target kind in %q" .target) }}{{ end -}}
{{- if has .target $targets -}}
{{- if gt $level $best }}{{ $best = $level }}{{ $file = .file }}{{ $ties = list .target }}
{{- else if eq $level $best }}{{ $ties = append $ties .target }}{{ end -}}
{{- end -}}
{{- end -}}
{{- if gt (len $ties) 1 -}}
{{- fail (printf "dotwarden: ambiguous variants for %s at the same specificity: %s" .path (join ", " $ties)) -}}
{{- end -}}
{{- $file -}}
```

## 4. Machines and targeting (ADR 0005)

- The **machine registry** lists machines: `name`, `platform` (darwin|linux),
  `distribution` (`/etc/os-release` `ID`; `macos` on darwin), `families` (from
  `ID_LIKE`, e.g. `arch` → target `arch-like`), and `retired`.
- Each machine stores its registry name locally; the generated chezmoi config
  exposes it as data. Names are user-chosen, `[a-z0-9-]`, independent of hostname.
- **Targets**: `platform:<p>`, `distribution:<d>`, `family:<f>`, `machine:<name>`.
- **Assignment** = includes + excludes. Applies if (no includes OR any include
  matches) AND no exclude matches. New tracked files have no includes (everywhere);
  the pending item states "applies to: all machines".
- Assignments live in `.chezmoidata/dotwarden.toml`; the fixed `.chezmoiignore`
  template evaluates them for the current machine, so plain chezmoi enforces them.
  Assigning never rewrites `.chezmoiignore`.
- **Specificity** (variants and values): machine > distribution > family >
  platform > default. Two matches at the same level are an error shown in the UI
  and block apply of that file only.
- **Rename** a machine: updates every assignment, variant and value override in one
  reviewed change. **Retire**: removes it from all targets via a review listing
  every affected file and value; the registry keeps it as retired.
- **Preview as machine**: render the managed set and diffs for another registry
  machine using an isolated chezmoi state and overridden machine data. Secrets are
  masked in previews.

## 5. Change model (ADR 0003)

HOME plays the role of git's working tree. The repository working tree is kept
identical to the git index at all times; dotwarden never leaves unstaged edits in
the repo.

| State | Meaning | Shown in |
|---|---|---|
| Local | Tracked file in HOME differs from the committed (rendered) version and the committed version has not changed since this machine last applied it | Changes → Local |
| New | Untracked file inside a tracked folder, not hidden | Changes → Local (marked new) |
| Staged | Content written into the repo and `git add`ed, not yet committed | Changes → Staged |
| Incoming | Committed version changed (pulled) and HOME still equals what was last applied | Changes → Incoming |
| Conflict | Both HOME and the committed version changed since last apply | Changes → Conflicts |

Classification uses chezmoi status plus chezmoi's last-written state (status
column 1: last written vs actual; column 2: actual vs target). Dotwarden must
verify the mapping in tests for every state, including after stage+commit (a file
must show no change once its HOME content is committed).

Actions:

- **Stage** (`Space`): write HOME content into source (plain files via chezmoi
  `add`/`re-add`; files with references via dotwarden's reverse mapping, §7) and
  `git add`. Hunk/line staging writes only the selected hunks. **Unstage** restores
  the source entry from HEAD.
- **Commit** (`c`), **amend** (`A`), multi-line messages. Commits by agents get an
  `Agent: <name>` trailer.
- **Discard** (`d`): snapshot HOME, then write the committed version to HOME.
- **Apply** incoming changes: per file or all; snapshot before overwriting.
- **Conflicts**: 3-way view (last applied / HOME / incoming) with per-hunk
  local/incoming choice; `e` opens `$EDITOR` (nvim diff mode when the editor is
  nvim) on a merge file. Result is staged.
- **Untrack**: remove from the repo; HOME copies on every machine are left in
  place; other machines show "no longer tracked" once.
- **Tracked folder**: tracking a folder tracks its current files and records the
  folder; new files inside appear as New. Nothing is added automatically.
- Symlinks are tracked as symlinks.
- **Hide rules** (`hide`, shared, gitignore syntax) remove noise from the browser
  and refuse tracking. Sensible defaults (caches, histories, `node_modules`,
  `Library` except explicitly un-hidden paths, …) are written at repo creation.

## 6. Git

Covered in CLI and TUI: fetch, pull (fast-forward by default; merge or rebase on
explicit choice with conflict resolution in dotwarden), push (to one or all
remotes), branches (list/create/switch/delete), remotes (add/remove/list), log with
per-commit diff and file history, revert a commit, stash/pop, point-in-time
restore. All machines share one default branch; the UI warns when a machine is on
another branch. Background fetch runs on TUI start and at a preference interval;
it never pulls or applies.

**Public remotes are refused** (ADR 0006). For github.com, gitlab.com,
codeberg.org and bitbucket.org, an anonymous `git ls-remote` over HTTPS with
credential helpers and prompts disabled succeeding means public. Checked when a
remote is added and before every push; push is blocked if any target remote is
public; offline skips the check with a warning. Unverifiable hosts require a
one-time human confirmation that the remote is private, stored per remote in
`machine.toml`; agents cannot give it.

## 7. Values and vault (ADR 0004)

- **Values** table: entries are `secret` or `variable`, each with a default and
  per-target overrides (specificity as §4).
- Variables live in plain `.chezmoidata/dotwarden.toml`. Secrets live in
  `vault/secrets/<NAME>.age` (default + overrides encrypted together); names are
  visible.
- **Encrypted files** (e.g. `~/.ssh/id_ed25519`) are stored whole in
  `vault/files/<id>.age` and rendered by a helper.
- Crypto: an age X25519 identity is generated at `dw vault init`. `recipient.txt`
  holds the public key, so adding or encrypting never needs unlock.
  `identity.age` is the identity encrypted with the passphrase (age scrypt);
  `recovery.age` is the identity encrypted with a generated recovery code shown
  once. Changing the passphrase rewrites only `identity.age`.
- **Unlock session**: the passphrase is read only from an interactive terminal
  (never args, env, or piped stdin). The decrypted identity is held in the OS
  keychain (macOS Keychain, Linux Secret Service; fallback a 0600 file under
  `$XDG_RUNTIME_DIR`) with an expiry from preferences (default 15 min). `dw lock`
  ends it.
- **References**: a tracked file containing references is stored as a template
  whose references resolve, for the receiving machine, to variables (from data) or
  secrets (through a dotwarden helper invoked by chezmoi with a per-run token that
  only the parent dotwarden process knows). Rendering secrets requires an unlock
  session.
- **Detection on track/stage**: values matching an existing secret, or flagged by
  the secret scanner, produce a pending item proposing: create/update secret and
  replace the value with a reference. When HOME holds a different value at a
  reference position, the change is shown as "secret NAME changed"; staging it
  updates the vault (requires unlock) and keeps the reference.
- **Masking**: no CLI output, diff, log, pending item, activity record or preview
  ever contains a secret value. When locked, lines of HOME that correspond to
  reference positions are hidden in diffs.
- The TUI Values screen (`V`) shows names, kinds, targets; secret values only
  while unlocked and only in the TUI.
- Honest boundary: a value applied to HOME is plaintext on disk and readable by any
  process running as the user. Dotwarden protects the repository, its own outputs
  and logs; it does not sandbox HOME.

## 8. Pending list and agents (ADR 0007)

- Track, untrack, assign, encrypt, variant and value-metadata changes are added to
  the **pending list**, persisted in `pending.json` and shared by CLI and TUI. Each
  item shows exactly what will change (files, rules, data). One review applies
  the selected items; items can be dropped. Stage/unstage/commit are immediate
  (lazygit behaviour); `z` undoes.
- Agents (CLI without an interactive human approval) may: inspect anything except
  secret values; track, untrack, stage, unstage, assign, commit; add secret
  placeholders and wire references; add pending items.
- Requires human approval: push, discard, apply over local changes, history
  rewrite (amend of pushed commits, rebase), branch delete, machine retire, remote
  add/remove, privacy confirmation. From an agent these become pending items
  marked "requested by <agent>"; the CLI exits with a distinct code. Humans approve
  in the TUI or with `dw approve <id>` in an interactive terminal.
- This is a guard against agent mistakes, not a security boundary against a
  hostile process running as the same user.
- Agent identity: `DOTWARDEN_AGENT=<name>` (and known agent environments) label the
  activity log and commit trailers.

## 9. CLI contract

- Every command supports `--json`; human output by default. Every argument and
  flag has help text. `dw completions <shell>` and a man page are generated.
- Paths are HOME paths (`~/x`, absolute, or relative to the current directory);
  repository paths are never required.
- Exit codes: 0 ok, 1 error, 2 usage, 3 needs approval (added to pending),
  4 vault locked, 5 conflict, 6 refused public remote, 7 stale (state changed).
- Mutations take a short lock and wait up to 10 s rather than failing instantly.
- Commands (indicative): `init`, `clone`, `status`, `ls`, `diff`, `track`,
  `untrack`, `stage`, `unstage`, `commit`, `discard`, `apply`, `pull`, `push`,
  `fetch`, `branch`, `remote`, `log`, `show`, `stash`, `revert`, `restore --at`,
  `assign`, `variant`, `machine (list|add|rename|retire|preview)`,
  `values (list|set-variable|add-secret|wire)`, `vault (init|unlock|lock|passwd|recover)`,
  `pending (list|drop|approve)`, `snapshot (list|restore)`, `archive (export|import)`,
  `hide`, `settings`, `doctor`, `self-update`.
- Every mutation supports `--dry-run`, returning exactly what would change.

## 10. TUI

Layout: left column 1 Status (machine, branch, ahead/behind, vault state, pending
count), 2 Files (HOME browser, yazi-like, hidden items toggled with `.`), 3 Changes
(Local / Staged / Incoming / Conflicts), 4 Git (Commits / Branches / Remotes /
Stash), 5 Machines (registry, assignments, preview-as). Right: main view (diff via
delta, preview, assignment editor). Bottom-right: Pending list. `V` opens Values
full screen. Narrow terminals show the focused panel only.

Default keys (lazygit-compatible, remappable in preferences): `Space` stage/track,
`a` stage all, `v` range select, `c` commit, `A` amend, `P` push, `p` pull,
`f` fetch, `d` discard, `e` `$EDITOR`, `o` open, `i` hide, `/` filter, `[` `]`
tabs, `h`/`l` panels, `1`–`5` panels, `x` menu, `z` undo, `?` help, `@` command log,
`+`/`_` screen mode, `y` copy path, `m` assign, `E` encrypt, `.` show hidden,
`V` values, `G` open lazygit on the repository (suspends dotwarden), `Enter` drill
in / staging view, `Esc` back, `q` quit.

Requirements: the event loop never blocks on I/O; background jobs (preview,
refresh, fetch) never block user actions and are cancelled when superseded; an
editor or lazygit handoff suspends and restores the terminal; theme colours come
from preferences.

## 11. Recovery

- Snapshots before any overwrite of HOME; retained 30 days with at least the 5 most
  recent per file (preferences).
- `z` undoes the last local action: stage/unstage, track/untrack, assign, discard
  (from snapshot), last unpushed commit. Never a push.
- Point-in-time restore of one file or all files from any commit (through Incoming
  so it is reviewed).
- Archive: git bundle of the repository + registry, encrypted with the vault
  passphrase; `archive import` restores a machine offline.

## 12. Settings

A Settings screen and `dw settings` cover: git identity/signing per repository,
remotes, default branch, fetch interval, diff/merge tool, chezmoi options exposed
by dotwarden (e.g. umask, editor), unlock timeout, snapshot retention, theme and
keymap. Changes to repository-wide settings go through the pending list.

## 13. Verification

- Unit and integration tests run with the bundled tools and isolated HOME/state,
  on macOS only. Implementations do not run Linux, Docker or emulated tests; the
  owner verifies Linux behaviour on real Linux machines after development, and
  Linux fixes follow from that. Linux-specific code must still compile and carry
  unit tests that do not need a Linux host.
- A multi-machine scenario on macOS: several isolated HOME/state directories
  registered as different machines (with overridden platform/distribution data)
  share a local bare remote and exercise assignments, variants, values, vault
  unlock, incoming, conflicts and retire.
- TUI tests use Ratatui's test backend with real key events; a PTY smoke test
  covers start, handoff to editor, resize and exit.
- No test may print a secret value; a test asserts outputs and logs contain none.
