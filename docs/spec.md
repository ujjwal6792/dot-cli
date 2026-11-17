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

- Backing up is one habit-sized step (`dw sync`, TUI `S`) and an overdue backup is
  visible; moving from a bare git repository in HOME is one command (`dw import`).

Non-goals: Windows; carrying over the history of an existing dotfile setup
(import tracks current files only, ADR 0008); background sync daemons; machine
status reporting; raw chezmoi passthrough; hosted services.

## 2. Distribution and runtime (ADR 0002)

- Release bundle per target: macOS arm64, Linux x86-64, Linux arm64. Contents:
  `bin/dotwarden`, `bin/dw` (symlink), `libexec/chezmoi` (v2.73.3 at time of
  writing), `libexec/git/` (portable git from desktop/dugite-native, v2.56.0-2 at
  time of writing), `libexec/delta` (0.20.1), licenses (git is GPLv2: include its
  license and a source offer/link), man pages and shell completions
  (`share/man/man1`, `share/completions`), and a SHA256 manifest. Pins are
  recorded in one file in the repo and verified at build time.
- Release rollout: Dotwarden versions start at 1.0.0 (the `v0.x` tags belong to
  dot v1). v1.0 publishes macOS arm64 only. Linux x86-64 is built and tested by
  CI from the start and attached to prerelease (`-rc`) tags only, until the owner
  verifies it on a real machine; it becomes a stable target in v1.1. Linux arm64
  stays pinned but unpublished. Intel Macs are unsupported (stated in the README).
- Releases are built by GitHub Actions on a `v*` tag (macOS and Ubuntu runners):
  format, clippy and tests, then bundles, the SHA256 manifest and a GitHub Release
  on the public `dot-cli` repository, with `install.sh` as a release asset.
- `curl -fsSL <url>/install.sh | sh`: detects OS/arch, downloads and verifies the
  bundle, installs to `~/.local/share/dotwarden/releases/<version>/`, points
  `~/.local/share/dotwarden/current` at it, and links `dotwarden` and `dw` into
  `~/.local/bin`. No sudo. Prints PATH advice if needed. Re-running it upgrades.
  On a stable release it refuses targets that are not published yet (Linux until
  v1.1) with a message naming the prerelease to use instead.
- `dw self-update [--check] [--dry-run]` reads the latest GitHub release, verifies
  checksums and switches `current`; keeps the previous version for
  `dw self-update --rollback`. Prereleases only with `--pre`. Disabled (with a
  message) when installed by a package manager or run from a source build.
- Packages (after v1.x): `.deb` (Ubuntu), AUR `dotwarden-bin`, Homebrew tap.
  Packages install the same bundle layout under the package manager's prefix.
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
| `$XDG_STATE_HOME/dotwarden/tui.log` | TUI action errors and refusals with timestamps (0600, trimmed to ~64 KB; paths, lines and hints only, never values) |
| `$XDG_STATE_HOME/dotwarden/machine.toml` | This machine's registry name and remote-privacy confirmations |
| `~/.config/dotwarden/config.toml` | Preferences: an ordinary, trackable dotfile |

Every chezmoi invocation passes explicit `--config`, `--source`,
`--persistent-state`, `--cache` and `--destination`, and an environment with
`CHEZMOI_*` variables removed, so a user's own chezmoi installation is neither read
nor modified. Every git invocation uses the bundled git with
`GIT_CONFIG_NOSYSTEM=1`; the user's global config is read for `user.name`,
`user.email`, signing and credential-helper settings, and the system `ssh` with
the user's keys and agent handles SSH remotes. Settings can override identity per repository.

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
   `gpg.{openpgp,x509,ssh}.program` plus the credential keys `credential.helper`,
   `credential.<url>.helper`, `credential.useHttpPath`,
   `credential.<url>.useHttpPath` and `credential.<url>.username` are read from
   the user's global config (honouring `GIT_CONFIG_GLOBAL` if the user set it)
   and passed on as `GIT_CONFIG_COUNT/KEY/VALUE`. Credential entries keep their
   order and empty values, because an empty `helper` resets the list. The
   anonymous public-remote probe is the exception: it gets none of them. Hooks, `core.fsmonitor`, URL rewrites, aliases
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
- **Apply** incoming changes: per file or all; snapshot before overwriting. When
  the HOME file being replaced was never written by dotwarden on this machine
  (typically the first apply after `clone`), apply still overwrites it, and the
  output (and `--dry-run` up front) lists every such file under "replaced
  existing files" with the snapshot id and `dw snapshot restore` to undo.
