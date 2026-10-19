# dot

A Rust + Ratatui workspace for **chezmoi, Git and machine configuration**.
Chezmoi imports, renders, compares and applies files. Dot presents those operations
in a persistent workspace with exact reviews, Git staging/commit/push, local
activity, recovery and optional machine synchronization.

The Go application has been replaced. The accepted design and original defects
remain in [the design walkthroughs](docs/tui-redesign-walkthroughs.md) and
[bug register](docs/tui-redesign-bugs.md). See [the Rust spec](docs/rust-rewrite-spec.md)
for implementation contracts and [verification](docs/rust-acceptance.md) for evidence.

## Build and run

Install Rust, system Git and chezmoi **v2.73.1**, then:

```sh
cargo build --locked --release
./target/release/dot --help
./target/release/dot init --repo git@github.com:YOUR_NAME/dotfiles.git
./target/release/dot
```

For an existing ordinary chezmoi source, use `dot init --source /absolute/source`.
Without `--repo`, init connects the existing source or initializes a new Git
repository. It creates local dot configuration and a small agent skill at
`~/.agents/skills/dot/SKILL.md`; it does not apply HOME files. Existing skill
customizations are preserved, with conflicting updates written separately.

When the TUI cannot find its source, its setup screen offers `i` to initialize a
local source, `c` to clone a repository, and `o` to connect an existing checkout.
These actions are clickable. Esc/Enter returns to browsing without setup; q exits.

Global overrides: `--config`, `--home`, `--source`, `--state-dir`,
`--chezmoi-config`. Use separate absolute paths for isolated environments.
The optional dot config defaults to `~/.config/dot/config.toml`; state and private
backups default to `~/.local/state/dot`. XDG configuration/data/state variables
are respected. Source defaults to `~/.local/share/chezmoi`; use `--source` for an
existing custom checkout. The config path is a **dot** TOML file; use
`--chezmoi-config` for native chezmoi configuration.

## Workspace

Persistent Status, Files, Git and Machines panels share a large details area and
compact activity log. `.agents/` and `.config/` appear as folders, not hundreds of
flattened paths. All includes grey untracked entries; Managed and Changed narrow
the view. Yellow folders have changed descendants. HOME matching source does not
mean the source is committed or published.

| Input | Action |
|---|---|
| `1`–`4`, `0`, Tab | Focus panels/details |
| arrows or `j/k`, `h/l` | Navigate rows/folders |
| Space in Files | Select files/folders |
| `c`, `a`, `r` in Files | Capture, apply, restore |
| `e` in Files | Edit native source; Ctrl+S reviews changes |
| Space in Git, `v` | Stage/unstage; multi-select Git paths |
| `c` in Git, `P`, `L`, `F` | Commit, push, pull, fetch |
| `/`, `f` | Search; All/Managed/Changed |
| `[`, `]`, `z` | Previous/next tab in the focused section; expand focused panel |
| `:`, `?`, Esc | All actions, help, back/cancel |

Mouse clicks focus rows/panels; checkboxes select, double-click opens folders,
and scrolling follows the pane. Action menus support double-click activation.
Review buttons are clickable. Terminal-delivered touchpad events use the same
paths. Narrow terminals show the focused pane and retain Actions/Help.

Each section shows its tabs in the border: Status (Overview/Paths), Files
(All/Managed/Changed), Git (Changes/Commits), Machines (Local/Reports), Details
(Diff/Source/Rules/Activity), and Activity (Recent/All). Click a tab or use `[`/`]`
in that section. Each section keeps its own active tab; Tab still changes focus.

Esc/Enter dismisses an error; `q` exits an error/help screen and Ctrl+C exits from
any screen unless a write operation is still running. Inspection does not block
exit. Failed inspection pauses automatic retries; use `R` or connect a source
through Actions to retry explicitly.

Actions include local policy controls, service setup, package-manager handoff,
source switching, activity/recovery and shared setting editing. Expensive chezmoi
and Git calls run off the terminal event loop. Text input owns character shortcuts.

## Directional file and Git workflow

Capture is **HOME → source**. Apply/restore is **rendered source → HOME**.
Restore explicitly overwrites local differences, with a file backup. Stop tracking
removes shared source but keeps HOME. Enable/disable only changes this machine's
automatic-update selection; restoring one file preserves the rest of that set.

Stage, commit and push are separate. Commit shows the actual Git index, including
externally staged work. Pull is clean and fast-forward-only. Resolve divergence,
rebase and advanced history operations through Git/Lazygit, then refresh dot.

