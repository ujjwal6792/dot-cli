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
    .chezmoidata/dotwarden.toml  assignments, tracked folders, variant index
    .chezmoidata/dotwarden-values/<NAME>.toml  one variable (default + overrides)
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
   distribution, families) is looked up in the registry by the `targets`
   helper. The registry is one file per machine,
   `.chezmoidata/dotwarden-machines/<name>.toml` holding
   `[dotwarden.machines.<name>]`, so registrations made on different computers
   never conflict textually. Verified with the pinned chezmoi: `.chezmoidata`
   files, also in subdirectories, deep-merge maps, so the files together form the
   map `dotwarden.machines` (keyed by name; the `targets` helper looks the name
   up with `dig`), so every plain-chezmoi evaluation agrees with the registry. An unknown
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
   *Caveats found:* (a) `.chezmoiignore` lines are patterns, not paths. The
   template escapes: glob characters `* ? [ ] { } \` with a backslash (without
   that, an assignment for `.a[1]b` ignored `.a1b`); a leading `#` (comment) and
   a leading `!` (negation) with a backslash; and every space or other Unicode
   separator as `[ ]`, because chezmoi trims whitespace around each line.
   Verified for `#x`, `!x`, `a b ` (trailing space), ` lead`, `a<NBSP>`, `[1]`,
   `*`, `?`, `{a,b}` and a backslash. Paths that cannot be one line (newline,
   carriage return, tab or any control character), absolute paths, `..`
   components and empty components are refused with a usage error when the
   assignment is made (`assign::set_assignment`), before anything is written.
   An include or exclude whose target kind is unknown never matches (`dw doctor`
   warns). (b) Ignoring every file of a directory leaves the empty
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
   the default content applies. Selection itself lives in the shared
   `dotwarden/select` helper (reused by values): only the best level matters, so
   a more specific match beats any tie at a lower level; two matches at the best
   specificity (for example a machine in two families, each with a variant)
   fail with `ambiguous variants for <path> at the same specificity: …`. An
   entry with an unknown target kind never matches and does not affect other
   entries or files. Retired machines have no targets: chezmoi on a retired
   machine fails with `machine "<name>" is retired and has no targets`. A tracked file
   using variants is a template such as `home/dot_gitconfig.tmpl`; the default
   content is a variant file too. A variant file is included raw (`{{` is never
   evaluated) unless it contains references (§7): the marker
   `{{ includeTemplate "dotwarden/value"` makes the template render it:
   ```
   {{- $v := includeTemplate "dotwarden/variant" (dict "data" . "path" ".gitconfig") | trim -}}
   {{- $f := printf ".dotwarden/variants/%s" (default ".gitconfig/default" $v) -}}
   {{- $t := include $f -}}
   {{- if contains "{{ includeTemplate \"dotwarden/value\"" $t }}{{ includeTemplate $f $ }}{{ else }}{{ $t }}{{ end -}}
   ```
   Verified in ticket 7: `includeTemplate` accepts any path under the source
   root and runs the file as a template with the given data, but the `template`
   action is not defined inside it, so references use `includeTemplate`.
   Variant files are `home/.dotwarden/variants/<target path>/<target with : as ->`
   (`.gitconfig/machine-mac`) and `<target path>/default`; a name already taken
   (after a machine rename) gets `-2`, `-3`, … appended. A source file's other
   attributes (`executable_`, `private_`) stay in the template's name.
   Dotwarden mirrors the selection in Rust (`variants::pick`) because chezmoi
   fails `dump`, `status` and `diff` as a whole on one ambiguity, even with
   `--keep-going`; for status, classification and staging it renders everything
   except the clashed files by naming the other managed files explicitly. An
   ambiguity is an error for that file only: `chezmoi apply` stops at the
   first error, but `chezmoi apply --keep-going` writes every other file and
   reports the failing one, so dotwarden applies with `--keep-going` and never
   names a clashed file.
