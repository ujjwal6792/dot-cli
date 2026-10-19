# Chezmoi overlap and engine ownership

> Historical Go baseline assessment. The accepted Rust rewrite now delegates file
> management to native chezmoi; see [the current spec](rust-rewrite-spec.md) and
> [verification](rust-acceptance.md). The analysis below preserves the design evidence.

Investigated 2026-10-19 against dot baseline `20096ca` and current official chezmoi
documentation. This is factual input to the design interview, not an accepted
engine migration. Dot currently pins chezmoi v2.73.1; any proposed adapter must
verify its exact command/format guarantees against that release.

## Current runtime boundary

Dot requires and verifies chezmoi, but the native Go engine performs import,
template rendering, comparison and application itself. A search of production
`cmd` and `internal` found no chezmoi add/re-add/apply/diff/status/template-command
invocations. Dependency/health checks execute `--version`; release tooling bundles
the executable; setup writes and validates a chezmoi-shaped tracker configuration.

Evidence: `internal/distribution/runtime.go:21`, `:61`;
`internal/setup/dependencies.go:75`; `internal/setup/tracker.go:90`;
`cmd/dot/main.go:200`, `:378`.

## Overlap, with semantic differences

| Responsibility | Existing chezmoi capability | Dot implementation |
|---|---|---|
| Import HOME files and later edits | [add](https://www.chezmoi.io/reference/commands/add/), [re-add](https://www.chezmoi.io/reference/commands/re-add/) | Track reads bytes/modes and writes source plus manifest (`internal/native/track.go:138`, `:257`). Not full source-attribute/template-preservation parity. |
| Render and apply desired files | [apply](https://www.chezmoi.io/reference/commands/apply/) supports selected targets and prompts on edits since last write | Adopt/Update build HOME writes and removals (`internal/native/plan.go:202`, `:222`, `:569`). Dot's enrollment and plan authorization are additional semantics. |
| Inventory and compare | [status](https://www.chezmoi.io/reference/commands/status/), [diff](https://www.chezmoi.io/reference/commands/diff/) | Manifest inventory, digest comparison and a custom line-diff implementation (`internal/native/inspection.go:40`, `:254`; `internal/native/plan.go:360`). Chezmoi status also distinguishes last-written→actual from actual→desired. |
| Templates and data | [Go templates with Sprig, template variables, layered data and helper functions](https://www.chezmoi.io/user-guide/templating/) | Direct standard Go text/template and a small data model; tracker parsing accepts scalar quoted strings (`internal/native/content.go:14`, `:49`). |
| Machine/OS differences | [Conditional templates, ignored paths, shared templates and separate included variant files](https://www.chezmoi.io/user-guide/manage-machine-to-machine-differences/) | Explicit logical sources/destinations, common→machine→OS overrides and assignment filtering (`internal/native/logical.go:8`, `internal/native/engine.go:179`). Similar user goal, different representation. |
| File attributes and removals | [Source attributes](https://www.chezmoi.io/reference/source-state-attributes/), [target removal and partial-file management](https://www.chezmoi.io/user-guide/manage-different-types-of-file/) | Custom regular-file writes, permission handling, parents and exact removals; rejects symlinks/nonregular files (`internal/native/storage.go:190`, `:218`). |
| Stop managing | [forget](https://www.chezmoi.io/reference/commands/forget/) removes source-state entries | Untrack removes manifest/profile mappings but retains source and HOME bytes (`internal/native/track.go:71`). These operations are not interchangeable. |

The prior Ghostty request—different macOS/Linux contents installed at the same
destination—is already possible in chezmoi through conditional templates that
include separate variant files. Dot adds explicit manifest-visible variant identity.

## Dot-specific value and current limits

The distinctive layer is structured logical-file metadata, shared eligibility
versus exact local enrollment, profile composition, selected-operation freshness,
recovery journals, bounded Git identity fields, and the unified TUI/Git workflow.
These should not be conflated with the duplicated import/render/diff/write engine.
Some underlying goals can also be expressed through chezmoi templates/configuration;
the distinction is dot's explicit model and enforced workflow, not impossibility
of achieving similar outcomes with chezmoi.

Chezmoi additionally documents password-manager integrations, encryption,
externals, richer template data/functions and executable scripts. These are not
automatically supported by dot merely because chezmoi is bundled:

- [Templates and runtime data](https://www.chezmoi.io/user-guide/templating/)
- [Encryption](https://www.chezmoi.io/user-guide/encryption/)
- [External content](https://www.chezmoi.io/user-guide/include-files-from-elsewhere/)
- [Scripts](https://www.chezmoi.io/user-guide/use-scripts-to-perform-actions/)

## Revised open question

The original feature-scope question was incomplete: should dot continue owning its
file engine, or should chezmoi own ordinary file/source/rendering semantics while
dot owns policy, logical-file presentation and Git interactions?

Recommendation for user decision: prefer a bounded chezmoi-backed integration for
the overlapping mechanics, subject to proving selected-only effects, preview/apply
agreement, enrollment filtering, recovery and migration from today's source layout.
Do not assume the manifest format is already a standard chezmoi source tree.
Do not imply all chezmoi features become enabled merely by selecting this direction.

The upstream [building-on-top guidance](https://www.chezmoi.io/developer-guide/building-on-top-of-chezmoi/)
recommends invoking the semantically versioned CLI, with dump/state for inspection.
That supports a Go adapter around the binary; it does not require a second UI.

The user accepted the chezmoi-backed direction after this comparison; see
[ADR 0006](adr/0006-chezmoi-owns-file-management.md). Integration, migration and
supported capabilities still require specification. UI and daily Git scope
decisions are recorded in [the interview](tui-redesign-discovery.md) and ADR 0005.