- **Conflicts**: 3-way view (last applied / HOME / incoming) with per-hunk
  local/incoming choice; `e` opens `$EDITOR` (nvim diff mode when the editor is
  nvim) on a merge file. Result is staged.
- **Untrack**: remove from the repo; HOME copies on every machine are left in
  place; other machines show "no longer tracked" once.
- **Tracked folder**: tracking a folder tracks its current files and records the
  folder; new files inside appear as New. Nothing is added automatically.
- **Several paths at once**: `track` adds items for every valid path and lists
  each refused one with its reason (hidden, possible secret, missing, not under
  HOME) under "skipped"; it exits 1 only when nothing was added. The same rule
  holds for `import` (§5a) and for staging in `sync` (§6a).
- Symlinks are tracked as symlinks.
- **Hide rules** (`hide`, shared, gitignore syntax) remove noise from the browser
  and refuse tracking. Sensible defaults (caches, histories, `node_modules`,
  `Library` except explicitly un-hidden paths, …) are written at repo creation.
  A running TUI reloads them when `hide` changes (from `i` or `dw hide`).

### 5a. Import (ADR 0008)

`dw import --bare DIR | --from-file LIST | --from-file -` turns an existing
setup's file list into track pending items for the files as they are in HOME now.
`--bare` reads `git --git-dir=DIR --work-tree=$HOME ls-files` with the bundled git
(read-only; the old repository is never written). Paths are HOME-relative; paths
that do not exist in HOME are skipped. A folder is tracked as a tracked folder
when every non-hidden file currently in it was in the list (deepest such folder
wins, never HOME itself); otherwise its files are tracked one by one. The
result follows the several-paths rule (§5): valid paths become items, hidden,
flagged-secret (without a vault) and missing paths are listed under "skipped"
with reasons; with a vault, secrets are moved to it as in `track`. `--dry-run`
shows the grouping, the items and the skips; `--apply` applies the items. The
items record the source (`DIR` and its HEAD short hash); while staged changes
come from them, the message `sync` generates and the TUI commit box's prefill
start with `Import from <DIR> at <hash>`. Agents may import (it only adds
pending items).

Clarified in T18: a complete folder (every non-hidden file in it listed) that
holds listed files directly becomes the tracked folder and covers its
subfolders; a complete folder holding only subfolders gives way to them, so
"deepest wins" picks e.g. `~/.config/nvim` (not `~/.config/nvim/lua`, and not a
fully listed `~/.config` that holds no files itself). Lists hold one path per
line (or NUL-separated); blank lines and lines starting with `#` are ignored;
relative paths are HOME-relative, `~/x` and absolute paths inside HOME are
accepted. Stdin is shown as `Import from stdin`. Files flagged as secrets inside
a collapsed folder are left out of it (as in `track`) and also listed under
"skipped". Nothing added exits 1. Applied import items record their label and
paths in `import.json` (state directory); the next commit removes it, and a
message is prefixed only while one of those paths is staged. The prefixed
generated message is `Import from … at …`, a blank line, then the usual sync
line.

## 6. Git

Covered in CLI and TUI: fetch, pull (fast-forward when possible; otherwise rebase
only unpushed commits, aborting on conflict; explicit `--merge` keeps conflicts
for stored-form resolution and `--continue`/`--abort`), push (to one or all
remotes), branches (list/create/switch/delete), remotes (add/remove/list), log with
per-commit diff and file history, revert a commit, stash/pop, point-in-time
restore. All machines share one default branch; the UI warns when a machine is on
another branch. Background fetch runs on TUI start and at a preference interval;
it never pulls or applies.

Branch create does not switch. Switch changes only source/index; overlapping
staged edits are refused, and HOME drift is retained. Branch deletion requires
human approval; current branches cannot be deleted, and unmerged deletion also
requires explicit `--force`. The current non-default branch is warned about.

