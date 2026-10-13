# Native CLI and distribution handoff (#18 → #19)

The final integrated #19 gate and fresh artifact hashes are recorded in
[native acceptance](native-acceptance.md). The #18 evidence below remains
provenance, not the final archive/run after #19's fixes.

## Delivered boundaries

`cmd/dot` is the only application executable. Bare `dot`, `dot init`, `browse`,
and retained interactive aliases use `setup.NewApp` and Bubble Tea's
`tea.WithContext`; there is no subprocess engine or checkout lookup. Scripted
operations use the same `setup.Engine` / `native.Engine` as the app.

Help/version, dependency health and public installation management do not load
private source, tracker, adoption or connection state. Errors exit 1; invalid
usage exits 2; cancellation exits 130. Inspection is offline and read-only.
`status`, `diff` and `scope show` default to JSON and accept `--format json`.
Diff includes the machine identity and its machine-filtered diffs. An unsupported
format/flag is an error, not a successful no-op.

Common path inputs are those documented in [native-setup.md](native-setup.md).
Commands put options **after** the verb/subcommand. The default source really is
inside HOME; Track excludes source, state and tracker paths. Neither development
nor packaged binaries search the working directory for an engine.

### Scriptable setup and intent

```text
dot init --check
dot init --action new --url https://host.example/user/config.git
dot init --action clone --url git@host.example:user/config.git
dot init --machine macos_user
dot init --tracker-plan --name 'User Name' --email user@example.invalid
dot init --tracker-apply tracker-plan.json --approve EXACT_ID --yes
dot init --complete
dot contract check
dot intent inventory --mode track --format json
dot intent plan --request request.json
dot intent preview --plan plan.json
dot intent apply --plan plan.json --approve EXACT_ID
dot pull --yes
dot push --yes
dot recovery
dot recover --approve EXACT_PLAN_ID
```

Plans/requests accept `-` for stdin. JSON is strictly decoded and bounded to
32 MiB. The plan is the engine's exact serialized plan, not a router-generated
approval summary. Required `--ack-unsafe`, `--ack-desktop`, `--ack-backup` gates
are independent. Request modes include `track`, `adopt`, `untrack`, `update`,
`metadata`; profile unions and machine-local deselections/customs remain the
shared engine's policy. An explicit URL creates/clones source only; it does not
authorize machine/tracker creation or adoption. Replace/backup and dependency
installation remain real reviewed app operations, not hidden script flags.

### Historical command inventory

Compared with `tools/dot/dot` at
`3ab354fca29999f43d1391fdc56eb798c6820a20`:

| Surface | Native disposition |
| --- | --- |
| bare dot, init, browse | Retained, one common complete app |
| -h/--help/help; -v/--version/version | Retained before private gates, per-verb help |
| -u/--uninstall; --health | Retained; uninstall requires owned root and consent; health checks actual dependencies |
| doctor | Alias of health; obsolete pin/secret-script execution removed |
| init --check, --source/--config/--name/--email | Retained; explicit HOME/state/Git paths also supported |
| init --url, --rounds | URL now needs explicit --action; rounds is redundant and rejected |
| status, diff, scope show | Machine-scoped offline JSON; no unscoped chezmoi invocation |
| diff --direction machine-to-repo | Retired flag; explicit Track intent previews this direction |
| pull | Explicit --yes, clean fast-forward-only; never applies |
| push --message TEXT | Auto-re-add/commit removed; push --yes sends existing commits only |
| self-update | Verified local prebuilt archive promotion; never private-source pull/apply |
| contract check | Retained shared native protocol/manifest validation |
| intent rounds/pick | Aliases of the same app, not separate pickers; legacy --mode flags retired |
| intent inventory PATH | Replaced by full machine-scoped inventory --mode track/adopt/untrack |
| intent plan REQUEST --out PLAN, preview PLAN, apply PLAN | Retained effects; explicit --request/--plan inputs and stdout JSON replace positional/output syntax |
| intent update --yes | No automatic apply; request mode update still needs plan/preview/exact apply |
| add/remove | Retired direct mutations; exact Track/untrack plans preserve required effects |
| update NAME VERSION, run NAME | Retired pin/arbitrary-script runners |
| scope init/catalog/expand GROUP | Legacy group runners retired; status/inventory expose machine/profile catalog, Adopt composes named profiles |
| layout plan/apply, migrate | Split-engine/layout and legacy group migration retired; use reviewed backup-first Replace |
| personal/work --profile; DOT_ENGINE/DOT_BROWSER/DOT_PYTHON | Retired, no migration or interpreter fallback |

