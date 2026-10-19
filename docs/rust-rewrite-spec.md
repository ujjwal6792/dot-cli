# Rust + Ratatui workspace specification

Accepted direction: discovery Q1–Q25 plus the user's explicit Rust rewrite request
on 2026-10-19. This document supersedes Go-specific implementation contracts.
The original defects retain stable IDs in `tui-redesign-bugs.md`.

## Architecture

One Cargo package builds `dot`. Ratatui/Crossterm own terminal drawing and events;
worker threads execute slow inspection and mutations. CLI and TUI call the same
backend. The native chezmoi CLI owns managed-path mapping, rendering, status,
diffs, capture, apply and forget. System Git retains existing authentication.

| Module | Responsibility |
|---|---|
| `config` | Local policy, paths, identity, atomic config writes |
| `process` | Argument-vector execution without shell interpolation |
| `backend` | Native chezmoi adapter, managed destinations, folder inventory |
| `git` | Porcelain parsing, actual index, diffs, explicit push target |
| `operations` | Exact plans, approval/freshness, locks, snapshots, outcomes |
| `settings` | Optional native chezmoi data/helper authoring |
| `sync` | Local sync policy, report branches, user-service definitions |
| `migration` | Previewed legacy conversion and source/state preservation |
| `ui` | Workspace, focused input, contextual actions, review forms |
| `main` | Scriptable CLI using the same modules |

No Go runtime, fallback engine, native template renderer, or arbitrary inverse
configuration parser remains. Source is an ordinary chezmoi repository. Optional
local policy/settings enable additional features without requiring a dot manifest.

## Workspace and input

Status, Files, Git and Machines occupy the left column, with Files receiving most
height. Details occupies the right, with a compact Activity area below. Details
follows the selected context and offers Diff/Source/Rules/Activity. `z` expands the
focused pane. Full-screen forms/reviews use the available area. Narrow terminals
show one focused pane and preserve panel switching and Actions/Help.

Files shows immediate HOME children plus managed paths absent from disk. Directory
rows aggregate descendant status and managed counts; yellow indicates changed
HOME or source descendants. Untracked entries are grey and carry a text marker.
All/Managed/Changed filters and search operate on the current directory. Arrow
navigation and mouse double-click enter folders. A folder selection resolves to
an exact file list before execution; it never enrolls future descendants.

Numbers focus panels, Tab cycles focus, Enter opens details, Escape backs out,
and `[`/`]` cycle only the focused panel's tabs, wrapping at either end. Visible
header tabs share this behavior with mouse clicks; each panel keeps its active tab.
Status offers Overview/Paths, Files All/Managed/Changed, Git Changes/Commits,
Machines Local/Reports, Details Diff/Source/Rules/Activity, and Activity Recent/All.
The active tab carries the focus accent while pane borders remain subdued.
Space selects in Files and stages/unstages in Git. Git `v` supports independent
multi-selection. Focused search and form text consume character keys before global
shortcuts, including q and digits. Source editor supports arrow/Home/End navigation,
paste and Ctrl+S review. Contextual footer hit targets invoke keyboard actions.
Mouse/trackpad support depends on terminal event delivery; native touch APIs are
not part of a terminal application.

## State and operation contract

HOME drift, rendered target state, source working changes, the real Git index,
commits and remote refs are distinct. Remote counts describe the last fetched
state, and machine summaries always show timestamps. No single clean/current flag
stands in for all of them. Unattributed changes do not invent an agent identity.

A plan contains an operation, frozen paths, optional input, preview, creation time,
fingerprint and approval ID. Its identity covers the review. Fingerprints cover
source bytes/modes, selected HOME objects, native/local config, Git HEAD/upstream,
index and effective Git config. Execution recomputes identity/freshness under the
same local/source locks. Empty selections are rejected for file operations.

This guards observable changes; it is not an atomic filesystem transaction or a
proof that arbitrary template evaluations are deterministic. Native scripts/hooks
can have effects outside the selected file list. External tools may bypass dot's
locks. Dot-owned TUI, CLI and background writes share serialization.

| Action | Behavior |
|---|---|
| Capture | `chezmoi add` exact HOME objects, nonrecursive, secret scan errors |
| Apply | Native apply exact destinations, nonrecursive, error on conflict |
| Restore | Explicit reviewed replacement of selected HOME from native target |
| Forget | Native forget from shared source, retains HOME |
| Enable/disable | Incremental local automatic-update scope only |
| Edit source | Reviewed exact source edit; native chezmoi renders it afterward |
| Stage/unstage | Actual Git index; selected explicit paths |
| Discard | Restore source worktree from HEAD, preserving index; baseline labeled |
| Commit | Explicit message; all actual staged changes are reviewed |
| Fetch/pull | Explicit fetch; clean fast-forward pull, no autostash/rebase |
| Push | Explicit current HEAD to configured upstream branch, not wildcard push |

Capturing an existing template or modify source is refused so rendered content
cannot silently flatten its behavior. Use source editing or an explicit mapped
setting. Existing Git work and configuration publication remain visible and
separate from capture/application.

## Backup, outcome and recovery

Before reversible file operations, preserve source snapshots and selected HOME objects in private
state. Persist the recovery inventory before mutation. Return updated, failed and
not-attempted targets; stop on first file-operation failure. Activity records
running and final outcomes. A source write may succeed before a later target fails.

