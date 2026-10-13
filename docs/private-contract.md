# Compatible private source contract

The public engine is not a configuration catalog. Supply a separately trusted
private Git checkout, its dedicated `home/`, and a private `dot-contract.json`.
`DOT_SOURCE` is that checkout root; `DOT_ENGINE` is public `tools/dot`.
`DOT_CONFIG` is the actual initialized chezmoi config, whose `[data].dotEngine`
must match the public engine. The launcher records explicit paths; no adapter
may infer an engine from a private source parent or fall back to unscoped apply.

Protocol and engine API are currently 1. `engine.json` declares supported
manifest versions 1/2 and exact approval versions 2/3. The private descriptor
declares `protocol`, `engine_api` (`min`/`max`), `manifest_version`,
`approval_version`, `manifest`, `home_source`, and `required_adapters`.
Manifest paths must remain inside the private checkout. Validate with:

```sh
dot contract check --engine "$DOT_ENGINE" --source "$DOT_SOURCE"
dot init --check
```

The init TUI connects a credential-free HTTPS or SSH source using system Git.
Existing users clone, then verify the configured origin and contract. New users
create an empty repository on their own hosting platform; dot only initializes
local Git, adds origin, and writes an empty declarative contract/manifest. It
does not create a hosting repository, commit, push, configure the tracker, or
adopt files during source setup. URL input is hidden and cleared on submission;
credentials, query parameters, and fragments are rejected without echoing them.

Existing local directories offer Continue or Replace. Replace requires a second
confirmation displaying the exact timestamped backup path, defaults to Cancel,
and moves the entire checkout (including dirty/untracked files and `.git`) into
an exclusively created sibling `dot-config.backup-<UTC timestamp>/checkout`.
Nothing is deleted or overwritten. Failed clone/contract checks retain their
checkout for inspection; retry through Continue or backup-first Replace.

Required public adapters are `intent`, `scope`, `gitconfig`, `skills`, and
`layout`, mapped to Python modules by `engine.json`. Private native ignore,
removal, bounded Git and skills controls must resolve this same explicit engine
and exact local approval. Missing/incompatible adapters refuse before mutation.
Private templates must not execute arbitrary helpers during browser previews.
Source metadata v2 compiles common/machine/OS destinations into exact approval
v3; updates preserve existing machine binding, bytes, modes and capabilities.
Do not turn legacy group approval into a wider exact approval implicitly.

Adopt browser inventory retains repository-backed entries as `managed` even when
they are not applicable. Its engine-resolved `os_status: supported` means both
current-OS and explicit-machine applicability passed; `unsupported` means one did
not. Folder snapshots and eligible/partial counts consume that result, not raw
`managed` status or a frontend reimplementation of OS/machine policy. Inventory
is still not authorization: exact planning, freshness and separate Unsafe/desktop
consent remain mandatory. Inapplicable entries stay navigable/editable individually.

Private manifests/profiles, package lists, machine inventory and native controls
are supplied by the private repository, never shipped by this public checkout.
Packages reads only supported private Brewfile/pacman/yay/apt declarations;
unsupported Brewfile Ruby is diagnosed as data, not evaluated. Navigation,
editing/saving and deselection cause no install or uninstall. Only reviewed
`dot init` or Packages **Install missing** can perform package operations.

Legacy application pins are a separate private contract: `packages/pins.toml`
and reviewed `scripts/install-pins.sh` are needed for `dot update NAME VERSION`
and the pin check in `dot doctor`. Neither file belongs in the public payload.
`dot run NAME` likewise runs an explicitly selected private script, not a
public engine adapter. Missing private pin/script support is an actionable
diagnostic, not permission to download or invent a replacement.

For template failures inspect the actual installed source and config shown by
`dot init --check` and `dot status`. Use `chezmoi --source "$DOT_SOURCE/home"
--config "$DOT_CONFIG" doctor` for that tracker, not a guessed development
checkout. Preserve machine data, approvals and recovery. Do not delete config,
rerun bootstrap as repair, or apply unscoped configuration to clear drift.

Build tools and the manually dispatched public workflow produce review-only
archives; they do not publish tags or releases. Native smoke, provenance and
independent final review remain separate release gates.
