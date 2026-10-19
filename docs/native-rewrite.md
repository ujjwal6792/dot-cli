> Historical Go baseline. The Rust rewrite supersedes this implementation; see [Rust spec](rust-rewrite-spec.md) and [current verification](rust-acceptance.md).

# Native rewrite acceptance

The owner approved deleting the interpreter-based implementation before its
replacement worked. The shared engine, setup, distribution and integrated
acceptance are now native Go. [Final local evidence](native-acceptance.md)
records actual artifact execution and platform limits; do not restore the old
runtime or infer release readiness from retained UI tests alone.

## Architecture and sequence

- `cmd/dot`: the only application executable and command router.
- `internal/tui`: retained Go views, interaction models, and model tests.
- Native domain/engine packages: added by #16 and #17, shared in-process by TUI
  and scriptable commands. Keep policy in the engine, not duplicated in views.
- Runtime dependencies: Git and chezmoi. Their processes are legitimate adapters;
  a Python/shell policy engine or second browser executable is not.
- #16 and #17 implement native behavior and its tests; they can share domain
  contracts but must agree on state ownership. #18 integrates both and packages
  the sole executable. #19 verifies the complete delivered result.

Previous #13–#15 completion describes the deleted implementation. Their behavior
is still required: source and identity belong to #17; assignment and adoption
policy belong to #16. UI code alone does not satisfy those requirements.

## Behavioral coverage to restore in Go

### #16: configuration engine and returning-user operations

- Protocol-1 contract/version validation and private source boundaries; reject
  incompatible capabilities and unknown assignment fields before mutation.
- Inventory versus authorization: applicable OS/machine filtering, exact folder
  snapshots, inapplicable entries visible for navigation, no future enrollment.
- #15 assignments: nonempty include, `all` only wildcard, exclusions win, local
  deselections/customs isolated to the machine, named profiles compose as a union.
- #14 adoption v3 machine binding preserved by plan/apply/update; reject stale or
  conflicting state rather than widening approval or rewriting another machine.
- Track/Adopt/untrack plans, direction-labeled previews, stable exact approval IDs,
  freshness checks, bytes/modes/path constraints, backups and interrupted recovery.
- Separate Unsafe, desktop-effect, and applicable backup confirmations; Cancel
  and Finish defaults, nothing preselected. Unknown gates must not permit apply.
- Bounded Git name/email changes; never wholesale replacement of `.gitconfig`.
- Machine-2 pull/import selection, explicit push, status/diff/scope JSON parity
  with TUI, offline inspection. No automatic apply/commit/push.

### #17: setup, state and dependencies

- HTTPS/SSH source URLs with secret-safe rejection of credentials/query/fragment;
  plain Git clone, origin verification and contract validation; no hosting API.
- New source: local init/remote only. Continue validates; Replace needs a second
  path-naming confirmation and timestamped backup preserving Git/dirty/untracked
  data. Stale backup plans, symlink destinations and collisions are rejected.
- Editable machine suggestion from OS/user; lowercase ids, collision suffixes,
  offline local uniqueness, and machine identity persisted in adoption v3.
- Explicit source/home/config/state paths; custom state directory isolation,
  no hidden reads/writes to another installation, no private executable fallback.
- Welcome → Source → Machine → Dependencies → Tracker → Done. Tracker detection
  is read-only; creation preserves existing machine/Git data and needs consent.
  Failed/skipped required setup cannot be presented as successful completion.
- `dot` and `dot init` share the app; `init --check` is offline/read-only and
  reports old/new user. Dependencies table shows exact operations and links.
- Brew ownership, pacman Enter/Space behavior, native sudo outside raw terminal
  mode. Only explicit dependency installation may invoke platform managers.

### #18: integration and distribution

- Complete top-level and per-verb help before configuration gates; preserve
  scriptability and meaningful exit codes. Required #10 commands work through
  one executable, not mock handlers or a launcher plus browser/engine payload.
- Resolve Git and chezmoi consistently, with clear missing-dependency diagnostics.
  Chezmoi remains a dependency and may be pinned/bundled per #10; no second
  application engine is packaged. No interpreter is needed by installed `dot`.
- Inventory old verbs against #10: retain required behavior natively; retire
  legacy pin runners, arbitrary-script runners, split-engine migration and other
  unrequired compatibility branches rather than porting everything mechanically.
- Replace deleted release tooling with Go build/package/verify tooling. Produce
  darwin-arm64, linux-amd64 and linux-arm64 artifacts, version/checksum metadata,
  appropriate ownership/update behavior, and native packaged smoke tests.
- Automation is release-tag-only: pushing `v*` builds archives/checksums and
  creates a GitHub release. No PR, branch-push, or scheduled CI is wanted.
  Agents perform tests and artifact acceptance locally before pushing/tagging.
  #18 expands the current binary-only release packaging as required by #10.

### #19: integrated completion gate

- Exercise real native engine behavior and isolated filesystem/Git operations,
  not just mock UI results. Recreate end-to-end source, machine assignment,
  approval, cancellation, freshness/recovery and package-manager scenarios.
- Test the extracted artifact independently of this checkout without Python,
  Bash, gum, or gh on runtime PATH. Git and chezmoi remain available; package
  managers are isolated fixtures except in explicit native-platform acceptance.
- Verify offline inspection, read-only setup checks, all supported platforms,
  complete help, documentation, machine vocabulary and the four #10 ADRs.
- Passing current model tests is not a release gate. No issue may claim the
  deleted Python behavior is still implemented merely because #13–#15 once passed.

## Historical evidence

Last committed pre-retirement implementation:
`3ab354fca29999f43d1391fdc56eb798c6820a20`.

Its `tests/` tree records behavioral scenarios for contracts, safe/unsafe
adoption, tracking, state isolation, independent updates, source setup, machine
identity, assignments, installation and releases. The three Python-backed Go
init integration tests were in `tools/dot/browser/init_test.go`; the remaining
pure Go init/model tests were retained in `internal/tui`.

Use those scenarios as evidence when implementing Go acceptance tests, not as
permission to reintroduce obsolete adapters or out-of-scope features. The old
Bats/Python harness and private templates invoked deleted files and were removed
with them. Git history retains the implementation; no duplicate legacy tree is
kept in the current source or release payload.