Supported file recovery requires the state to match the recorded post-operation
fingerprint. Intervening human changes block automatic restoration. Interrupted
operations retain snapshots for manual inspection because their exact completion
state is unknown. Git history/remote effects, user-manager installs and arbitrary
scripts are not claimed to be reversible by file backups.

## Shared configuration

Optional settings live in `.chezmoidata.json` under `dot_settings`, with a native
`.chezmoitemplates/dot-setting` helper. Shared defaults are overridden by OS, then
chezmoi hostname. Dot's display label is independent of the native hostname key.
A `.tmpl` explicitly invokes the helper at the appropriate location. This enables
common font-size changes without teaching dot a Ghostty/TOML/INI line-merging engine.

Values are strings and templates own format-specific quoting. Existing custom
helpers are preserved rather than overwritten. Removing an override restores
inheritance. Arbitrary templates remain native source; structured provenance is
available only for declared mappings, not guessed from arbitrary HOME edits.

## Automation and reports

Automatic sync is a separate local opt-in and operates only on enabled paths.
Default interval is 300 seconds with bounded exponential backoff to an hour.
No missed-interval catch-up burst occurs. The worker reloads local policy each run.
Dirty source or differing enabled HOME destinations pause before pull. Pull must
fast-forward; removed/missing enabled targets pause for explicit review.

Strict mode checks source/config before commands that evaluate source. Its current
supported subset is regular-file updates using the verified chezmoi version with
no templates, hooks, externals, symlinks, exact/remove/modify/run entries or other
special source behavior. Unknown behavior pauses. Trusted repository mode requires
explicit local consent and permits native unattended behavior with broader effects.
A shared source change cannot turn on local trust or reporting.

launchd (macOS) and systemd user services (Linux) are generated from the actual
executable and config path. Tool paths are resolved when configuration is loaded.
Installation and activation require an explicit command or confirmed TUI action.
Service-manager behavior needs host validation and is distinct from unit generation.

Report publishing is separately opt-in, using a stable installation UUID and a
separate `dot-reports/UUID` branch. Only summary revision/time/outcome/counts are
published. Explicit refspecs, separate commit trees and expected-ref checks avoid
modifying the configuration branch/index or overwriting another installation's
branch. Reports have repository visibility and normal Git history retention.
Fetched summaries are cached. Unchanged automatic summaries publish at most an
hourly heartbeat; changed outcomes/revisions can publish immediately. Local apply, report upload and report retrieval
have separate results. Report-only automatic commits/pushes are the narrow exception
to manual configuration publication.

## Agent integration and utilities

`dot init` connects/creates a normal source and installs the small bundled skill
under HOME `.agents/skills/dot/`. Compare the last installed skill hash before
updating; preserve modified/conflicting files and write a proposed update beside
an existing skill. Agent labels can be supplied through `DOT_ACTOR` and `DOT_INTENT`.
Activity records omit config values and raw command output, retaining 30 days by
default. They are self-reported operational context, not authenticated authorship.

Global Actions exposes Settings, local policy, service setup, package-manager
installation, source switching, shared settings, reports, history and recovery.
Package authentication runs outside raw terminal mode. Source switching archives
the old checkout/config before connecting a distinct repository; HOME stays intact,
and local scope/trust is reset. The original checkout is retained.

## Legacy conversion and distribution

Legacy migration previews convertible entries and held cases. Preserve old source,
config and available local state before conversion into a new directory. Ordinary
unrestricted plain entries can convert through native chezmoi add against a temporary
HOME. OS/machine constraints, custom templates, partial `.gitconfig`, logical maps
and unusual effects remain held for explicit translation. No implicit live Apply
or reconstruction of another machine's local enrollment occurs.

Cargo replaces Go build/test tooling. Native release jobs target Darwin ARM64,
Linux x86-64 and Linux ARM64. Python packaging (build-time only) verifies the retained
chezmoi v2.73.1 archive/binary/license pins, matches the binary/tag version, and emits
bundle checksums. End users need the bundle and system Git, not Rust/Python.
The old Go installer/updater protocol is retired; verified bundle replacement is
the distribution upgrade path. No release is automatically published by this rewrite.

## Acceptance traceability

| Defect | Required coverage |
|---|---|
| BUG-01 | Selected restore preserves unrelated enabled paths |
| BUG-02 | Files/Git selections remain independent; navigation performs no mutation |
| BUG-03 | Native forward/reverse diffs match apply/capture bytes |
| BUG-04 | Actual q/digit/shortcut key events remain text during search/forms |
| BUG-05 | Applying and partial outcomes accurately identify writes |
| BUG-06 | Mutation/remote completion refreshes inventory and invalidates reviews |
| BUG-07 | Utilities are reachable from global Actions; contextual keys avoid collision |
| BUG-08 | Footer mouse targets and keyboard handlers invoke the same actions |
| UX-01 | Capture → stage → commit → push exists with explicit boundaries |
| UX-02 | Git branch/index/outgoing state remains distinct from HOME drift |
| UX-03 | Archive/switch completes with truthful backup feedback |
| UX-04 | Persistent workspace, folder hierarchy, detail area, usable narrow layout |

Verification must include native fixtures, real Git remotes, Ratatui buffer/event
checks and a real PTY journey. Test pass evidence is recorded separately in
`rust-acceptance.md`; historical Go evidence and cross-compilation do not prove
native Rust runtime acceptance on Linux.
