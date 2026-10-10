# dots-cli

Public reusable `dot` CLI, safety engine, prebuilt configuration browser and
agent interaction skills for `ujjwal6792/dots-cli`. The private source of truth
is `ujjwal6792/dotfiles`; no private manifests, configuration, credentials,
history, machine inventories, approvals or recovery backups belong here.

This is a staged review candidate, not a claim that a new release is published.
The owner replaced the former `dots-installer` with independent `dots-cli` history;
no bootstrap or export command deletes or publishes repositories.

## Install the public CLI

Releases contain complete `dot-cli-VERSION-PLATFORM.tar.gz` archives for
`darwin-arm64`, `linux-amd64`, and `linux-arm64`, including the prebuilt browser
and Python engine. Go is not required on the target. Python is needed only for
engine commands; `dot --help` and `dot --version` work without Python or private
configuration. The installer does not install dependencies or create a chezmoi tracker;
it never installs Python or other dependencies.

Use an immutable release URL and a checksum reviewed independently from the
archive. The checksum provides integrity relative to that reviewed value, not
publisher attestation:

```sh
curl --fail --location --proto '=https' --proto-redir '=https' --tlsv1.2 \
  'https://raw.githubusercontent.com/ujjwal6792/dots-cli/REVIEWED_COMMIT/install.sh' \
  -o install.sh
less install.sh
bash install.sh --version VERSION --platform PLATFORM \
  --url 'https://github.com/ujjwal6792/dots-cli/releases/download/vVERSION/dot-cli-VERSION-PLATFORM.tar.gz' \
  --sha256 REVIEWED_ARCHIVE_SHA256
```

Default locations are `~/.local/share/dot-cli` (versioned payload and atomic
`current` pointer), `~/.local/bin/dot`, and
`${XDG_STATE_HOME:-~/.local/state}/dot/install.json`. The mode-0600 local record
uses schema `record_version: 1`: `owner` (`artifact/curl` or
`package-manager/brew|pacman`), release repository/tag/platform/archive SHA-256,
canonical `install_root` and `engine_path`, plus `browser_sha256`. Optional
private/config connection lives separately in `connection.json` (see the
[shared state contract](docs/install-record-contract.md)); neither checkout is
needed to install or identify the public artifact. These are local mutable
ownership/recovery records, not trust roots.
The installer does not edit shell startup files; add `~/.local/bin` to PATH
manually. `--root` and `--state-dir` accept absolute paths, including spaces.
Package-manager-owned installs are never overwritten; use `brew upgrade` or
`pacman -Syu` instead. No formula/AUR package is published by this project yet.

The install is public tooling only: no private Git clone, chezmoi config/tracker,
dependency or application installation, live configuration adoption, or startup
file edits. Private config is connected only through `dot init`.

## Download, review, execute (legacy config bootstrap)

```sh
curl --fail --location --proto '=https' --proto-redir '=https' --tlsv1.2 \
  "$APPROVED_BOOTSTRAP_URL" -o bootstrap.sh
printf '%s  bootstrap.sh\n' "$APPROVED_BOOTSTRAP_SHA256" | shasum -a 256 -c -
less bootstrap.sh
bash bootstrap.sh --help
```

Use immutable reviewed platform asset URLs and independently reviewed SHA-256
values. Compare checksums before executing downloaded code. A checksum fetched
beside an untrusted artifact is not publisher identity/provenance. Interactive
questions use `/dev/tty`; `--yes` is explicit installer consent only. This legacy
bootstrap remains for existing Git/config installations. New public CLI installs
should use `install.sh` above. It does not install dependencies or create a
chezmoi tracker (the installer does not install dependencies or create a
chezmoi tracker either).

## Defaults and identity

- Public checkout: `~/.local/share/dot-cli` (`--engine-directory ABSOLUTE_PATH`).
- Private checkout: `~/.local/share/dot-config` (`--directory ABSOLUTE_PATH`).
- Launcher: `~/.local/bin/dot`; add this directory to PATH yourself if needed.
  Bootstrap leaves Bash/Zsh startup bytes unchanged. Custom absolute `ZDOTDIR`
  is detected safely; startup code is not run.
- `DOT_SOURCE` remains private; `DOT_ENGINE` resolves public `tools/dot`.
  Private/public descriptor compatibility is checked before configuration mutation.
- Local metadata: `~/.local/state/dot/`. Initialized chezmoi machine data and
  exact approvals stay local-unmanaged, never in public release artifacts.
- Private API identity/visibility and Git access are verified before retrieval.
  Denied access never creates a repository or switches the private source.
- Existing matching checkouts are detected read-only; neither checkout is
  fetched or updated by bootstrap (including legacy `--existing update|keep`).
  Dirty/unrelated checkouts and symlink/path conflicts are never reset or overwritten.
- Legacy co-located installs require separate approved recoverable migration,
  not silent replacement. Current development checkouts are not moved.