For scripts and agents, inspection and plans emit JSON:

```sh
dot status
dot diff .config/ghostty/config
dot plan capture .config/ghostty/config
# Review the returned paths, direction, diff and id:
dot execute PLAN_ID --approve PLAN_ID
dot plan stage dot_config/ghostty/config
dot plan commit -m 'Update terminal settings'
dot plan push
```

Execute each reviewed plan separately. Changes to source, reviewed HOME paths,
local rules or Git state invalidate old plans. Empty selections never mean all.
`DOT_ACTOR` and `DOT_INTENT` provide optional non-secret activity labels. Unknown
external changes have no invented attribution. Local activity retention defaults
to 30 days; raw config values and command output are excluded from activity records.
Native commands have a 120-second timeout; `DOT_COMMAND_TIMEOUT` can extend it
(up to one hour). Timed-out operations need partial-result inspection.
Native chezmoi templates/hooks may execute during inspection; the TUI is not a
sandbox for untrusted repository code.

## Shared settings

`dot setting` stores optional settings in ordinary `.chezmoidata.json` and a native
chezmoi helper. Shared → OS → hostname overrides are resolved **by chezmoi**:

```sh
dot setting ghostty.font-size 16                # review the generated plan
dot setting ghostty.font-size 18 --scope machine
# Execute each exact plan after review, or use --yes for an explicitly intended edit.
```

Bind the setting explicitly in the corresponding native `.tmpl` source:

```gotemplate
font-size = {{ template "dot-setting" (dict "root" . "key" "ghostty.font-size") }}
```

Machine scope uses chezmoi's hostname, independently of dot's display label.
`--scope os` uses chezmoi's OS. `--remove --scope machine` removes the override to
inherit again. Values are literal strings; templates own format-specific quoting.
Arbitrary existing templates remain editable source, with no promise of automatic
inverse mapping from a rendered HOME edit. Capture refuses to flatten templates.

## Optional synchronization and reports

Enable exact files through the workspace or `dot plan enable PATH...`, then opt in:

```sh
dot settings sync true --yes
dot sync
dot service --install --start
```

The local launchd/systemd user service checks every five minutes while awake,
backs off offline and pauses on dirty files/conflicts. Strict mode is conservative:
it holds dynamic/special sources, hooks, templates, externals, additions, removals
and other unproven behavior for review. Explicit per-machine
`dot settings trusted true --yes` permits native unattended repository behavior;
repository code can then have broader effects. This policy cannot be enabled by
changing the shared source. Only enabled destinations are passed to apply.

`dot settings reporting true --yes` separately enables automatic **report-only**
commits/pushes to `dot-reports/INSTALLATION_ID` branches. Reports contain revision,
time, outcome and counts. They do not touch the configuration branch or index, and
have the same visibility as the repository. `dot reports --fetch` refreshes cached
summaries; the Machines view always describes last reported state. Automatic
publication skips unchanged summaries until an hourly heartbeat. Upload failures
are distinct from local apply results. Detailed activity stays local by default.

## Recovery and migration

`dot recovery` lists private operation backups. `dot recovery ID --yes` restores
supported file operations only when the state still matches their recorded result.
Interrupted operations keep backup files for manual inspection; arbitrary script
side effects are not reversible. Backups and plan contents may contain private
config values and remain under the private local state directory.

`dot migrate OLD_SOURCE NEW_SOURCE` previews conversion of an old dot catalog.
Add `--yes` to archive originals and convert supported plain entries into a **new**
chezmoi source. Ambiguous mappings, OS/machine constraints, custom templates and
partial `.gitconfig` semantics are held for explicit translation. Live HOME remains
unchanged. `dot source /existing/new/repository --yes` archives the old source and
switches connection, clearing local automatic scope/trust. It keeps the original
checkout and HOME files.

## Distribution and verification

Release tags build native macOS ARM64, Linux x86-64 and Linux ARM64 bundles.
Each includes `dot`, verified private `libexec/chezmoi` v2.73.1, its license and
release metadata. Keep the whole bundle together and verify its SHA256SUMS before
installing. Add the extracted directory to PATH or symlink its `dot` executable.
System Git remains external. The retired Go install/update command protocol is
not used; replace a verified bundle to upgrade, retaining local configuration/state.

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

Integration tests exercise real chezmoi and local Git remotes with isolated HOME,
source, config and state. Current local evidence is macOS; native Linux execution
belongs to the release matrix and is not implied by a cross-build.
