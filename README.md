# dots-cli

One Go CLI for setting up and managing a user's private dotfiles repository.
The target runtime dependencies are **Git and chezmoi**. Go dependencies are
compiled into `dot`; users do not need Go, Python, Bash, gum, or GitHub CLI to
run the application. Native package managers are invoked only for explicitly
requested dependency installation.

## Install a native release

Choose the archive for `darwin-arm64`, `linux-amd64` or `linux-arm64` and obtain
its SHA256 from the trusted release publisher. Verify the downloaded archive
before extracting it into a new directory. Keep the whole bundle together:
`dot`, `libexec/chezmoi`, `licenses/chezmoi.txt`, `release.json`.

```sh
./extracted/dot install --root "$HOME/.local/share/dot-cli" \
  --archive /absolute/path/to/release.tar.gz --sha256 TRUSTED_SHA256 --yes
export PATH="$HOME/.local/share/dot-cli/bin:$PATH"
dot --health
dot
```

System Git is required; chezmoi **v2.73.1** is privately bundled and verified.
Homebrew-owned tools are never overwritten. Manager-owned dot installations must
use their own manager. Linux amd64's bundled chezmoi requires glibc; musl and a
minimum glibc version are not established. Linux runtime acceptance was explicitly
waived by the owner; see [acceptance evidence](docs/native-acceptance.md).
These local acceptance builds are not published releases.

## First run and another machine

Bare `dot` and `dot init` open one Bubble Tea app:
**Welcome → Source → Machine → Dependencies → Tracker → Done**.

- Paste your credential-free HTTPS/SSH repository URL to clone, or choose New
  user to initialize locally and add origin. Create the remote on your platform
  yourself; dot does not create, commit or push it.
- The default checkout is `~/.local/share/dot-config`. Existing source offers
  Continue or Replace. Replace defaults to Cancel and needs a second confirmation
  naming a timestamped backup; the complete old checkout is preserved, never deleted.
- Edit the suggested machine id in the embedded Huh form. Identity and exact
  adoption remain local. Existing identity is immutable; the machine catalog
  controls locally known collisions.
- Dependencies show missing tools, exact brew/pacman commands and official links.
  Enter authorizes native installation; Space leaves setup incomplete. Native
  authentication runs outside the raw terminal; dot never captures passwords.
- Review Tracker's paths and bounded identity values. Cancel is the default;
  confirmation creates/connects local tracker state without replacing `.gitconfig`.
  Done rechecks readiness and enrolls no files.

On machine 2, clone the same repository, choose a distinct identity, inspect Diff,
then explicitly select what to Adopt. Pull is separately confirmed, clean and
fast-forward-only; it never adopts. Push sends existing commits, never creates one.

`dot init --check` is offline and read-only: `old-user` means a valid Git root and
contract, not that all setup steps are complete. Invalid existing data fails clearly.
Use `--home`, `--source`, `--config`, `--state-dir` for isolated paths; options follow
the verb. Defaults and precedence are in [native setup](docs/native-setup.md).

## Review configuration

Returning users open Status & Diff in a six-tab workspace. Track imports exact
HOME files into the repository; Adopt writes exact repository entries to this machine. Nothing is
preselected. Space selects, `e` edits metadata, `M` opens logical-file metadata,
Tab/Shift+Tab cycles tabs 1–6, `m` retargets machines in Track, `s` composes Adopt
profiles, `p` previews, and `a` enters Cancel-default final confirmation.
The workspace uses bounded inventory and diff panes with a persistent key footer.
Track initially lists HOME's immediate children; `l`/Right/Enter loads a folder
when expanded, and `h`/Left collapses it or returns to its parent. `/` filters the
loaded tree as you type; Enter searches all HOME folders. PgUp/PgDn and Home/End
navigate large lists, `J`/`K` scroll the diff, and `?` opens the complete keymap.
Folder selection still snapshots exact eligible files recursively. Cursor previews
are debounced and cancelled when superseded; browsing never authorizes a write.
Settings groups connection details, paths, tools and source maintenance into
bordered sections. `/` searches settings; Ctrl+N/Ctrl+P focuses visible actions
and Enter activates them. `A` expands advanced command details. Machine identity
and paths are displayed read-only; verification never installs or applies files.
Use your terminal's paste/copy shortcut; dot accepts bracketed paste and does
not launch system clipboard helpers for Ctrl+V/Ctrl+Shift+C.
Unsafe (`u`), desktop (`d`) and applicable backup replacement (`b`) gates are
independent. Every changed input requires a fresh plan.

[Machine assignments](docs/machine-assignments.md) default to `all`; exclusions
win. Local deselections/customs survive profile changes without changing shared
entries. Profile selection freezes a union of exact ids; Update never enrolls
future additions. Directory selection likewise freezes current files.

For agents and scripts:

```sh
dot status --format json
dot diff --format json
dot scope show --format json
dot intent plan --request request.json > plan.json
dot intent preview --plan plan.json
dot intent apply --plan plan.json --approve EXACT_PLAN_ID
```

Supply required `--ack-unsafe`, `--ack-desktop`, `--ack-backup` separately only
after reviewing the actual plan. Inspection is offline. Unsupported helpers/private
executable templates fail closed; the retired runtime is not a fallback.
Use `dot recovery` and `dot recover --approve EXACT_PLAN_ID` for interrupted
configuration work; conflicting human edits are preserved/refused.

Help works before setup: `dot --help` and `dot COMMAND --help`. See
[native distribution](docs/native-distribution.md) for the full retained/retired
verb inventory and verified archive update/uninstall/recovery commands.

## Releases

GitHub Actions runs only when a version tag such as `v1.2.3` is pushed. It builds
`darwin-arm64`, `linux-amd64`, and `linux-arm64` archives, generates `SHA256SUMS`,
and creates a GitHub release. Tags such as `v1.2.3-rc.1` create prereleases.
Each archive contains `dot`, private pinned chezmoi v2.73.1, its license and
verified release metadata. System Git remains an external dependency.

There are no pull-request or branch-push checks. Agents run tests, vet, and builds
locally before pushing. Release tags publish the tagged code, including its
current implementation status.

## Develop and verify

```sh
go test ./...
go vet ./...
go build -o dot ./cmd/dot
go run ./cmd/dot --help
```

Development builds require system Git and chezmoi v2.73.1. Release builds have
no checkout fallback. `cmd/dot-release` cross-builds/verifies archives;
`cmd/dot-accept` executes an extracted artifact outside the checkout with Git-only
PATH and isolated HOME. [Durable acceptance evidence](docs/native-acceptance.md)
records exact commands, hashes, keyboard transcripts and waived execution gates.
[Spec #10](https://github.com/ujjwal6792/dots-cli/issues/10) subsumes #7/#8/#9;
the [ADRs](docs/adr/) record the native architecture decisions.

## Product boundaries

- One Bubble Tea app for setup and subsequent use; scriptable verbs call the
  same in-process engine.
- User-owned private repository, connected with system Git. Users create their
  remote repository themselves.
- Track and Adopt are separate, exact reviewed operations. Navigation never
  authorizes changes. Unsafe and desktop effects require separate confirmation.
- Machine identity and local adoption state remain local; shared assignments
  change only through an approved Track plan.
- Brew and pacman are the initial dependency-install integrations. AUR helpers,
  apt integration, Windows, and legacy group-scope migration are outside #10.
- Private configuration, credentials, inventories and recovery backups are not
  shipped in public releases.