## Legacy installed split: dot migrate

From the reviewed CLI, with explicit legacy installed `DOT_SOURCE` and tracker
`DOT_CONFIG`, run `dot migrate plan --engine /reviewed/public-checkout --out /private/new-plan.json`.
Review the preview and private plan, then `dot migrate apply /private/new-plan.json --approve EXACT_PLAN_ID`.
`--engine` names an independent reviewed Git checkout, not a private-history
extraction or fetch instruction. Only a matching version-1 installation record
under `HOME/.local/share` and recognized legacy launcher are eligible.

Snapshot verification preserves private history/index/dirty/ignored work, modes,
symlinks, identity, exact approval bytes and unrelated local recovery/config.
Old checkouts remain; the new public/private checkouts use the defaults above.
Tracker source/engine references, launcher and version-2 record are journaled.
Nothing is adopted/applied/provisioned. Concurrent editing is unsupported.
Shared/linked Git metadata and path-conditioned identity changes fail closed.

An incomplete `~/.local/state/dot/installed-migration.json` refuses unsafe retries.
Inspect its immutable `installed-recovery/PLAN_ID/README` and verified snapshots;
manually restore original references with their modes, verify old work/scope,
and retain/quarantine new destinations and recovery data before a fresh plan.
At the snapshotting stage, originals remain authoritative: never restore an
unverified partial backup. Do not delete/reset work or rerun bootstrap as repair.

## Independent updates

### Command quick reference

| Command | Purpose |
| --- | --- |
| `dot init`, `dot init --check` | Consented first-run setup; read-only installed-path diagnostics |
| `dot browse` | Dotfiles Track/Adopt plus visible Packages; no navigation side effects |
| `dot status`, `dot diff` | Exact scope/drift and direction-labeled changes, not whole-machine parity |
| `dot intent plan/preview/apply` | Shared exact-plan engine for human/agent review and approval |
| `dot self-update` | Public tooling review; `--yes` approves its fast-forward only |
| `dot pull` | Private exact-scope review; `--yes` approves that private update only |
| `dot update NAME VERSION` | Legacy private application pin/installer contract, not self-update |
| `dot migrate plan/apply` | Separately approved, recoverable installed-checkout split |
| `dot doctor`, `dot contract check` | Actual tracker/private adapters and compatibility diagnostics |

See [private contract and adapter diagnostics](docs/private-contract.md).
No private package manifests are bundled; Packages is a browser view, not a
blanket provision command. Stop tracking uses a reviewed `intent` untrack plan;
it removes private source/scope entries while retaining live bytes and modes.

For artifact installs, `dot self-update --version VERSION --sha256 REVIEWED_SHA256`
downloads and verifies the requested immutable release, checks a present private
connection read-only, and previews the public tooling switch. Add `--yes` to
promote it atomically. It never applies configuration, runs private hooks, changes
approvals or invokes package managers, and needs no private connection when none
is configured. Existing Git-checkout installs retain their reviewed fast-forward
path (`--yes` explicitly applies that tooling-only update).

The installed `bin/dot-browser` is exempt from the dirty-check **only** when its
regular-file bytes/mode match the matching version-2 local installation record.
Unrecorded/forged artifacts, symlinks, tracked edits and unrelated untracked or
ignored files still stop the update; `bin/` is never blanket-ignored. A tooling-only
update preserves the verified browser. If browser source changes, independently
review the release artifact and supply `--browser-url HTTPS_OR_ABSOLUTE_PATH
--browser-sha256 REVIEWED_SHA256` to both preview and consented self-update.
Checksum/compatibility validation precedes mutation. Artifact promotion updates
the local recorded hash; an ordinary promotion failure restores previous browser
bytes/record and the transaction's tooling HEAD when no concurrent edits exist.
An interrupted operation or concurrent editing requires inspection, not a reset.
No approved asset URL or release provenance is supplied by this staging candidate.

`dot pull` previews private upstream changes against the existing exact approved
scope; `--yes` explicitly fast-forwards the private checkout and applies only
unchanged approved entries/capabilities. New entries stay opt-in. Changed approved
metadata requires fresh adoption, not a broader pull. Legacy/unscoped approvals,
dirty (including ignored local files), ahead/diverged, inaccessible and incompatible
updates stop without reset. Public tooling is never updated by pull. Preview uses
bounded Git keys rather than raw credential-bearing source diffs; private hooks
and checkout filters are not run. `dot update NAME VERSION` retains package-pin
semantics; it is not tooling maintenance.

Both update commands use the configured remote branch upstream; review/trust the
public distribution independently. External concurrent editing is unsupported.
Late target I/O failures can leave private source advanced: inspect the shared
engine's local recovery instead of resetting work or retrying blindly.

## First-run setup: dot init

