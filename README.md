# dots-cli

Public bootstrap for the private `dot` configuration system. This repository
contains **only** installer logic and public setup documentation. The source of
truth, managed configuration, and target inventories stay in the private
dotfiles repository; nothing here includes private configuration, credentials,
history, or machine inventories.

## Install

Download, review, execute (preferred):

```sh
curl --fail --location --proto '=https' --tlsv1.2 \
  "$APPROVED_BOOTSTRAP_URL" -o bootstrap.sh
printf '%s  bootstrap.sh\n' "$APPROVED_BOOTSTRAP_SHA256" | shasum -a 256 -c -
less bootstrap.sh
bash bootstrap.sh
```

Or pipe directly, understanding that human prompts still use `/dev/tty` and
an unattended run needs `--yes`:

```sh
curl --fail --location --proto '=https' --tlsv1.2 \
  "$APPROVED_BOOTSTRAP_URL" | bash -s -- [options]
```

## Defaults

- Permanent checkout: `~/.local/share/dot` (override with an absolute
  `--directory '/path/with spaces'`). Real history, branch, and `origin`
  upstream are preserved so `dot push`/`dot pull` work.
- Launcher: `~/.local/bin/dot`, plus idempotent marked PATH blocks in Bash and
  Zsh startup files (a nondefault absolute `ZDOTDIR` is respected).
- Local state: `$XDG_CONFIG_HOME/chezmoi/chezmoi.toml` (or
  `~/.config/chezmoi/chezmoi.toml`), `~/.local/state/dot/`, isolated
  `cacheDir`/`persistentState`. These are local-unmanaged.
- Chezmoi machine data: `name`, `email`, and `profile` (default `personal`,
  `--profile work`). Existing Git name/email is preserved; only missing values
  are prompted for.
- An **explicit empty adoption scope** is written before chezmoi is configured.
  Installation never applies configurations; adoption is a separate step.
- Matching existing checkouts offer **Update** or **Keep existing**. Updates are
  clean and fast-forward-only (`--existing update|keep`). Unrelated folders,
  launchers, malformed PATH markers, and user-owned symlinks are conflicts and
  are never overwritten.

## Platforms and dependencies

- **macOS:** Apple Silicon (`arm64`) macOS 15+ only. Intel Macs, Rosetta
  (`x86_64`), and older macOS are rejected. Missing tools use the default
  Apple Silicon Homebrew (`/opt/homebrew`) route; Homebrew setup is offered,
  pinned (commit + SHA-256), and downloaded privately. No dependency step runs
  as root, and normal Homebrew operations never use sudo.
- **Ubuntu:** 24.04+ on `amd64`/`x86_64` and `arm64`/`aarch64`. 22.04 only
  with an already compatible `python3` 3.11+ (including `tomllib`) on PATH.
  Missing Bash/Git/curl/Python go through apt; GitHub CLI through its official
  signed apt repository; Gum/chezmoi through reviewed pinned Debian packages
  verified by embedded SHA-256 pins. GitHub CLI 2.45.x/2.46.x are rejected on
  every platform. No Python PPA and no system-Python replacement on 22.04.
- **Arch:** official Arch on native `x86_64` only. CachyOS/Manjaro/ARM are not
  automatically supported. Missing tools map to official `pacman` targets, and
  the required **full system upgrade** is disclosed explicitly before any sudo
  or package transaction. No AUR and no compilation.
- Bash, Git, curl, GitHub CLI, Python 3.11+, chezmoi, and Gum are the baseline.
  Compatible installed tools are reused; already-installed dependencies are
  verified, not reinstalled. A command on PATH that fails capability checks is
  a failure, not proof of success.
- Dependency plans are printed before consent. Unattended mode (`--yes`)
  requires cached/passwordless sudo (`sudo -n`), existing commit identity, and
  verified private access. The installer does not feed passwords to sudo, does
  not use sudo for Homebrew, and cannot be used to silently approve a broader
  transaction than was displayed.

## Access and adoption

- The bootstrap is anonymous; the **private source is retrieved only after**
  GitHub authentication and verified access (API + Git) to the private
  repository. Existing credentials are reused only after that verification;
  otherwise account-switch or HTTPS browser login is offered. Tokens are never
  printed, logged, or committed.
- You may **clone/use a verified existing private dotfiles repository**, or,
  after separate explicit approval, **create a NEW private one**. Creation is
  never a fallback for denied access, never renames/deletes another repo, and a
  refused consent makes no remote change.
- Installation does **not** adopt configurations. The subsequent optional
  Adopt flow previews exact rendered target changes and bounded removals, and
  applies only after explicit apply approval, with an additional acknowledgment
  for Unsafe selections (shell, terminal, Git, desktop, services). Safe/Unsafe
  classification and OS applicability warnings (including the Arch/CachyOS
  desktop warning on Ubuntu) are always shown. Nothing is preselected.
- A matching existing chezmoi/HOME-as-repo manager requires explicit
  replacement consent (`--replace-chezmoi` in addition to `--yes`
  unattended), a private rollback backup, and exact narrow retirement. HOME,
  unrelated repos, credentials, and runtime data are never recursive deletion
  targets. Linux sync-timer cleanup is the owner's responsibility; nothing
  enables, stops, or edits timers.
- Canonical skills live only at `~/.agents/skills`; the previous canonical
  library is backed up exactly at `~/.agents/skills.bak` (never a timestamped
  tree, never silently overwritten). Noncanonical harness silos are retired,
  not merged. The Gemini mirror is generated output.
- Unselected files are never touched: selection scope persists locally, and
  normal `dot`/chezmoi operations enforce it. Scope expansion requires a new
  explicit choice and preview. Missing tools are printed and saved **locally**
  for later installation; nothing machine-specific is published.

## Unattended mode and recovery

- `--yes` is noninteractive: existing identity and authentication must already
  exist, replacement additionally requires `--replace-chezmoi`, and no
  configuration group is ever selected implicitly. Without a controlling
  terminal and without `--yes`, the bootstrap exits with guidance.
- Failures report the phase. Partial installations may leave installed
  prerequisites or a partial clone; inspect the diagnostic before retrying.
  Recovery backups and journals live under `~/.local/state/dot/`. There is no
  automatic rollback of packages, identity, or credential integration.
- Unsupported-case behavior is explicit: unsupported OS/architecture,
  incompatible Python, rejected GitHub CLI versions, missing Homebrew on a
  Mac that needs it, download/checksum failure, and privilege denial all fail
  closed with actionable guidance.

## Boundaries

- No live HOME adoption, package transaction, repository creation/deletion/
  rename, or push is triggered by reading this documentation.
- Publication updates keep the `--stage dependencies|migration|adoption|install`
  stages, `--repository OWNER/REPO`, and the fixture-only `--source-url` seam
  compatible with the private installer and scoped-management engine.
- Linux legacy sync-timer removal remains the owner's responsibility. Reusing
  the old remote URL can affect legacy clones; coordinate the supervised
  transition before depending on it.