7. **Git isolation.** Dotwarden's git (and the git chezmoi runs, which gets the
   bundled git first on `PATH`) runs with `GIT_CONFIG_NOSYSTEM=1` and
   `GIT_CONFIG_GLOBAL=/dev/null`; only `user.name`, `user.email`,
   `user.signingkey`, `commit.gpgsign`, `gpg.format`, `gpg.program` and
   `gpg.{openpgp,x509,ssh}.program` are read from the user's global config
   (honouring `GIT_CONFIG_GLOBAL` if the user set it) and passed on as
   `GIT_CONFIG_COUNT/KEY/VALUE`. Hooks, `core.fsmonitor`, URL rewrites, aliases
   and every other global setting are not applied. Other `GIT_*` variables are
   dropped; `GIT_SSH_COMMAND`, `GIT_ASKPASS`, `GIT_TERMINAL_PROMPT` and
   `SSH_AUTH_SOCK` pass through so SSH remotes work.
8. **Isolation.** With explicit `--config/--source/--persistent-state/--cache/
   --destination` and `CHEZMOI_*` stripped, a pre-existing `~/.config/chezmoi`
   and `~/.local/share/chezmoi` are neither read (their data and source never
   appear) nor modified (byte, size and mtime identical after init, status,
   doctor, managed, status, apply, data); on a fresh HOME neither directory is
   created.

9. **Track and untrack (ticket 2).** Verified against chezmoi v2.73.3 by
   `tests/track.rs`:
   - `chezmoi add --dry-run --verbose <files>` prints `diff --git a/<S> b/<S>`
     for every repository path it would create (directories too, with mode
     `40xxx`) without touching the repository or the persistent state; split at
     the middle of the line, `<S>` is unambiguous even with spaces, quotes and
     non-ASCII. This is the exact preview of a track item.
   - `chezmoi add` records last-written state itself, so `chezmoi status` is
     empty right after tracking; it does not stage anything (dotwarden runs
     `git add`).
   - chezmoi strips each source-name attribute once, in a fixed order (a folder
     literally named `exact_x` is `exact_exact_x`; a file named `x.tmpl` is
     `x.tmpl.literal`; a file named `dot_x` is `literal_dot_x`).
   - `chezmoi managed` omits files an assignment keeps off this machine, so
     "is it tracked" is answered by decoding the source tree (`src/source.rs`,
     cross-checked against `chezmoi managed`); tracking such a file again would
     overwrite the shared version.
   - Tracked folders are recorded as `dotwarden.tracked_folders = ["<target path>", …]`
     in `.chezmoidata/dotwarden.toml`.

10. **Variables (ticket 7).** `home/.chezmoidata/dotwarden-values/<NAME>.toml`
    holds `[dotwarden.values.<NAME>]` (`kind = "variable"`, `default`, and
    `[[…overrides]]` with `target`/`value`); the files deep-merge into
    `dotwarden.values`. The `dotwarden/value` helper (below) resolves a name
    through `dotwarden/select`. See §7.

Exact template code (written once into every repository; the files in
`assets/home/` are the source of truth and are embedded in the binary):