`dot init` (#30) owns dependency installation and chezmoi tracker
setup/create-or-detect. Each install/create operation must show its operation and
obtain informed consent. Detection is read-only: it does not create, repair,
switch, adopt or expand scope. Use `dot init --check` for offline read-only
record/origin/contract/tracker identity detection. Conflicts stop with actual
source/config diagnostics; machine data is preserved. Bootstrap is not a repair
or creation fallback.
Dotfiles Track/Adopt, self-update and migration have no package-install or
tracker-creation effects. Packages is the browser's separately gated install
entrypoint below. `--replace-chezmoi` is not a bootstrap repair/retirement shortcut.

After initialization, init offers **Finish** (default), `packages` (separately
reviewed Install missing with exact plan-ID consent), and `rounds` (prebuilt
Track/Adopt browser). `dot init --rounds` enters optional rounds after setup;
`--profile personal|work` chooses only new tracker machine data, not adoption scope.
Init has no blanket `--yes`. Each dependency gets its own reviewed manager
operation and default-No consent. Tracker creation separately previews the
complete config with actual `home/`, HOME, engine, cache/state paths and preserved
Git identity; private templates/hooks never execute during creation.

Separate optional Track and Adopt rounds start with no
entries selected and **Finish is the default**. Bootstrap `--yes` skips rounds; installing
tooling does not approve configuration. Empty adoption scope, including zero
managed drift, is not whole-machine parity. Unsafe/desktop effects require
separate acknowledgments. Package pins/application setup are separate tasks.
Missing tools used by configs are advisory, not permission to install them while browsing.

In either Track or Adopt, Space on a folder selects an exact snapshot of current
eligible files; `h/l` independently collapse/expand it. `[x]` means all eligible
files are selected, `[-]` means only some. Adopt stores exact current entry IDs,
including templates/Unsafe entries with unchanged separate consent gates. Future
entries remain opt-in. Search, resizing and preview scrolling retain the basket.
Adopt folder selection/counts include only entries the shared engine reports as
applicable to the current OS and explicit machine. Inapplicable entries remain
visible for navigation and metadata editing; a folder never implicitly opts them in.

## Packages beside Dotfiles

`P` switches between Dotfiles and Packages in `dot browse`. Packages reads the
private checkout's `packages/Brewfile` (macOS), `packages/pacman.txt` and
`packages/yay.txt` (Arch), or `packages/apt.txt` (Ubuntu). It displays desired,
installed and missing status. `m` chooses the source, `e` edits it, `L` imports
a supplied file from inside the private checkout, and `Ctrl+S` saves with a
private byte-preserving backup. `r` refreshes status. Lists/inventories never
belong in the public engine or release payload.

Editing, importing, saving, navigation and deselection never install or
uninstall. `i` explicitly reviews **Install missing**; final consent defaults to
Cancel. Approval binds an exact plan ID and fresh source/manager/state/resolution
snapshots. Config approvals cannot install packages. AUR builds require an
additional explicit risk acknowledgment, run unprivileged, and retain the
helper's build-script/dependency prompts. Missing managers/sudo never trigger
a fallback installer. Package names need not be version-pinned; legacy pin
updates remain separate.

Package installation has exactly two independently reviewed entrypoints:
Packages **Install missing** and `dot init` (#30). Native manager confirmations
remain interactive; resolution snapshots are not repository locks. Inspect the
final dependency transaction and AUR scripts before confirming. No real package
install or native Linux acceptance is claimed by the fixture tests.

## Platforms and prebuilt release packages

- **macOS:** native Apple Silicon arm64 macOS 15+; no Intel/Rosetta.
- **Ubuntu:** native amd64/x86_64 or arm64/aarch64 24.04+; 22.04 needs existing
  Python 3.11+ with tomllib.
- **Arch:** official Arch x86_64; no ARM/derivatives. Bootstrap/prerequisite
  provisioning does not use AUR; private Packages lists may explicitly select
  AUR-only yay targets with the review/consent gates above. If init proposes missing
  dependencies, its official pacman route must disclose a **full system upgrade**
  before informed consent. Bootstrap never executes that transaction.

Release assets are `darwin-arm64`, `linux-amd64` (Ubuntu/Arch), and `linux-arm64`
(Ubuntu). Targets do not need Go. Existing Bash, Git, curl, GitHub CLI, Python
3.11+ are bootstrap prerequisites; chezmoi and Gum provisioning belongs to init.
Unsupported/missing prerequisites fail with guidance. GitHub CLI 2.45/2.46 is incompatible. Native platform smoke acceptance
is distinct from fixture-only mocked package-manager tests.

## Recovery and agent skills

Conflicts or interrupted operations stop with recovery guidance rather than
resetting checkouts or deleting state. Canonical skills use a bounded singleton
`skills.bak`; other backups and private history are not public payload.
Review `skills/setup-dots/SKILL.md` for installation/init boundaries and
`skills/dot/SKILL.md` for read-only interaction, exact plans and explicit approval.