Dotwarden stashes are local records, not ordinary Git stashes: they retain the
exact source index in `refs/dotwarden/stashes/` and tracked HOME contents
separately, preserving partial staging, references and variants. Untracked HOME
files are excluded. Push cleans affected HOME/index to HEAD after a snapshot;
pop requires the same HEAD, clean index and unchanged affected HOME paths.
Conflicts retain the stash. Payloads and safety snapshots are age-encrypted when
a vault exists (unlock required); without a vault, detected secret-like content
is refused. Custom templates, binary/symlink files and staged vault identity
changes are refused rather than flattened. Staged tracking additions/removals
are included; previously untracked HOME files remain outside the stash.
Push/pop preview HOME and index paths and require confirmation; drop uses human
approval. These records are separate from any stashes made by external Git.

**Public remotes are refused** (ADR 0006). For github.com, gitlab.com,
codeberg.org and bitbucket.org, an anonymous `git ls-remote` over HTTPS with
credential helpers and prompts disabled succeeding means public. Checked when a
remote is added and before every push; push is blocked if any target remote is
public; offline skips the check with a warning. Unverifiable hosts require a
one-time human confirmation that the remote is private, stored per remote in
`machine.toml`; agents cannot give it.

**Clone pushes the registration.** `dw clone` by a human pushes the "Register
machine" commit to the cloned remote right away (metadata only; the privacy check
runs first), so two new machines never diverge on their registrations. Offline,
on a refused or failed push, or for an agent, the commit stays local and clone
prints `run dw push to share this machine with the others` (exit 0).

**Pending pushes are satisfied by a push.** A successful push removes the pending
push items it satisfied (same branch, the remote it pushed to, or all remotes when
the item named none) and says so.

**HTTPS remotes** work through the user's own credential helpers from the global
git config (for example the one `gh auth setup-git` writes), passed through as in
§3 item 7. Fetch, pull and push never prompt (`GIT_TERMINAL_PROMPT=0`); the public
probe stays anonymous and gets no helper. When an HTTPS fetch or push fails with an
authentication error, the message suggests `gh auth setup-git` or the SSH form of
the URL (`git@github.com:owner/repo.git` for known hosts). `dw remote add` warns on
HTTPS URLs to known hosts only when no credential helper applies to the URL, and
`dw doctor` reports whether the default remote is HTTPS and has a helper.

**Backup state.** `dw status` (and `--json`, the TUI Status panel and banner)
reports the last successful push from this machine (recorded in the state
directory) and the number of unbacked changes: Local and New files, staged
paths, and commits not on the upstream. When unbacked changes exist and the
oldest is older than the `backup_warn_days` preference (default 7), it is a
warning: `last pushed 9 days ago · 4 unbacked — run dw sync`. Nothing is shown
when everything is pushed. The age of the oldest unbacked change is measured from
this machine's last push (an upper bound), or, never pushed, from the oldest
commit no remote holds; the record is `last-push.json` in the state directory,
written by every successful push.

### 6a. Sync (ADR 0009)

`dw sync [-m MSG | -e] [--yes] [--dry-run] [--remote NAME]` runs, stopping at the
first step that needs a decision:

1. Fetch and pull (fast-forward only; a divergence stops with the `pull` hint).
2. Apply Incoming files that have no Local edit, with snapshots; Conflicts and
   files that need an unlocked vault stop sync before anything is staged (exit 5
   or 4) and name `dw resolve` / `dw vault unlock`.
3. Stage every Local file and every New file in a tracked folder (the several-paths
   rule, §5: flagged secrets without a vault, clashed variants and refused
   templates are skipped and listed). Pending items are never applied.
4. Commit everything staged (also changes staged earlier) with `-m`, an editor
   (`-e`), or the generated message `sync from <machine>: N changed, M new
   (<first few paths>)`. Nothing staged means no commit.
5. Push to the upstream remote (or `--remote`), with the privacy check; push only
   when HEAD is ahead. A repository with no remote at all skips steps 1 and 5:
   sync commits locally, says how to add a remote (`dw remote add origin URL`)
   and exits 0; `dw status` keeps reporting the commits as unbacked.

In a terminal, sync first prints the plan (files applied, staged, skipped, the
message, the push target) and asks `Proceed? [Y/n]`; `--yes` skips the question.
Without a terminal and without `--yes`, and for agents, steps 1–4 run (they need
no approval) and the push becomes a pending item (exit 3). `--dry-run` prints the
plan using the last fetch, without network. Staging and the commit are journalled
like their own commands (§11), so `dw undo` reverts the commit while it is on no
remote; pulls are not journalled and applied files are recovered from snapshots.
The exit code is the first stopping step's (§9), else 0.