## Dependency and archive trust

System Git is resolved once and passed to all setup/policy operations. Development
builds use system/explicit chezmoi **v2.73.1**, checking its version; incompatible
existing tools appear in the setup dependency table and are never overwritten.
Packaged builds require adjacent verified `release.json` and `libexec/chezmoi`.
They cannot fall back to Homebrew/system chezmoi or override the bundled binary.
The bundled dependency is private to the release directory, not placed on PATH.

`internal/distribution/archive.go` owns immutable v2.73.1 upstream archive,
executable, checksum-file and MIT-license hashes. All three archives and the
official checksum file were independently downloaded; candidate hashes from the
external pointer matched the downloaded bytes and upstream checksum entries.
Packaging verifies that file, then each archive, executable architecture, Go
product/version metadata and license before using bytes. Source/version evidence:
https://github.com/twpayne/chezmoi/releases/tag/v2.73.1 .

The release inventory is exactly:

```text
dot                       0755
libexec/chezmoi            0755
licenses/chezmoi.txt       0644
release.json              0644
```

Metadata binds schema, strict SemVer, platform, chezmoi version, actual dependency
runtime requirements, and exact file hashes/modes. Dot's build identity stamp and
Go product path must agree with the metadata. Linux metadata records the actual
ELF interpreter/shared libraries; glibc/musl compatibility is not guessed.
No private source, scripts, engine tree or separate browser is packaged.
Tar/gzip ordering, modes, owners and times are normalized. Same source/toolchain
and inputs reproduce archives; reproducibility is not claimed across Go versions.

Verification needs a caller-supplied trusted SHA256 **before parsing/extraction**.
Symlinks/hardlinks, duplicate paths, unknown inventory, traversal, unsafe modes,
forged metadata/dependencies and trailing streams fail closed. Extraction reserves
a new clean absolute directory exclusively. Package verification checks binary
formats/build metadata without executing cross-platform payloads.

Checksums are trust inputs from the release publisher, not signatures or a claim
of protection against a compromised publisher. Runtime dependency hashes are
compiled in; changing the pinned dependency requires an intentionally reviewed
lock/tooling update, never a runtime download of “latest”.

## Installation ownership and promotion

```text
EXTRACTED/dot install --root /absolute/private/dot-cli \
  --archive RELEASE.tar.gz --sha256 TRUSTED_HASH --yes
/absolute/private/dot-cli/bin/dot self-update --root /absolute/private/dot-cli \
  --archive NEXT_RELEASE.tar.gz --sha256 TRUSTED_NEXT_HASH --yes
/absolute/private/dot-cli/bin/dot uninstall --root /absolute/private/dot-cli --yes
EXTRACTED/dot install-recover --root /absolute/private/dot-cli --yes
```

Add ROOT/bin to PATH yourself. No shell installer is required. Ownership is
separate from machine state: ROOT/owner.json strictly binds dot-native owner,
root, schema and filesystem UID. Nonempty foreign roots, manager/system paths,
symlinked paths and group/other-writable roots are refused. Package-manager-owned
dot must use its original manager; no-root errors explain that requirement.
Homebrew's chezmoi is never modified by artifact install/update/uninstall.

Releases live at ROOT/releases/ARCHIVE_SHA256. A nonblocking filesystem lock
serializes manager operations. Fully verified/fsynced staged contents are
published before a durable promotion journal; ROOT/current is atomically switched,
and ROOT/bin/dot is the owned relative launch link. Existing release bytes cannot
be silently replaced. Pending promotion blocks later operations. Recovery validates
both releases and the current pointer, rolls back to the previous release and
refuses conflicting human changes. Failed staging is retained for diagnosis.
Uninstall removes only owned launch/current links: releases, owner record,
private repositories, HOME targets, tracker, connection and adoption remain.

