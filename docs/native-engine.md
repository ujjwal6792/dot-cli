# Native engine handoff (#16 → #17/#18)

`internal/native` is the shared, in-process policy package. `native.New(Config)`
requires explicit absolute, non-symlink `Source`, `Home`, and `State` paths;
`OS` is `darwin` or `linux`. Setup must canonicalize system-level path aliases
(for example macOS `/var` → `/private/var`) before constructing it. `Git`
defaults to system `git`. `TrackerConfig` and `Chezmoi` are explicit optional
adapter paths; private scripts are never policy adapters.

## Setup owner (#17)

- Reuse exported `Contract`, `VersionRange`, `Manifest`, `Entry`, `Assignment`,
  `Machine`, `MachineInfo`, `Adoption`, and `Config`; do not duplicate schemas.
- `CheckContract(ctx)` checks the Git checkout root, protocol/API 1, manifest
  1/2, approval 2/3 capabilities and bounded private paths without mutation.
- `SaveIdentity(Machine)` creates only adoption v3 with an empty exact scope,
  `profile: unapproved`. Existing identity is immutable; malformed/legacy
  records fail closed. `LoadAdoption()` is the identity source of truth.
- Source provisioning, connection persistence, machine suggestions/catalog
  selection, dependencies and tracker creation remain #17's responsibility.
- A setup engine can embed `*tui.NativeEngine`, adding the retained
  `SourceEngine`, `MachineEngine` and setup/dependency interfaces. Native policy
  does not import TUI or setup packages, avoiding a dependency cycle.

## Router/distribution owner (#18)

- `Plan(ctx, Request)` → typed `Plan`; `Preview(ctx, Plan)` freshness checks;
  `Apply(ctx, Plan, Consent)` requires the exact `Plan.ID` plus separate gates.
  Serialize using JSON; use `native.Decode` for strict request/plan input.
- Modes: `track`, `adopt`, `untrack`, `update`; `metadata` is the logical Track
  editor (repository changes only, never implicit adoption). Adopt `Entries == nil` resolves
  named-profile unions; `Entries: []string{}` is an explicitly empty scope.
  Update never enrolls new entries or mapping/capability changes.
- `Status(ctx)`, `Diff(ctx)`, `ScopeShow(ctx)` return machine-scoped JSON-ready
  values. They and contract/inventory/preview are offline and read-only.
- `Pull(ctx, true)` / `Push(ctx, true)` are explicit remote operations. Pull is
  fast-forward-only and refuses dirty/untracked work. Neither applies, commits,
  resets, stashes or force-pushes. Push sends already-existing commits only.
- `Recovery()` inspects the private journal; `Recover(ctx, exactPlanID)` rolls
  back only reviewed actions and refuses post-interruption conflicts.
- `tui.NewNativeEngine(engine)` implements the retained real browser seam.
  `tui.NewReturningModel(adapter)` adds returning-user navigation and separately
  confirmed pull/push. Routing, CLI flags and packaged acceptance remain #18.

Public plans carry exact bytes/modes for ordinary reviewed files; bounded Git
actions carry only approved `user.name`/`user.email` values and whole-file hashes,
not unrelated target credentials. Native bounded Git entries use declarative
`git_values`; a reviewed Track operation imports only those fields, retaining
other already-approved fields. Declarative `git_source` templates can also supply
bounded values. Legacy private helper templates are never executed. Declarative
templates use Go's standard `text/template` functions, machine/OS data and scalar
quoted-string `[data]` fields from the explicit tracker config, with no
command/network functions. Unsupported data/template constructs fail explicitly.
