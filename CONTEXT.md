# Native dot context

## Domain language

[Machine](GLOSSARY.md#language) is the target-local identity, owned by adoption v3
(`machine: {id, os}`). Suggested ids use `macos_user` on Darwin; applicability OS
remains `darwin` or `linux`. Identity is immutable once saved.

[Machine-assignment](GLOSSARY.md#language) is shared eligibility, not local
authorization: `machines: {include: ["all"], exclude: []}`. Include is nonempty,
exclusions win, and local deselections/customs belong to adoption, not the manifest.
Named profiles compose a union at reviewed Adopt time; Update uses exact prior scope.

## Boundaries

`cmd/dot` is the sole Go application. `internal/setup` and `internal/native` own
effects; `internal/tui` is their view adapter. Bubble Tea v2, Bubbles, Lipgloss and
the embedded Huh machine form are compiled libraries, not separate prompt apps.
Git uses system/user configuration; release chezmoi is privately pinned and verified.
Private executable templates/helpers remain unsupported and fail closed.

When changing source/identity/tracker/dependencies, read [native setup](docs/native-setup.md).
When changing policy/JSON/recovery, read [native engine](docs/native-engine.md) and
[assignments](docs/machine-assignments.md). For package trust/ownership/promotion,
read [distribution](docs/native-distribution.md). For readiness, rerun and record
[artifact acceptance](docs/native-acceptance.md), not just model tests.

## Decisions and scope

Four [ADRs](docs/adr/) record the single app, pinned chezmoi/system Git, retired
personal/work tags and assignment schema. Spec #10 subsumes #7/#8/#9; issue
closure remains the reviewer's responsibility. Only the existing v-tag publisher
is automated. Linux runtime execution is owner-waived; never describe cross-build
verification as a Linux runtime pass. No privileged host manager acceptance.