The plan in a terminal and in the TUI is made after step 1 (a fast-forward pull
changes no HOME file), so it lists exactly what will be applied and staged;
declining leaves the pull in place. `--remote` names the remote for both the
pull and the push. Diverged history exits 1. Incoming files without a Local edit
are applied before a conflict or locked vault stops sync.

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
  updates the vault (requires unlock) and keeps the reference. Without a vault,
  tracking a single flagged file is refused (message: file, line, hint, never the
  value); tracking a folder tracks its clean files and lists the flagged ones as
  "skipped: possible secret" in the item (they stay New). Assignments whose value
  is code, an env var name, a placeholder or a plain word are not flagged.
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
  the selected items as one transaction (one lock, one chezmoi `add`, one data
  write, one `git add`); on failure it is rolled back and the items are retried
  individually. Items can be dropped. Stage/unstage/commit are immediate
  (lazygit behaviour); `z` undoes.
- Agents (CLI without an interactive human approval) may: inspect anything except
  secret values; track, untrack, import, stage, unstage, assign, commit, sync (its
  push becomes a pending item); add secret placeholders and wire references; add
  pending items.
- Requires human approval: push, discard, apply over local changes, history
  rewrite (amend of pushed commits, rebase), branch delete, machine retire, remote
  add/remove, privacy confirmation. From an agent these become pending items
  marked "requested by <agent>"; the CLI exits with a distinct code. Humans approve
  in the TUI or with `dw approve <id>` in an interactive terminal.
- This is a guard against agent mistakes, not a security boundary against a
  hostile process running as the same user.
- Agent identity: `DOTWARDEN_AGENT=<name>` (and known agent environments) label the
  activity log and commit trailers. `AI_AGENT`, `CLAUDECODE`, `OPENCODE`,
  `CODEX_SANDBOX`, `CURSOR_AGENT` and `GEMINI_CLI` are recognised, but only when
  stdin/stdout is not a terminal, so a person in an agent's terminal still acts as
  themselves. `dw activity [-n N] [--actor A]` shows the log; records hold the
  command name, actor and exit code only. Reads and dry-runs are not logged.

## 9. CLI contract

- Every command supports `--json`; human output by default. Every argument and
  flag has help text. `dw completions <shell>` and a man page are generated.
- Paths are HOME paths (`~/x`, absolute, or relative to the current directory);
  repository paths are never required.
- Exit codes: 0 ok, 1 error, 2 usage, 3 needs approval (added to pending),
  4 vault locked, 5 conflict, 6 refused public remote, 7 stale (state changed).
- Mutations take a short lock and wait up to 10 s rather than failing instantly.
- Commands (indicative): `init`, `clone`, `import`, `sync`, `status`, `ls`, `diff`,
  `track`, `untrack`, `stage`, `unstage`, `commit`, `discard`, `apply`, `pull`, `push`,
  `fetch`, `branch`, `remote`, `log`, `show`, `stash`, `revert`, `restore --at`,
  `assign`, `variant`, `machine (list|add|rename|retire|preview)`,
  `values (list|set-variable|add-secret|wire)`, `vault (init|unlock|lock|passwd|recover)`,
  `pending (list|drop|approve)`, `snapshot (list|restore)`, `archive (export|import)`,
  `hide`, `settings (list|get|set|unset)`, `doctor`, `self-update`; also `approve`, `activity`,
  `completions`, `man [--dir]` and `skill [--install DIR]` (the bundled agent skill).
- Every mutation supports `--dry-run`, returning exactly what would change.
  Exception: `vault init|unlock|lock|passwd|recover` (interactive passphrase flows).
  `pull --dry-run` uses what was last fetched and never touches the network.

## 10. TUI

Layout: left column 1 Status (machine, branch, ahead/behind, vault state, pending
count), 2 Files (HOME browser, yazi-like, hidden items toggled with `.`), 3 Changes
(Local / Staged / Incoming / Conflicts), 4 Git (Commits / Branches / Remotes /
Stash), 5 Machines (registry, assignments, preview-as). Right: main view (diff via
delta, preview, assignment editor). Bottom-right: Pending list. `V` opens Values
full screen. Narrow terminals show the focused panel only.