Self-update is deliberately a **local verified archive operation**. Downloading
and independently selecting/trusting a release precedes it. No GitHub CLI, Go
compiler, shell, network “latest” lookup or hidden checkout fallback runs on the
installed application's update path.

## Local maintainer tooling and #19 gate

All build/test/acceptance tooling is Go. The existing workflow still triggers only
on `v*` release-tag pushes; it calls `cmd/dot-release build` and publishes archives
plus SHA256SUMS. No PR, branch-push, scheduled or manual CI was added.

From the repository root:

```text
go test ./...
go vet ./...
go run ./cmd/dot-release build --version 0.10.18-rc.1 --out DIST
go run ./cmd/dot-release verify --archive FILE --sha256 HASH --version 0.10.18-rc.1
go run ./cmd/dot-release extract --archive FILE --sha256 HASH --dir NEW_ABSOLUTE_DIR
go run ./cmd/dot-accept --archive HOST_FILE --sha256 HOST_HASH --dir NEW_ABSOLUTE_DIR
```

`build --deps-dir DIR` uses an offline cache containing the three upstream
archives and `chezmoi_2.73.1_checksums.txt`; every cached byte is still checked.
Without it, the tool downloads only the pinned HTTPS assets. Build sanitizes Go
workspace/flag/experiment/architecture settings, disables CGO and VCS embedding,
cross-builds all three targets and re-verifies each archive before SHA256SUMS.

For actual version-upgrade/recovery testing, build a second version and add
`--update-archive NEXT_HOST_FILE --update-sha256 NEXT_HOST_HASH` to dot-accept.
This requires a different real version, verifies new execution and conflict-safe
rollback; no successful fake update flag exists.

The acceptance tool extracts outside the checkout, uses an isolated working
directory and HOME, and gives dot PATH containing **only system Git**. No Python,
Bash, shell, gum, gh, Go compiler, manager or system chezmoi is available. Native
pseudoterminals exercise both bare/init Welcome and returning-user pages with
normal quit. Real scripted setup, tracker, Track, machine-2 clone/local Git remote,
explicit pull/push, JSON, exact adoption, target bytes/executable mode, stale plans,
checksum-before-install, public/private update separation, recovery and uninstall
are exercised. Fixtures and transcripts/files are retained, never confused with
mocked engine success. #19 extends this to full keyboard setup/Continue/Replace,
logical Track, machine-2 Adopt, independently omitted gates, local profile/custom
policy, malformed data, concurrent approvals, and actual manager/source interrupts.
Acceptance compiles a non-installing native manager/SSH child fixture with Go on
the maintainer side; only that fixture enters PATH in the explicitly isolated
manager cases. Ordinary app journeys still have Git-only PATH. A manager's zero
exit must fail readiness, not masquerade as successful tool installation.

Linux **runtime execution is explicitly owner-waived**. Do not start Docker.
Both Linux artifacts are cross-built and inspected; these are not Linux runtime
passes. Real hosting/authentication and privileged package-manager installation
are not exercised. #19 owns whole-journey/edge-case acceptance, complete user
documentation/skills, four ADRs and the final release decision. No tag was created,
push performed, issue closed or host dependency overwritten by #18.

Context7 discovery exposed no Context7 tools. Current official Bubble Tea v2
`WithContext`/input/output source and the installed official x/sys PTY API docs
were consulted instead; upstream release/checksum evidence is recorded above.

## Exact local evidence for the next owner (2026-10-14)

All paths below are beneath the approved external base:
`/private/var/folders/lt/nylpq1rx5xxcfgm1h7tcv_sr0000gn/T/opencode/`.

| Path beneath that base | Contents/evidence |
| --- | --- |
| `dots18-upstream/` | Independently downloaded three pinned v2.73.1 archives and official checksum file; offline build cache |
| `dots18-final-dist/` | Final 0.10.18-rc.1 archives for all three platforms and SHA256SUMS |
| `dots18-final-update-dist/` | Real 0.10.18-rc.2 upgrade archives for all three platforms and SHA256SUMS |
| `dots18-repro-dist/` | Independent repeat rc.1 build; SHA256SUMS identical to final-dist |
| `dots18-final-accept-v3/` | Final real Darwin extracted bundle, Git-only PATH, two isolated HOMEs, local remote, reviewed plans/state and isolated installation/upgrade/recovery fixtures |
| `dots18-linux-amd64-inspected/`, `dots18-linux-arm64-inspected/` | Earlier verified extraction for static inspection, **not execution**; final archives reverified during final build |