`home/.chezmoiignore`
```
{{- /* Written once by dotwarden; never regenerated or hand-edited (ADR 0005).
       Evaluates the assignments in .chezmoidata/dotwarden.toml for this machine:
       a file applies when (no includes OR any include matches) AND no exclude
       matches. Everything printed here is a target path that is NOT applied.
       Each line is a chezmoi ignore pattern, so a path is escaped to match
       literally: glob characters get a backslash; a leading # (comment) or !
       (negation) is backslash-escaped; spaces and other Unicode separators
       become [ ] because chezmoi trims whitespace around lines. Paths that
       cannot be represented (control characters, newlines) are refused when
       the assignment is made. An include or exclude with an unknown target
       kind never matches. */ -}}
{{- $targets := includeTemplate "dotwarden/targets" . | fromJson -}}
{{- range $path, $a := (dig "dotwarden" "assignments" (dict) .) -}}
{{- $include := get $a "include" | default (list) -}}
{{- $exclude := get $a "exclude" | default (list) -}}
{{- $applies := not $include -}}
{{- range $include }}{{ if has . $targets }}{{ $applies = true }}{{ end }}{{ end -}}
{{- range $exclude }}{{ if has . $targets }}{{ $applies = false }}{{ end }}{{ end -}}
{{- if not $applies }}
{{- $p := regexReplaceAll `([\\*?\[\]{}])` $path `\$1` -}}
{{- $p = regexReplaceAll `^([#!])` $p `\$1` -}}
{{- $p = regexReplaceAll `(\pZ)` $p `[$1]` }}
{{ $p }}
{{- end }}
{{ end -}}
```

`home/.chezmoitemplates/dotwarden/targets`
```
{{- /* JSON list of the targets the current machine matches, e.g.
       ["platform:linux","distribution:cachyos","family:arch-like","machine:laptop"].
       The machine's registry name comes from the generated chezmoi config
       (data.dotwardenMachine.name); everything else from the registry.
       Retired machines are not in any target: a retired machine is refused. */ -}}
{{- if not (hasKey . "dotwardenMachine") -}}
{{- fail "dotwarden: chezmoi data 'dotwardenMachine' is missing; run chezmoi through dotwarden" -}}
{{- end -}}
{{- $name := .dotwardenMachine.name -}}
{{- $machine := dig "dotwarden" "machines" $name (dict) . -}}
{{- if not $machine -}}
{{- fail (printf "dotwarden: machine %q is not in the machine registry" $name) -}}
{{- end -}}
{{- if get $machine "retired" | default false -}}
{{- fail (printf "dotwarden: machine %q is retired and has no targets" $name) -}}
{{- end -}}
{{- $targets := list (printf "platform:%s" $machine.platform) (printf "distribution:%s" $machine.distribution) (printf "machine:%s" $machine.name) -}}
{{- range $machine.families }}{{ $targets = append $targets (printf "family:%s" .) }}{{ end -}}
{{- toJson $targets -}}
```

`home/.chezmoitemplates/dotwarden/select`
```
{{- /* Specificity selection shared by variants and values: machine >
       distribution > family > platform. Call with
       (dict "targets" <this machine's targets> "entries" <list of dicts with a
       .target> "what" "<label for errors>"). Prints the winning entry as JSON,
       or nothing when no entry matches. Two matches at the best level are an
       error; ties at a lower level do not matter when a more specific entry
       matches. An entry whose target kind is unknown never matches. */ -}}
{{- $levels := dict "platform" 1 "family" 2 "distribution" 3 "machine" 4 -}}
{{- $best := 0 -}}
{{- $winner := dict -}}
{{- $ties := list -}}
{{- range .entries -}}
{{- $level := get $levels (index (splitList ":" .target) 0) | default 0 -}}
{{- if and (gt $level 0) (has .target $.targets) -}}
{{- if gt $level $best }}{{ $best = $level }}{{ $winner = . }}{{ $ties = list .target }}
{{- else if eq $level $best }}{{ $ties = append $ties .target }}{{ end -}}
{{- end -}}
{{- end -}}
{{- if gt (len $ties) 1 -}}
{{- fail (printf "dotwarden: ambiguous %s at the same specificity: %s" .what (join ", " $ties)) -}}
{{- end -}}
{{- if $winner }}{{ toJson $winner }}{{ end -}}
```

`home/.chezmoitemplates/dotwarden/variant`
```
{{- /* Variant selection. Call with (dict "data" . "path" "<target path>").
       Prints the `file` of the most specific matching variant of that target
       (see dotwarden/select), or nothing when no variant matches and the
       default content applies. */ -}}
{{- $targets := includeTemplate "dotwarden/targets" .data | fromJson -}}
{{- $entries := dig "dotwarden" "variants" .path (list) .data -}}
{{- $winner := includeTemplate "dotwarden/select" (dict "targets" $targets "entries" $entries "what" (printf "variants for %s" .path)) -}}
{{- if $winner }}{{ (fromJson $winner).file }}{{ end -}}
```

`home/.chezmoitemplates/dotwarden/value`
```
{{- $v := dig "dotwarden" "values" .name (dict) .data -}}
{{- if not $v -}}
{{- fail (printf "dotwarden: variable %q does not exist" .name) -}}
{{- end -}}
{{- $targets := includeTemplate "dotwarden/targets" .data | fromJson -}}
{{- $w := includeTemplate "dotwarden/select" (dict "targets" $targets "entries" (get $v "overrides" | default (list)) "what" (printf "overrides of %s" .name)) -}}
{{- if $w }}{{ (fromJson $w).value }}{{ else }}{{ get $v "default" }}{{ end -}}
```
(the shipped file also starts with a comment.)

## 4. Machines and targeting (ADR 0005)

- The **machine registry** (one data file per machine, §3 item 4) lists machines: `name`, `platform` (darwin|linux),
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
  platform > default. Two matches at the same level are a clash, shown in
  `dw status`, `dw doctor`, `dw variant list` and the TUI (`◆N!`, Status panel);
  it blocks that file only (not classified, not staged, not applied) while all
  other files stay usable.
- **Variants** (`dw variant add|list|remove`, TUI `t`; pending items, any agent
  may add them). `add PATH --target T` makes a tracked regular file a variant
  file: its stored content becomes the `default` variant and the *current HOME
  content* becomes the variant for `T` (checked like assignment targets;
  one variant per target). `remove` deletes one; removing the last turns the
  file back into a plain file with the default content. The preview lists every
  repository file added or removed and, per registry machine, which variant it
  receives before and after. Untracking removes all variants; renaming a machine
  rewrites variant targets (files keep their names); retiring one leaves its
  variant in place but unused (noted in the preview; `variant remove` still
  works). `dw machine preview` marks the variant each file would get. The TUI
  shows `◆N` (N stored versions, default included) in Files and Changes, lists
  the versions with the one this machine receives in the main view, and `t`
  opens the target picker for a new variant of the selected file.
- **Assigning** (`dw assign`, TUI `m`) is one pending item per request holding the
  final lists of each path; any agent may add it. A path must be tracked or a
  folder holding tracked files. A folder's assignment covers everything below
  it; a file applies only when its own assignment and those of all folders above
  it allow it. Targets are checked against the registry: unknown platform or
  machine (and retired machines) are refused; a distribution or family no
  registered machine has is accepted with a warning. The preview lists which
  tracked files start or stop applying on which registry machines.
- **Add** (`dw machine add`) pre-registers another machine through a pending item.
- **Rename** a machine: one pending item that renames the registry file and
  rewrites every `machine:<old>` string or key in the data file (assignments,
  variants, value overrides). Renaming this machine also rewrites `machine.toml`
  and the generated chezmoi config. Another computer registered under the old
  name must register again after pulling.
- **Retire**: approval-required (never an agent). Removes the machine from every
  exclude and from every include list that has other includes, and marks it
  retired (the entry is kept). When it is a file's *only* include it stays there:
  a retired machine never matches, so the file then applies to no machine (never
  to all); the preview says "applies to no machine now". Variants or overrides
  naming it are listed as notes, not edited. The preview lists each assignment and
  every file whose applicability changes. This machine cannot be retired.
- **`dw machine use NAME`** (human only) re-points this computer's `machine.toml`
  and generated chezmoi config to an existing registry entry whose platform and
  distribution match, for when the machine was renamed on another computer.
  `dw status`, `dw doctor` and the TUI Status panel say when the local name is not
  in the registry.
- **Preview as machine** (`dw machine preview NAME [--diff]`, TUI Machines panel
  Enter): `chezmoi managed` (and `diff` against this HOME) with a throwaway
  config, state and cache and the machine name overridden; HOME and the real
  state are never written. Retired machines are refused. Secrets are masked in
  previews.

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
  the source entry from HEAD. A variant file is staged by writing the HOME content
  into the variant file this machine currently receives (the default when it
  receives the default), never into another; the classifier, `dw diff` and
  discard use the rendered variant this machine receives.
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
- **Variables** live in plain data, one file per variable
  (`.chezmoidata/dotwarden-values/<NAME>.toml`, so two computers editing
  different variables never conflict textually; a machine rename rewrites
  `machine:<old>` overrides there). Names are 1–64 of `[A-Za-z0-9_-]`, unique
  ignoring case (they are file names). A value is 1–1024 characters on one
  line. A machine receives the most specific matching override, else the
  default (a clash at one level is an error naming the targets).
  `dw values list|show NAME|set-variable NAME VALUE [--target T]|
  unset-variable NAME [--target T]|wire PATH NAME` — each change is one pending
  item with an exact preview (files, data, the value each registry machine
  receives before and after, files using the variable); any agent may add and
  apply them. An override needs a default first; removing a variable that
  files still reference is refused.
- **Reference syntax** (what users see in the repository):
  `{{ includeTemplate "dotwarden/value" (dict "data" . "name" "NAME") }}`.
  A file with references is stored as a template (`dot_x` → `dot_x.tmpl`);
  every `{{` of its ordinary text is written as `{{ "{{" }}` (and a final `{`
  before a reference as `{{ "{" }}`), so literal braces survive. A template
  holding any other action is not one dotwarden wrote: wiring and staging it are
  refused. In a variant the references live in the variant file (it is rendered
  as a template when it contains the marker, else included raw), so only the
  variants wired gain them; a variant file whose last reference disappears is
  stored raw again.
- **Wire** (`dw values wire PATH NAME`): replaces every occurrence of the
  variable's value *for this machine* in the stored content (for a variant file,
  the variant this machine receives) by a reference; the preview names the
  occurrences' lines. HOME is not touched, and the file renders identically
  here; other machines render their own value.
- **Reverse mapping on stage.** HOME is diffed (by character) against what the
  stored template renders on this machine; each reference position that still
  holds the resolved value stays a reference; edits elsewhere go into the
  literal text. If the text at a reference position changed, the reference is
  kept and the variable is reported changed: `dw status` ("Variable NAME
  changed: a → b"), `dw diff` (a `# variable NAME changed` line) and the stage
  output. Staging then writes the new value to the variable at the most
  specific target this machine matched (that override, else the default; the
  message names which), in the same staged change. Text at a reference position
  that was deleted drops the reference. Refused: different new text at two
  positions of one variable, an edit too large to be a value, files that are
  not UTF-8. Other files that reference the changed variable then show it as
  Incoming (their HOME holds the old value). The working diff never contains
  reference text.
- **Detection**: after `track` and `stage`, when a file's content contains this
  machine's value of an existing variable (3+ characters), a pending `wire`
  item is proposed (never applied automatically) and reported.
- The `secret` kind plugs into the same table and reference mechanism (ticket 8):
  `home/.chezmoidata/dotwarden-values/<NAME>.toml` holds `kind = "secret"`, the
  override `targets` (no values) and `placeholder = true` for a name without a
  value; the values are in `vault/secrets/<NAME>.age`. `dw values add-secret
  NAME [--target T] [--placeholder]` reads the value from a terminal only (agents:
  placeholders only); a new secret needs only `recipient.txt`, changing an
  existing one needs an unlock. The `dotwarden/value` helper template calls
  `dw __secret NAME TARGETS` through chezmoi's `output` (`DOTWARDEN_HELPER`,
  `DOTWARDEN_RENDER_TOKEN` in chezmoi's environment); `DOTWARDEN_MASK` renders
  `«secret:NAME»` instead (previews as another machine). While locked, files
  that need the vault are not classified or applied (exit 4 when named).
  `dw encrypt PATH` turns a tracked file into a stub template
  (`dotwarden/file`) plus `vault/files/<id>.age`; detection proposes a
  `secret_move` pending item (file, line, byte range, no value); commits
  holding key-like text are refused (`DOTWARDEN_ALLOW_SECRETS=1`, humans).
- Secrets live in
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
- The TUI Values screen (`V`) shows names, kinds, defaults, overrides per
  target and the files using each variable (secret rows arrive with ticket 8;
  their values only while unlocked and only in the TUI). `Enter` edits the
  default in an input box, `o` adds an override (`TARGET VALUE`), `n` creates a
  variable (`NAME VALUE`); each becomes a pending item. `Esc` returns.
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
`+`/`_` screen mode, `y` copy path, `m` assign, `t` add a variant, `E` encrypt, `.` show hidden,
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