Default keys (lazygit-compatible, remappable in preferences): `Space` stage/track,
`a` stage all (in Files: track all visible entries of the folder, or untrack/drop them when all already are); `A` in Files drops every pending item of the folder, `v` range select, `c` commit, `A` amend, `P` push, `p` pull,
`f` fetch, `d` discard, `e` `$EDITOR`, `o` open, `i` hide, `/` filter, `[` `]`
tabs, `h`/`l` panels, `1`–`5` panels, `x` menu, `z` undo, `?` help, `@` command log,
`+`/`_` screen mode, `y` copy path, `m` assign, `t` add a variant, `E` encrypt, `.` show hidden,
`V` values, `G` open lazygit on the repository (suspends dotwarden), `H` history
of the selected file (Files, Changes) on the Commits tab, `Enter` drill
in / staging view, `Esc` back, `q` quit, `S` sync (plan dialog, then y/n; §6a),
`,` settings.

**Onboarding.** Without a repository the TUI opens an onboarding screen instead
of the panels, with three paths, each previewing and running the same code as the
CLI:

- *New*: machine name (suggested from the hostname), then "Set a vault passphrase
  now? [Y/n]" (vault init; the recovery code is shown once and must be confirmed
  with "I saved it"), then the remote: when `gh` is on `PATH` and authenticated,
  offer `gh repo create <name> --private` (default `dotfiles`) and use its SSH
  URL; else ask for an SSH URL with an example. Then a first `track` suggestion
  list (common dotfiles that exist in HOME, none preselected).
- *Clone*: URL and machine name, then the vault unlock if the repository has one,
  then the Incoming tab with the "replaced existing files" summary (§5).
- *Import*: the bare repository path (default `~/.dotfiles`, `~/.cfg` if present),
  the import preview (§5a), then the New path's vault and remote steps.

`dw init` in a terminal asks the same vault and remote questions after creating
the repository (`--no-vault`, `--no-remote` skip them; agents never get them).

**Backup banner.** The Status panel shows the backup state (§6); when it is a
warning, a one-line banner above the panels repeats it with `S sync`.

**Discoverability and navigation.** Each panel's footer shows its most useful
keys for the current selection (fitting 80 columns; `?` lists all). The main view
scrolls (`Ctrl-d`/`Ctrl-u`, `PgDn`/`PgUp`, `Home`/`End`, mouse wheel; `J`/`K`
by line) for diffs, previews and logs. Mouse: click focuses a panel and selects a
row; double-click is `Enter`. `v` range-selects in Files and Changes; Space then
acts on the range.

**Flows that were CLI-only.** Variants: `t` adds, `D` on a listed variant removes.
Values screen: `w` wires the selected variable into a file (path picker), `-`
unsets the selected override or (when unreferenced) the variable. `@` opens a
log with two tabs: *Commands* (the bundled tool commands dotwarden ran this
session; arguments only, values masked) and *Activity* (the records of
`dw activity`, filterable by actor).

Requirements: the event loop never blocks on I/O; background jobs (preview,
refresh, fetch) never block user actions and are cancelled when superseded; an
editor or lazygit handoff suspends and restores the terminal; theme colours come
from preferences.

## 11. Recovery

- Snapshots before any overwrite of HOME; retained 30 days with at least the 5 most
  recent per file (preferences).
- `z` (TUI) / `dw undo [--dry-run]` undoes the last local action: stage/unstage
  (also of hunks and lines), track/untrack, assign and every other applied pending
  item, discard (from its snapshot) and the last commit while it is on no remote
  branch (HEAD moves back, the changes stay staged). Never a push. The journal
  (`undo.json`, 20 actions, in the state directory) stores HEAD and the index tree
  before and after each action plus chezmoi's state; the repository equals its
  index, so that describes it completely. An entry is undone only while the
  repository still is in its recorded "after" state; otherwise exit 7 (stale)
  and nothing changes. Pull, revert, branch and stash operations are not journalled.
- Point-in-time restore of one file or all files from any commit (through Incoming
  so it is reviewed).
  `restore --at REV PATH…|--all` saves only commit IDs and guards locally. It
  renders historical data/variants for this machine and requires both historical
  and current assignments to receive each path. HOME and source/index/HEAD are
  unchanged until apply; `diff` previews the candidate. Local/staged collisions
  appear as Conflicts; changed guards refuse apply. `restore --clear` removes
  candidates. Apply snapshots HOME (encrypted with the vault), writes the
  historical result as a Local edit against current source, and preserves
  unrelated forward Incoming changes. Regular UTF-8 files and Dotwarden-owned
  templates are supported; locked secrets require unlock. Stage/commit the
  result normally to make the restore part of shared history.