Final rc.1 archive SHA256 values:

```text
7d7c20c8b9112901752004b4282d9ea2ff3806022e31d1464432ea391186710b  dot-0.10.18-rc.1-darwin-arm64.tar.gz
64b5713a5fe1fd32438794e7f01ec2c3ab346a9373f08a275ff9f2f51483545e  dot-0.10.18-rc.1-linux-amd64.tar.gz
b8ed4c5cb6edbc6d552bb21e0e82bd2c9ca0542a72990d0d02ba735e84b5211e  dot-0.10.18-rc.1-linux-arm64.tar.gz
```

The final Darwin rc.2 upgrade archive is
`dots18-final-update-dist/dot-0.10.18-rc.2-darwin-arm64.tar.gz`, SHA256
`23870fd64a7741c79f339ae1043b5139c89edcf52352fd793995d1819f83eed0`.
These are **local acceptance labels**, not published/tagged versions.

Exact repeatable commands from a checkout containing #18 (choose a new `--dir`):

```text
go test ./...
go vet ./...
go test -race ./cmd/dot ./cmd/dot-release ./internal/distribution ./internal/acceptance
env GOOS=linux GOARCH=amd64 CGO_ENABLED=0 go build ./...
env GOOS=linux GOARCH=arm64 CGO_ENABLED=0 go build ./...
go run ./cmd/dot-release build --version 0.10.18-rc.1 --out /private/var/folders/lt/nylpq1rx5xxcfgm1h7tcv_sr0000gn/T/opencode/dots18-final-dist --deps-dir /private/var/folders/lt/nylpq1rx5xxcfgm1h7tcv_sr0000gn/T/opencode/dots18-upstream
go run ./cmd/dot-release build --version 0.10.18-rc.2 --out /private/var/folders/lt/nylpq1rx5xxcfgm1h7tcv_sr0000gn/T/opencode/dots18-final-update-dist --deps-dir /private/var/folders/lt/nylpq1rx5xxcfgm1h7tcv_sr0000gn/T/opencode/dots18-upstream
go run ./cmd/dot-accept --archive /private/var/folders/lt/nylpq1rx5xxcfgm1h7tcv_sr0000gn/T/opencode/dots18-final-dist/dot-0.10.18-rc.1-darwin-arm64.tar.gz --sha256 7d7c20c8b9112901752004b4282d9ea2ff3806022e31d1464432ea391186710b --dir /private/var/folders/lt/nylpq1rx5xxcfgm1h7tcv_sr0000gn/T/opencode/dots19-accept-new --update-archive /private/var/folders/lt/nylpq1rx5xxcfgm1h7tcv_sr0000gn/T/opencode/dots18-final-update-dist/dot-0.10.18-rc.2-darwin-arm64.tar.gz --update-sha256 23870fd64a7741c79f339ae1043b5139c89edcf52352fd793995d1819f83eed0
```

All listed tests/vet/cross-builds and the real Darwin acceptance/upgrade/rollback
passed. ELF inspection found amd64 chezmoi requires
`/lib64/ld-linux-x86-64.so.2` and `libc.so.6`; arm64 chezmoi is static. This does
**not** establish a minimum glibc version, musl compatibility of the amd64 asset,
or Linux runtime acceptance. Linux execution remains explicitly owner-waived.

Useful #19 extensions: full keyboard-driven source-to-Done journey (the #18 PTY
gate opens/quits real pages; scripted commands perform the real setup effects),
machine assignment/profile/custom edge cases against the extracted executable,
cancelled source/manager operations, initial-install interruption versus upgrade
interruption, manager ownership fixtures, hostile metadata/archives, and final
user-help/skills/ADRs. Never exercise privileged host installation to do this.