- Archive: `dw archive export FILE [--force] [--dry-run]` writes a git bundle of
  every ref of the repository (registry and vault included) with this machine's
  registry name and its `remote.*`/`branch.*` settings, sealed with age passphrase
  encryption under the vault passphrase (asked on the terminal; it must be the
  vault's). Staged but uncommitted changes are not included (a warning says so).
  `dw archive import FILE [--dry-run]` on a computer without a repository decrypts
  it, rebuilds the repository with all branches and remotes, and registers the
  computer under its old name; `dw apply --all` then writes its files. No network
  is used, and a failed import leaves nothing behind. Privacy confirmations are not
  carried over.

## 12. Settings

A Settings screen (`,`) and `dw settings` cover: git identity/signing per repository,
remotes, default branch, fetch interval, diff/merge tool, chezmoi options exposed
by dotwarden (e.g. umask, editor), unlock timeout, snapshot retention,
`backup_warn_days`, theme and keymap. Changes to repository-wide settings go
through the pending list.

- **Two kinds.** Preferences are this person's and machine's: they live in
  `~/.config/dotwarden/config.toml` (an ordinary, trackable dotfile) and take
  effect immediately (the TUI reloads the file when it changes). Repository-wide
  settings (default branch, per-repository git identity and signing, chezmoi
  options) live in the repository and change through pending items with a
  preview. Remotes keep their own commands (§6).
- **Preferences file.** Unknown keys and invalid values are reported by
  `dw doctor` and the Settings screen and fall back to their defaults; the file
  is never rewritten except for the keys the user changes (`toml_edit`, comments
  kept). Keys: `fetch_interval_minutes` (default 10, 0 = off),
  `unlock_timeout_minutes` (15), `snapshot_retention_days` (30),
  `snapshot_keep_per_file` (5), `backup_warn_days` (7), `diff_tool`,
  `merge_tool`, `editor` (default `$VISUAL`/`$EDITOR`), `theme`, `[theme.colors]`,
  `[keys]`.
- **Keymap.** `[keys]` maps action names to keys, lazygit-like:
  `stage = "space"`, `commit = "c"`, `sync = "S"`; one key or a list. Action names
  are listed by `dw settings keys` and in `?`. Two actions bound to the same key in
  one panel, unknown actions and unparsable keys are reported and those bindings
  fall back to their defaults.
- **Themes.** `theme = "dark" | "light" | "terminal"` (`terminal` uses only the
  16 ANSI colours, so the terminal's palette decides); `[theme.colors]` overrides
  roles (`accent`, `border`, `selected`, `added`, `removed`, `warning`, `error`,
  `muted`) with `#rrggbb` or ANSI names. delta follows the theme's light/dark.
- **CLI.** `dw settings list` (every key, value, default and source),
  `get KEY`, `set KEY VALUE [--dry-run]`, `unset KEY`, `keys`. Repository-wide keys
  add a pending item; agents may change preferences but not identity or signing
  (those need approval).
- **Screen modes.** `+`/`_` cycle normal → half → full-screen main view.

## 13. Verification

- Unit and integration tests run with the bundled tools and isolated HOME/state,
  on macOS only. Implementations do not run Linux, Docker or emulated tests; the
  owner verifies Linux behaviour on real Linux machines after development, and
  Linux fixes follow from that. Linux-specific code must still compile and carry
  unit tests that do not need a Linux host. The release workflow's Ubuntu runner
  (§2) builds the Linux bundle and runs the test suite there; its result informs
  the owner's verification but does not replace it.
- Release gate for v1.0: the owner backs up their own dotfiles with dotwarden
  (imported from their bare repository, `dw sync` daily) for one to two weeks on
  a release candidate; problems found become tickets before the tag.
- A multi-machine scenario on macOS: several isolated HOME/state directories
  registered as different machines (with overridden platform/distribution data)
  share a local bare remote and exercise assignments, variants, values, vault
  unlock, incoming, conflicts and retire.
- TUI tests use Ratatui's test backend with real key events; a PTY smoke test
  covers start, handoff to editor, resize and exit.
- No test may print a secret value; a test asserts outputs and logs contain none.
