#!/usr/bin/env bash
# Standalone/public-safe installer. No private config payload belongs here.
set -Eeuo pipefail
umask 077
YES=false REPLACE_CHEZMOI=false STAGE=install DIRECTORY="${HOME:?HOME is required}/.local/share/dot-config" ENGINE_DIRECTORY="${HOME:?HOME is required}/.local/share/dot-cli" BROWSER_URL='' BROWSER_SHA256=''
REPOSITORY=ujjwal6792/dotfiles SOURCE_URL='' ENGINE_URL='' ENGINE_REPOSITORY=ujjwal6792/dots-cli UPDATE=ask PROFILE=personal ZSH_DIRECTORY=''
DEPENDENCIES=(bash git curl gh python3)
TEMP='' PHASE=arguments PLATFORM='' UBUNTU_VERSION='' DEB_ARCH=''
OWNED_BROWSER_TMP=''
OWNED_CHECKOUT_TMP=''
fail() { printf 'dot installer: %s\n' "$*" >&2; exit 1; }
cleanup() {
  [ -z "$TEMP" ] || rm -f -- "$TEMP/homebrew.sh" "$TEMP/"*.deb "$TEMP/dot-github-cli.gpg" "$TEMP/dot-github-cli.list"
  [ -z "$TEMP" ] || rmdir "$TEMP" 2>/dev/null || true
  [ -z "$OWNED_BROWSER_TMP" ] || rm -f -- "$OWNED_BROWSER_TMP"
  # Only the private directory returned by this invocation's mktemp is owned.
  [ -z "$OWNED_CHECKOUT_TMP" ] || rm -rf -- "$OWNED_CHECKOUT_TMP"
}
trap cleanup EXIT
trap 'printf "dot installer: incomplete during %s; inspect the diagnostic, adoption results/recovery if applicable, and rerun.\n" "$PHASE" >&2' ERR
usage() {
  echo 'usage: bootstrap.sh [--yes] [--directory ABSOLUTE_PATH] [--engine-directory ABSOLUTE_PATH] [--engine-url ABSOLUTE_LOCAL_FIXTURE] [--source-url ABSOLUTE_LOCAL_FIXTURE] [--browser-url HTTPS_OR_ABSOLUTE_FIXTURE --browser-sha256 REVIEWED_SHA256] [--stage install|dependencies|migration|adoption]'
}
while [ $# -gt 0 ]; do
  case "$1" in
    --yes) YES=true; shift ;;
    --replace-chezmoi) REPLACE_CHEZMOI=true; shift ;;
    --directory|--engine-directory|--zsh-directory|--existing|--profile|--repository|--source-url|--engine-url|--browser-url|--browser-sha256|--stage)
      [ $# -ge 2 ] || fail "missing value for $1"
      case "$1" in
        --directory) DIRECTORY="$2" ;; --engine-directory) ENGINE_DIRECTORY="$2" ;; --zsh-directory) ZSH_DIRECTORY="$2" ;; --existing) UPDATE="$2" ;; --profile) PROFILE="$2" ;;
        --repository) REPOSITORY="$2" ;; --source-url) SOURCE_URL="$2" ;; --engine-url) ENGINE_URL="$2" ;; --browser-url) BROWSER_URL="$2" ;; --browser-sha256) BROWSER_SHA256="$2" ;; --stage) STAGE="$2" ;;
      esac
      shift 2 ;;
    --help|-h) usage; exit 0 ;;
    *) usage >&2; fail 'unknown argument' ;;
  esac
done
case "$DIRECTORY" in /*) ;; *) fail '--directory must be absolute' ;; esac
case "$ENGINE_DIRECTORY" in /*) ;; *) fail '--engine-directory must be absolute' ;; esac
case "$PROFILE" in personal|work) ;; *) fail 'profile must be personal or work' ;; esac
case "$UPDATE" in ask|update|keep) ;; *) fail 'existing choice must be update or keep' ;; esac
  case "$STAGE" in install|dependencies|migration|adoption) ;; *) fail 'unknown stage' ;; esac
[[ "$REPOSITORY" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || fail 'repository must be OWNER/REPO, without credentials'
if [ -n "$SOURCE_URL" ]; then
  case "$SOURCE_URL" in /*) ;; *) fail '--source-url accepts only absolute local fixture paths' ;; esac
else
  SOURCE_URL="https://github.com/$REPOSITORY.git"
fi
if [ -n "$ENGINE_URL" ]; then case "$ENGINE_URL" in /*) ;; *) fail '--engine-url accepts only absolute local fixture paths' ;; esac; else ENGINE_URL="https://github.com/$ENGINE_REPOSITORY.git"; fi
if [ -n "$BROWSER_URL" ]; then
  [[ "$BROWSER_SHA256" =~ ^[0-9a-fA-F]{64}$ ]] || fail '--browser-sha256 must be a reviewed 64-digit SHA-256'
  BROWSER_SHA256="$(printf '%s' "$BROWSER_SHA256" | LC_ALL=C tr 'A-F' 'a-f')"
  case "$BROWSER_URL" in https://*) ;; /*) ;; *) fail '--browser-url must be HTTPS or an absolute fixture path' ;; esac
  case "$BROWSER_URL" in https://*@*|*\?*|*\#*) fail '--browser-url must not contain credentials, query parameters, or fragments' ;; esac
elif [ -n "$BROWSER_SHA256" ]; then fail '--browser-sha256 requires --browser-url'; fi
[ "$(id -u)" != 0 ] || fail 'do not run the whole installer as root or through sudo'
optional_adoption() {
  echo 'Installation and adoption are separate. Safe means lower impact, not loss-free replacement.'
  if "$YES"; then
    echo '--yes does not select or approve Track/Adopt. Optional rounds skipped; scope remains unchanged.'
    printf 'Later: "%s/tools/dot/dot" intent rounds (or agent intent plan/preview/apply with exact approval).\n' "$DIRECTORY"
    return
  fi
  if [ ! -f "$HOME/.local/bin/dot" ]; then
    [ "$STAGE" != adoption ] || fail 'optional rounds require the installed split launcher; finish bootstrap first'
    echo 'Optional Track/Adopt rounds unavailable; initialization is pending for dot init.'
    return
  fi
  [ -x "$ENGINE_DIRECTORY/tools/dot/dot" ] && [ ! -L "$ENGINE_DIRECTORY/tools/dot/dot" ] || fail 'optional rounds require installed public engine'
  [ -f "${XDG_CONFIG_HOME:-$HOME/.config}/chezmoi/chezmoi.toml" ] || { echo 'Tracker initialization is pending; run dot init. Finish selected; no changes made.'; return; }
  local config
  config="$(python3 - "$HOME" "$DIRECTORY" "$ENGINE_DIRECTORY" <<'PY'
import json, sys
from pathlib import Path
home, checkout, engine = map(Path, sys.argv[1:])
record = home / '.local/state/dot/install.json'
if record.resolve() != home.resolve() / record.relative_to(home):
    raise SystemExit('installation record must not traverse symlinks')
data = json.loads(record.read_text())
if data.get('directory') != str(checkout) or data.get('engine') != str(engine) or data.get('version') != 2:
    raise SystemExit('optional adoption requires a matching installation record')
print(data['config'])
PY
)" || fail 'review installed checkout/config before adoption'
  PHASE=optional-adoption
  if ! { exec 3<> /dev/tty; } 2>/dev/null; then
    fail 'optional adoption needs a terminal; use agent intent plan/preview/apply instead'
  fi
  DOT_SOURCE="$DIRECTORY" DOT_ENGINE="$ENGINE_DIRECTORY/tools/dot" DOT_BROWSER="$ENGINE_DIRECTORY/bin/dot-browser" DOT_CONFIG="$config" "$HOME/.local/bin/dot" init --rounds <&3 >&3 2>&3
}
if [ "$STAGE" = adoption ]; then optional_adoption; exit 0; fi
PHASE=platform
# Stable platform seam: future routes belong here, not in another installer.
platform_check() {
  local version major release distro architecture
  case "$(uname -s)" in
    Darwin)
      PLATFORM=macos
      [ "$(uname -m)" = arm64 ] || fail 'Intel/Rosetta macOS is unsupported; run natively on Apple Silicon'
      version="$(sw_vers -productVersion)"; major="${version%%.*}"
      [[ "$major" =~ ^[0-9]+$ ]] && [ "$major" -ge 15 ] || fail 'Apple Silicon macOS 15+ is required' ;;
    Linux)
      PLATFORM=ubuntu
      # Treat os-release as data, never execute distro metadata as shell code.
      release="$(cat /etc/os-release)"
      distro="$(printf '%s\n' "$release" | awk -F= '$1 == "ID" {gsub(/[\047\042]/, "", $2); print $2}')"
      UBUNTU_VERSION="$(printf '%s\n' "$release" | awk -F= '$1 == "VERSION_ID" {gsub(/[\047\042]/, "", $2); print $2}')"
      case "$distro" in
        arch)
          PLATFORM=arch
          [ "$(uname -m)" = x86_64 ] || fail 'unsupported Arch architecture; official Arch Linux x86_64 is required (Arch Linux ARM is not supported)'
          return ;;
        ubuntu) ;;
        *) fail 'unsupported Linux distribution; use supported Ubuntu or official Arch Linux' ;;
      esac
      [[ "$UBUNTU_VERSION" =~ ^[0-9][0-9]\.[0-9][0-9]$ ]] || fail 'cannot determine supported Ubuntu release'
      major="${UBUNTU_VERSION%%.*}"
      [ "$UBUNTU_VERSION" = 22.04 ] || [ "$major" -gt 24 ] || { [ "$major" -eq 24 ] && [ "${UBUNTU_VERSION#*.}" -ge 4 ]; } || fail 'Ubuntu 24.04+ is required; 22.04 additionally requires an already compatible python3 (3.11+)'
      DEB_ARCH="$(dpkg --print-architecture)"; architecture="$(uname -m)"
      case "$DEB_ARCH:$architecture" in
        amd64:x86_64|arm64:aarch64|arm64:arm64) ;;
        *) fail 'unsupported Ubuntu architecture; native amd64/x86_64 or arm64/aarch64 is required' ;;
      esac ;;
    *) fail 'unsupported operating system; use Apple Silicon macOS 15+, supported Ubuntu, or official Arch Linux x86_64' ;;
  esac
}
platform_check
prepare_browser() {
  [ -n "$BROWSER_URL" ] || return 0
  command -v shasum >/dev/null || fail 'shasum is required to verify browser artifact'
  OWNED_BROWSER_TMP="$(mktemp "${TMPDIR:-/var/tmp}/dot-browser.XXXXXXXX")"
  case "$BROWSER_URL" in https://*) curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' --tlsv1.2 "$BROWSER_URL" -o "$OWNED_BROWSER_TMP" ;; /*) cp "$BROWSER_URL" "$OWNED_BROWSER_TMP" ;; esac
  [ "$(shasum -a 256 "$OWNED_BROWSER_TMP" | awk '{print $1}')" = "$BROWSER_SHA256" ] || fail 'browser artifact checksum mismatch; not installed'
  chmod 755 "$OWNED_BROWSER_TMP"
}
if ! "$YES"; then
  # stdin may contain this very script. Never consume it for questions.
  if ! { exec 3<> /dev/tty; } 2>/dev/null; then
    fail 'no controlling terminal; rerun with --yes and preconfigured Git identity/private access, or download and run in a terminal'
  fi
fi
ask() { printf '%s ' "$1" >&3; IFS= read -r ANSWER <&3 || fail 'terminal input ended'; }
confirm() {
  "$YES" && return 0
  ask "$1 [y/N]"
  case "$ANSWER" in y|Y|yes) ;; *) fail 'cancelled; nothing further will be changed' ;; esac
}
compatible() {
  command -v "$1" >/dev/null 2>&1 || return 1
  case "$1" in
    python3) python3 -c 'import sys,tomllib; sys.exit(sys.version_info < (3,11))' >/dev/null 2>&1 ;;
    bash) bash -c 'test "$BASH_VERSINFO" -ge 3' >/dev/null 2>&1 ;;
    git) git --version >/dev/null 2>&1 ;;
    curl) curl --version >/dev/null 2>&1 ;;
    gh)
      local version
      version="$(gh --version 2>/dev/null)" || return 1
      [[ "$version" =~ ^gh\ version\ ([0-9]+)\.([0-9]+)\.([0-9]+) ]] || return 1
      # Upstream documents these community versions as broken by API retirement.
      if [ "${BASH_REMATCH[1]}" = 2 ] && { [ "${BASH_REMATCH[2]}" = 45 ] || [ "${BASH_REMATCH[2]}" = 46 ]; }; then return 1; fi
      gh auth setup-git --help >/dev/null 2>&1 && gh auth switch --help >/dev/null 2>&1 ;;
    chezmoi) chezmoi --version >/dev/null 2>&1 ;;
    gum) gum --version >/dev/null 2>&1 ;;
  esac
}
BREW_BIN='' PYTHON_BIN=''
activate_brew() {
  BREW_BIN="$(dirname "$(command -v brew)")"
  PYTHON_BIN="$(brew --prefix python@3.11)/libexec/bin"
  export PATH="$PYTHON_BIN:$BREW_BIN:$PATH"
}
setup_homebrew() {
  echo 'Homebrew setup is missing. Its reviewed, pinned installer may request administrator permission to provision /opt/homebrew.'
  confirm 'Approve Homebrew provisioning and dependency discovery?'
  compatible curl || fail 'curl must already be available to download Homebrew securely'
  command -v shasum >/dev/null || fail 'shasum is required to verify Homebrew setup'
  TEMP="$(mktemp -d "${TMPDIR:-/tmp}/dot-bootstrap.XXXXXXXX")"
  curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 \
    'https://raw.githubusercontent.com/Homebrew/install/700c9a145d37a3f0f3bd3b7c208d7adab31bd278/install.sh' -o "$TEMP/homebrew.sh"
  [ "$(shasum -a 256 "$TEMP/homebrew.sh" | awk '{print $1}')" = ece8385d14080da35eb3c1f72cf43b71be5d1a3838e554e4bb748b9888b15cec ] || fail 'Homebrew installer checksum mismatch; downloaded code was not executed'
  if "$YES"; then NONINTERACTIVE=1 bash "$TEMP/homebrew.sh"; else bash "$TEMP/homebrew.sh" <&3 >&3 2>&3; fi
  if ! command -v brew >/dev/null 2>&1; then export PATH="/opt/homebrew/bin:$PATH"; fi
  command -v brew >/dev/null || fail 'Homebrew setup did not produce brew; inspect its output before retrying'
  activate_brew
}
install_packages() {
  local package closure pending seen=$'\n' plan=()
  # These affect only the installer subprocess, not the user's Homebrew config.
  export HOMEBREW_NO_AUTO_UPDATE=1 HOMEBREW_NO_INSTALLED_DEPENDENTS_CHECK=1
  export HOMEBREW_NO_INSTALL_CLEANUP=1 HOMEBREW_NO_ASK=1
  [ "$(brew --prefix)" = /opt/homebrew ] || fail 'automatic packages require the supported default Apple Silicon Homebrew prefix /opt/homebrew; preinstall compatible dependencies or review your Homebrew layout'
  closure="$(brew deps --topological --union --full-name --formula "$@")"
  # The closure can include another requested package; keep its first,
  # dependency-ordered occurrence rather than installing it twice.
  while IFS= read -r package; do
    [ -n "$package" ] || continue
    package="${package#homebrew/core/}"
    [[ "$package" =~ ^[A-Za-z0-9][A-Za-z0-9_.+@-]*$ ]] || fail 'dependency plan contains a non-core or invalid formula; no packages installed'
    case "$seen" in *$'\n'"$package"$'\n'*) continue ;; esac
    plan+=("$package"); seen+="$package"$'\n'
  done < <(printf '%s\n' "$closure" "$@")
  printf 'Full dependency-order bottle plan (no implicit dependent updates or cleanup):'
  printf ' %s' "${plan[@]}"; echo
  confirm 'Approve this dependency plan?'
  for package in "${plan[@]}"; do
    # Never let Homebrew implicitly install an unplanned dependency without the
    # explicit bottle flag. Recheck the closure immediately before each step.
    pending="$(brew deps --missing --formula "$package")"
    [ -z "$pending" ] || fail "unplanned or unsatisfied dependencies for $package; no implicit installation attempted; inspect Homebrew metadata and retry"
    echo "Installing explicit bottle: $package"
    brew install --force-bottle "$package"
  done
}
macos_dependencies() {
  PHASE=dependencies
  local tool package missing=() brew_available=false
  if command -v brew >/dev/null 2>&1; then
    if brew --version >/dev/null 2>&1; then brew_available=true; activate_brew; fi
  elif [ -x /opt/homebrew/bin/brew ]; then
    export PATH="/opt/homebrew/bin:$PATH"; brew_available=true; activate_brew
  fi
  for tool in "${DEPENDENCIES[@]}"; do
    if ! compatible "$tool"; then
      package="$tool"; [ "$tool" != python3 ] || package=python@3.11
      missing+=("$package")
    fi
  done
  printf 'Dependency plan: Bash, Git, curl, GitHub CLI, Python 3.11+, chezmoi, Gum.\n'
  if [ "${#missing[@]}" -eq 0 ]; then
    echo 'All dependencies are compatible; reuse them (no package transaction).'
  else
    printf 'Homebrew bottles only (no compilation, no sudo for normal packages):'; printf ' %s' "${missing[@]}"; echo
    if ! "$brew_available"; then setup_homebrew; fi
    install_packages "${missing[@]}"
    activate_brew
  fi
  for tool in "${DEPENDENCIES[@]}"; do compatible "$tool" || fail "dependency verification failed: $tool; inspect package output before retrying"; done
  echo 'Dependencies verified.'
}
platform_dependencies() {
   case "$PLATFORM" in
     macos) macos_dependencies ;;
     ubuntu) ubuntu_dependencies ;;
     arch) arch_dependencies ;;
   esac
}

arch_dependencies() {
   PHASE=dependencies
   local tool package missing=()
   for tool in "${DEPENDENCIES[@]}"; do
     if ! compatible "$tool"; then
       case "$tool" in
         bash|curl) package="core/$tool" ;;
         python3) package=core/python ;;
         gh) package=extra/github-cli ;;
         git|chezmoi|gum) package="extra/$tool" ;;
       esac
       missing+=("$package")
     fi
   done
   echo 'Dependency plan: Bash, Git, curl, GitHub CLI, Python 3.11+, chezmoi, Gum.'
   if [ "${#missing[@]}" -eq 0 ]; then
     echo 'All dependencies are compatible; reuse them (no package transaction).'
   else
     if ! command -v pacman >/dev/null || ! command -v sudo >/dev/null; then fail 'Arch setup requires pacman and sudo; provision these explicitly and retry'; fi
     printf 'Official Arch dependency targets:'; printf ' %s' "${missing[@]}"; echo
     printf 'Sudo transaction: pacman -Syu'; printf ' %s' "${missing[@]}"; echo
     echo 'This refreshes package databases AND performs a FULL SYSTEM UPGRADE, including unrelated installed packages from your configured repositories, dependencies, replacements, package scripts and hooks. A reboot or .pacnew review may be required.'
     echo 'Read https://archlinux.org/news/ and review your pacman configuration first. No AUR helper, compilation, partial upgrade, repository edits, or application/desktop provisioning is requested.'
     echo 'Interactive pacman displays its resolved transaction and asks for final approval. --yes approves the full transaction with --noconfirm and requires cached/passwordless sudo.'
     confirm 'Approve sudo and this FULL SYSTEM UPGRADE plus official dependency targets?'
     if "$YES"; then
       sudo -n -v || fail '--yes needs already available sudo permission; authorize it separately or rerun in a terminal'
       sudo -n pacman -Syu --noconfirm "${missing[@]}" || fail 'Arch transaction failed; inspect pacman output, resolve permissions/network/package errors and complete the full upgrade before retrying. No partial-upgrade fallback or package rollback was attempted.'
     else
       sudo -v <&3 >&3 2>&3
       sudo pacman -Syu "${missing[@]}" <&3 >&3 2>&3 || fail 'Arch transaction failed or cancelled; inspect pacman output and complete any pending full upgrade before retrying. No partial-upgrade fallback or package rollback was attempted.'
     fi
     hash -r
   fi
   for tool in "${DEPENDENCIES[@]}"; do compatible "$tool" || fail "dependency verification failed: $tool; inspect pacman output and PATH before retrying"; done
   echo 'Dependencies verified.'
}

ubuntu_dependencies() {
  PHASE=dependencies
  local tool missing=() base=() upstream=() need_gh=false
  if [ "$UBUNTU_VERSION" = 22.04 ] && ! compatible python3; then
    fail 'Ubuntu 22.04 requires an already available python3 3.11+ with tomllib on PATH; provision it explicitly or upgrade Ubuntu. No Python PPA or system-Python replacement was attempted.'
  fi
  for tool in "${DEPENDENCIES[@]}"; do
    if ! compatible "$tool"; then
      missing+=("$tool")
      case "$tool" in
        gh) need_gh=true ;;
        gum|chezmoi) upstream+=("$tool") ;;
        *) base+=("$tool") ;;
      esac
    fi
  done
  echo 'Dependency plan: Bash, Git, curl, GitHub CLI, Python 3.11+, chezmoi, Gum.'
  if [ "${#missing[@]}" -eq 0 ]; then
    echo 'All dependencies are compatible; reuse them (no package transaction).'
  else
    if ! command -v apt-get >/dev/null || ! command -v sudo >/dev/null; then fail 'Ubuntu setup requires apt-get and sudo; provision these explicitly and retry'; fi
    command -v sha256sum >/dev/null || fail 'sha256sum is required for verified Ubuntu downloads; provision coreutils and retry'
    echo 'Ubuntu sudo plan: apt metadata refresh and named packages/dependencies only; no full system upgrade, removals, Python PPA, or application installation.'
    if [ "${#base[@]}" -gt 0 ]; then printf 'Ubuntu apt packages:'; printf ' %s' "${base[@]}"; echo; fi
    if "$need_gh" || [ "${#upstream[@]}" -gt 0 ]; then
      base+=(ca-certificates)
      echo 'Ubuntu apt transport prerequisite: ca-certificates.'
    fi
    if "$need_gh"; then
      echo 'Third-party repository: https://cli.github.com/packages stable main; package gh.'
      echo 'Scoped key: /etc/apt/keyrings/dot-github-cli.gpg; signed-by source: /etc/apt/sources.list.d/dot-github-cli.list. Existing conflicting files are never replaced.'
    fi
    for tool in "${upstream[@]}"; do
      case "$tool" in
        gum) echo "Verified upstream Debian package: Gum 0.17.0 ($DEB_ARCH), github.com/charmbracelet/gum; apt may run package maintainer scripts." ;;
        chezmoi) echo "Verified upstream Debian package: chezmoi 2.73.0 ($DEB_ARCH), github.com/twpayne/chezmoi; apt may run package maintainer scripts." ;;
      esac
    done
    echo 'Downloads use embedded reviewed SHA-256 pins, not an unverified installer or a freshly downloaded checksum as authority.'
    confirm 'Approve sudo, package, and disclosed third-party setup?'
    if "$YES"; then sudo -n -v || fail '--yes needs already available sudo permission; authorize it separately or rerun in a terminal'; else sudo -v <&3 >&3 2>&3; fi
    ubuntu_sudo env DEBIAN_FRONTEND=noninteractive apt-get update --error-on=any
    if [ "${#base[@]}" -gt 0 ]; then ubuntu_apt_install "${base[@]}"; fi
    # curl may itself have just been installed. No insecure download fallback.
    compatible curl || fail 'dependency verification failed: curl; verified downloads cannot proceed'
    TEMP="$(mktemp -d "${TMPDIR:-/tmp}/dot-bootstrap.XXXXXXXX")"
    for tool in "${upstream[@]}"; do ubuntu_download_package "$tool"; done
    if "$need_gh"; then ubuntu_github_repository; fi
    local packages=()
    if "$need_gh"; then packages+=(gh); fi
    for tool in "${upstream[@]}"; do
      case "$tool" in
        gum) packages+=("$TEMP/gum_0.17.0_$DEB_ARCH.deb") ;;
        chezmoi) packages+=("$TEMP/chezmoi_2.73.0_linux_$DEB_ARCH.deb") ;;
      esac
    done
    if [ "${#packages[@]}" -gt 0 ]; then ubuntu_apt_install "${packages[@]}"; fi
    hash -r
  fi
  for tool in "${DEPENDENCIES[@]}"; do compatible "$tool" || fail "dependency verification failed: $tool; inspect package output and PATH before retrying"; done
  echo 'Dependencies verified.'
}

ubuntu_sudo() {
  if "$YES"; then sudo -n "$@"; else sudo "$@" <&3; fi
}
ubuntu_apt_install() {
  echo 'Resolved apt transaction (includes dependency additions/upgrades; package removal is forbidden):'
  ubuntu_sudo env DEBIAN_FRONTEND=noninteractive apt-get --simulate --no-remove --no-install-recommends install "$@"
  confirm 'Approve this resolved apt transaction?'
  ubuntu_sudo env DEBIAN_FRONTEND=noninteractive apt-get --yes --no-remove --no-install-recommends install "$@"
}
ubuntu_download() {
  curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' --tlsv1.2 "$1" -o "$2"
  [ "$(sha256sum "$2" | awk '{print $1}')" = "$3" ] || fail "checksum mismatch: ${2##*/}; artifact was not installed; review the upstream pin before retrying"
}
ubuntu_download_package() {
  local filename url digest
  case "$1:$DEB_ARCH" in
    gum:amd64) digest=4c59b09c7248ea03c1544a11506b1152b1d8bd20e602fb2c2e9d158204d1f490 ;;
    gum:arm64) digest=9ae145bdd738f65e843f904957496d5aea1c2c182f3f1999b609ed9627f6a68e ;;
    chezmoi:amd64) digest=24df0be680b422e87fd8b4930fba583650920d68af81d53162a30867f8a8e0c8 ;;
    chezmoi:arm64) digest=99372f1e46c0532d377c0a368d3d3ba60eb252a9dbba3ca43e1767f571a73362 ;;
    *) fail 'no reviewed upstream package for this architecture' ;;
  esac
  case "$1" in
    gum) filename="gum_0.17.0_$DEB_ARCH.deb"; url="https://github.com/charmbracelet/gum/releases/download/v0.17.0/$filename" ;;
    chezmoi) filename="chezmoi_2.73.0_linux_$DEB_ARCH.deb"; url="https://github.com/twpayne/chezmoi/releases/download/v2.73.0/$filename" ;;
  esac
  ubuntu_download "$url" "$TEMP/$filename" "$digest"
}
ubuntu_github_repository() {
  local key=/etc/apt/keyrings/dot-github-cli.gpg list=/etc/apt/sources.list.d/dot-github-cli.list path
  local digest=6084d5d7bd8e288441e0e94fc6275570895da18e6751f70f057485dc2d1a811b
  local source="deb [arch=$DEB_ARCH signed-by=$key] https://cli.github.com/packages stable main"
  ubuntu_download 'https://cli.github.com/packages/githubcli-archive-keyring.gpg' "$TEMP/dot-github-cli.gpg" "$digest"
  printf '%s\n' "$source" > "$TEMP/dot-github-cli.list"
  for path in /etc/apt /etc/apt/keyrings /etc/apt/sources.list.d "$key" "$list"; do
    if ubuntu_sudo test -L "$path"; then fail "apt path conflict (symlink): $path; left untouched"; fi
  done
  if ubuntu_sudo test -e "$key"; then
    [ "$(ubuntu_sudo sha256sum "$key" | awk '{print $1}')" = "$digest" ] || fail "apt key conflict: $key; left untouched"
  fi
  if ubuntu_sudo test -e "$list"; then
    [ "$(ubuntu_sudo cat "$list")" = "$source" ] || fail "apt source conflict: $list; left untouched"
  fi
  ubuntu_sudo install -d -m 755 /etc/apt/keyrings /etc/apt/sources.list.d
  ubuntu_sudo install -m 644 "$TEMP/dot-github-cli.gpg" "$key"
  ubuntu_sudo install -m 644 "$TEMP/dot-github-cli.list" "$list"
  echo 'GitHub CLI repository configured; these disclosed repository files remain if a later step fails.'
  ubuntu_sudo env DEBIAN_FRONTEND=noninteractive apt-get update --error-on=any
}
platform_check
for tool in bash git curl python3; do compatible "$tool" || { [ "$STAGE" = dependencies ] && { echo "Missing/incompatible prerequisite: $tool; dot init owns dependency setup."; exit 0; }; fail "missing/incompatible prerequisite $tool (Python 3.11+ required); bootstrap does not install dependencies"; }; done
[ "$STAGE" = dependencies ] && { echo 'Dependency detection only; explicit setup belongs to dot init.'; exit 0; }

# This Python payload is installer logic, not private configuration. It performs
# preflight checks and atomic, ownership-checked installation of local metadata.
local_setup() {
  python3 - --action "$1" --home "$HOME" --directory "$DIRECTORY" \
    --source "$SOURCE_URL" --profile "$PROFILE" --brew-bin "$BREW_BIN" \
    --python-bin "$PYTHON_BIN" --zsh-directory "$ZSH_DIRECTORY" --yes "$YES" --replace "$REPLACE_CHEZMOI" --engine-directory "$ENGINE_DIRECTORY" <<'PY'
import argparse, hashlib, json, os, pathlib, re, shlex, shutil, stat, subprocess, sys, tempfile, tomllib, uuid
parser = argparse.ArgumentParser(add_help=False)
for name in ('action', 'home', 'directory', 'source', 'profile', 'brew-bin', 'python-bin', 'zsh-directory', 'yes', 'replace', 'engine-directory'):
    parser.add_argument('--' + name, required=True)
settings = parser.parse_args()
action, source, profile = settings.action, settings.source, settings.profile
home = pathlib.Path(settings.home)
checkout = pathlib.Path(settings.directory)
engine = pathlib.Path(settings.engine_directory)
state = home / '.local/state/dot/install.json'
scope = home / '.local/state/dot/adoption.json'
current_cache = home / '.local/state/dot/chezmoi-cache'
current_state = home / '.local/state/dot/chezmoi-state.boltdb'
config = pathlib.Path(os.environ.get('XDG_CONFIG_HOME', str(home / '.config'))) / 'chezmoi/chezmoi.toml'
launcher = home / '.local/bin/dot'
begin, end = '# >>> dot installer PATH >>>', '# <<< dot installer PATH <<<'

def fail(message):
    sys.exit('dot installer: ' + message)

def reject_symlink_redirects(path):
    # Reject user-owned symlink redirects, including ancestor redirects. System
    # /var -> /private/var links outside HOME are not configuration ownership.
    for part in [path, *path.parents]:
        if part == home and part.is_symlink():
            fail(f'path conflict (symlink): {part}; left untouched')
        if part == home:
            break
        if part.is_symlink():
            fail(f'path conflict (symlink): {part}; left untouched')

def atomic(path, text, mode=0o600):
    reject_symlink_redirects(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, name = tempfile.mkstemp(dir=path.parent)
    try:
        with os.fdopen(fd, 'w') as stream:
            stream.write(text)
        os.chmod(name, mode)
        os.replace(name, path)
    finally:
        if os.path.exists(name):
            os.unlink(name)

def git(*args):
    return subprocess.check_output(['git', '-C', str(checkout), *args], text=True).strip()

def strip_startup_comment(line):
    quote, escaped = None, False
    for index, character in enumerate(line):
        if escaped:
            escaped = False
        elif character == '\\' and quote != "'":
            escaped = True
        elif quote:
            if character == quote:
                quote = None
        elif character in ("'", '"'):
            quote = character
        elif character == '#' and (index == 0 or line[index - 1].isspace()):
            return line[:index]
    return line

def zsh_startup_directory():
    supplied = settings.zsh_directory or os.environ.get('ZDOTDIR')
    if supplied:
        return pathlib.Path(supplied)
    # Never launch Zsh/source .zshenv to discover a path: startup code can run
    # applications and services. Interpret only plain assignments as data.
    startup = home / '.zshenv'
    values = {**os.environ, 'HOME': str(home)}
    diagnostic = 'cannot safely discover ZDOTDIR from .zshenv; provide --zsh-directory ABSOLUTE_PATH or export the configured ZDOTDIR; startup code was not executed'
    if startup.exists():
        for line in startup.read_text().splitlines():
            line = strip_startup_comment(line).strip()
            if not line:
                continue
            assignment = re.fullmatch(r'(?:export[ \t]+)?([A-Za-z_][A-Za-z_0-9]*)=(.*)', line)
            if not assignment:
                fail(diagnostic)
            key, value = assignment.groups()
            quote = value[:1] if value[:1] in ("'", '"') else ''
            if quote:
                if len(value) < 2 or value[-1] != quote or quote in value[1:-1] or '\\' in value:
                    fail(diagnostic)
                value = value[1:-1]
            elif re.search(r'[\s\'"\\;|&<>]', value):
                fail(diagnostic)
            if quote != "'":
                if re.search(r'`|\$\(', value):
                    fail(diagnostic)
                def expand(match):
                    variable = match.group(1) or match.group(2)
                    if variable not in values:
                        fail(diagnostic)
                    return values[variable]
                value = re.sub(r'\$\{([A-Za-z_][A-Za-z_0-9]*)\}|\$([A-Za-z_][A-Za-z_0-9]*)', expand, value)
                if '$' in value:
                    fail(diagnostic)
                if not quote and (value == '~' or value.startswith('~/')):
                    value = str(home) + value[1:]
            values[key] = value
    return pathlib.Path(values.get('ZDOTDIR', str(home)))

paths = [str(home / '.local/bin'), settings.python_bin, settings.brew_bin]
paths = [p for p in paths if p]
path_assignment = 'export PATH=' + shlex.quote(':'.join(paths)) + ':"$PATH"'
block = begin + '\n' + path_assignment + '\n' + end + '\n'
zdot = zsh_startup_directory()
if not zdot.is_absolute():
    fail('ZDOTDIR must be absolute')
login = next((home / p for p in ('.bash_profile', '.bash_login', '.profile') if (home / p).exists()), home / '.bash_profile')
startup = list(dict.fromkeys([login, home / '.bashrc', zdot / '.zprofile', zdot / '.zshrc']))
expected = '#!/usr/bin/env bash\nexport DOT_SOURCE=' + shlex.quote(str(checkout)) + '\nexport DOT_ENGINE=' + shlex.quote(str(engine / 'tools/dot')) + '\nexport DOT_BROWSER=' + shlex.quote(str(engine / 'bin/dot-browser')) + '\nexport DOT_CONFIG=' + shlex.quote(str(config)) + '\nexec ' + shlex.quote(str(engine / 'tools/dot/dot')) + ' "$@"\n'

def installation_record():
    # Bootstrap detects only the split installation record; dot init owns the tracker.
    for path in [state, scope, config, launcher, checkout, engine, current_cache, current_state, *startup]:
        reject_symlink_redirects(path)
    if not state.exists():
        return None
    record = json.loads(state.read_text())
    if record.get('version') != 2 or record.get('directory') != str(checkout) or record.get('engine') != str(engine):
        fail('installation record conflicts with requested checkout/source/config; left untouched')
    if config.exists():
        data = tomllib.loads(config.read_text())
        if data.get('sourceDir') not in (str(checkout), str(checkout / 'home')):
            fail('chezmoi source conflicts with installation record; migration needed')
        if data.get('data', {}).get('dotEngine') != str(engine / 'tools/dot'):
            fail('chezmoi dotEngine conflicts with installation record; migration needed')
    return record

def present(path):
    return path.exists() or path.is_symlink()

def check_manager_migration():
    journal = home / '.local/state/dot/manager-migration.json'
    owned(journal)
    if journal.exists():
        progress = json.loads(journal.read_text())
        if progress.get('stage') != 'complete':
            fail('incomplete retirement: current manager migration needs manual recovery; completed=' + ', '.join(progress.get('completed', [])) + '; pending=' + ', '.join(progress.get('pending', [])) + '; backup=' + progress.get('backup', '(see journal)') + '; inspect/restore recorded snapshots and reconcile manager-migration.json before retrying; no unsafe retry')

def owned(path):
    reject_symlink_redirects(path)
    if not path.is_relative_to(home) or path == home:
        fail(f'incomplete retirement: unsupported ownership outside HOME: {path}; review manually')
    for item in [path, *path.parents]:
        if present(item) and item.lstat().st_uid != os.getuid():
            fail(f'incomplete retirement: unknown ownership: {item}; left untouched')
        if item == home:
            break

def inventory(path):
    # No symlink traversal, Git commands, hooks, filters, or secret rendering.
    items = [path]
    if path.is_dir():
        for root, dirs, files in os.walk(path, followlinks=False):
            items.extend(pathlib.Path(root) / name for name in dirs + files)
    digest = hashlib.sha256()
    for item in sorted(set(items)):
        info = item.lstat()
        if info.st_uid != os.getuid():
            fail(f'incomplete retirement: unknown ownership: {item}')
        digest.update(str(item.relative_to(path)).encode())
        if stat.S_ISLNK(info.st_mode):
            digest.update(b'L' + os.readlink(item).encode())
        elif stat.S_ISDIR(info.st_mode):
            digest.update(b'D')
        elif stat.S_ISREG(info.st_mode):
            digest.update(b'F')
            with item.open('rb') as stream:
                for chunk in iter(lambda: stream.read(1024 * 1024), b''):
                    digest.update(chunk)
        else:
            fail(f'incomplete retirement: unsupported special file: {item}')
    return digest.hexdigest()

def reject_nested_repositories(directory, allow_root=False):
    for root, dirs, files in os.walk(directory, followlinks=False):
        bare_repository = 'HEAD' in files and 'config' in files and 'objects' in dirs
        if ('.git' in dirs + files and (not allow_root or pathlib.Path(root) != directory)) or bare_repository:
            fail(f'incomplete retirement: nested unrelated repository in {root}; left untouched; review manually')
        if '.git' in dirs:
            dirs.remove('.git')

def verify_git_metadata(metadata, source=False):
    reject_symlink_redirects(metadata)
    shared = ('commondir', 'worktrees', 'objects/info/alternates')
    if not metadata.is_dir() or any(present(metadata / name) for name in shared):
        label = 'source Git' if source else 'Git'
        fail(f'incomplete retirement: linked/shared {label} metadata is unsupported: {metadata}; left untouched; review manually')

def manager_layout(record):
    if record is None:
        return None
    data = tomllib.loads(config.read_text())
    if data.get('hooks'):
        fail('incomplete retirement: current manager hook configuration requires manual review; hooks were not executed')
    if any(key in data and not isinstance(data[key], str) for key in ('cacheDir', 'persistentState')):
        fail('incomplete retirement: invalid current manager data paths; review manually')
    cache = pathlib.Path(data.get('cacheDir') or str(pathlib.Path(os.environ.get('XDG_CACHE_HOME', str(home / '.cache'))) / 'chezmoi'))
    database = pathlib.Path(data.get('persistentState') or str(config.parent / 'chezmoistate.boltdb'))
    cache_defaults = {home / '.cache/chezmoi', pathlib.Path(os.environ.get('XDG_CACHE_HOME', str(home / '.cache'))) / 'chezmoi', current_cache}
    state_defaults = {config.parent / 'chezmoistate.boltdb', config.parent / 'chezmoi.boltdb', current_state}
    if cache not in cache_defaults or database not in state_defaults:
        fail('incomplete retirement: current manager data ownership is ambiguous; review manually; paths left untouched')
    for path in (config, cache, database):
        owned(path)
    if present(cache):
        if not cache.is_dir():
            fail('incomplete retirement: current manager cache is not a directory; review manually')
        if cache != current_cache:
            reject_nested_repositories(cache)
    if present(database) and not database.is_file():
        fail('incomplete retirement: current manager state is not a regular file; review manually')
    return {'cache': cache, 'state': database}

def replacement_consent(prompt, cancelled):
    if settings.yes == 'true':
        if settings.replace != 'true':
            fail('--yes additionally requires --replace-chezmoi; existing management intact; no configs applied')
    else:
        with open('/dev/tty', 'w') as output, open('/dev/tty') as input:
            output.write(prompt + ' [y/N] ')
            output.flush()
            if input.readline().strip().lower() not in ('y', 'yes'):
                fail(cancelled + '; existing management intact; no configs applied')

def private_snapshot(path, target):
    owned(path)
    digest = inventory(path)
    if path.is_dir():
        shutil.copytree(path, target, symlinks=True)
    else:
        shutil.copy2(path, target, follow_symlinks=False)
    if inventory(target) != digest or inventory(path) != digest:
        raise RuntimeError('current manager data changed during backup: ' + str(path))
    items = [target]
    if target.is_dir():
        for root, dirs, names in os.walk(target, followlinks=False):
            items += [pathlib.Path(root), *(pathlib.Path(root) / n for n in dirs + names)]
    for item in items:
        if not item.is_symlink():
            item.chmod(item.stat().st_mode & 0o700)
    return digest

def migrate_current_manager(record):
    if record is None or 'cache' in record:
        return
    if action != 'preflight':
        fail('incomplete retirement: current manager migration must complete during approved preflight; rerun')
    active = manager_layout(record)
    journal = home / '.local/state/dot/manager-migration.json'
    if journal.exists():
        fail('incomplete retirement: current manager metadata changed after completed migration; review manually; prior backup preserved')
    for path in (current_cache, current_state):
        owned(path)
        if present(path):
            fail(f'incomplete retirement: dedicated current manager destination conflict: {path}; left untouched; review manually')
    with config.open(newline='') as stream:
        original = stream.read()
    # Change only supported top-level path assignments, preserving all machine
    # data, comments and other settings byte-for-byte. Never source old config.
    root, sections, in_section = [], [], False
    replacements = {'cacheDir': current_cache, 'persistentState': current_state}
    replaced = set()
    assignment = re.compile(r'''([ \t]*(cacheDir|persistentState)[ \t]*=[ \t]*)("(?:[^"\\\r\n]|\\[^\r\n])*"|'[^'\r\n]*')([ \t]*(?:\#[^\r\n]*)?(?:\r\n|\n|\r)?)''')
    for line in original.splitlines(keepends=True):
        if line.lstrip().startswith('['):
            in_section = True
        if not in_section and re.match(r'\s*(cacheDir|persistentState)\s*=', line):
            match = assignment.fullmatch(line)
            if not match:
                fail('incomplete retirement: unsupported current manager path assignment; review manually; config left untouched')
            prefix, key, _, suffix = match.groups()
            line = prefix + json.dumps(str(replacements[key])) + suffix
            replaced.add(key)
        (sections if in_section else root).append(line)
    prefix = ''.join(root)
    additions = ''.join(key + ' = ' + json.dumps(str(path)) + '\n' for key, path in replacements.items() if key not in replaced)
    updated = prefix + ('\n' if additions and prefix and not prefix.endswith('\n') else '') + additions + ''.join(sections)
    try:
        candidate = tomllib.loads(updated)
    except tomllib.TOMLDecodeError:
        fail('incomplete retirement: unsupported current manager path assignment; review manually; config left untouched')
    expected_data = {**tomllib.loads(original), 'cacheDir': str(current_cache), 'persistentState': str(current_state)}
    if candidate != expected_data:
        fail('incomplete retirement: unsupported current manager path assignment; review manually; config left untouched')
    backup = home / '.local/state/dot' / ('manager-backup-' + uuid.uuid4().hex)
    print('WARNING: exact current manager data migration and private rollback plan:', flush=True)
    entries = [('config', config, config), ('installation', state, state), ('selection', scope, scope)]
    for name, target in (('cache', current_cache), ('state', current_state)):
        old = active[name]
        if present(old):
            entries.append((name, old, target))
        print(f'  Current {name}: {old} -> {target}; private backup {backup}/{name} if present', flush=True)
    for name, path, _ in entries[:3]:
        print(f'  Preserve/backup {name}: {path} -> {backup}/{name}', flush=True)
    print('Only cache/state paths and their installation metadata change; active selection and machine data remain unchanged. No configurations are adopted.', flush=True)
    replacement_consent('Separately approve private backup and current manager data migration?', 'cancelled current manager migration')
    progress = {'version': 1, 'backup': str(backup), 'stage': 'backup', 'completed': [], 'pending': [name for name, _, _ in entries if name != 'selection'], 'plan': [{'name': name, 'path': str(path), 'destination': str(target), 'snapshot': name} for name, path, target in entries]}
    def save():
        atomic(journal, json.dumps(progress, indent=2) + '\n')
    try:
        backup.mkdir(mode=0o700)
        save()
        digests = {name: private_snapshot(path, backup / name) for name, path, _ in entries}
        progress['stage'] = 'migration'
        save()
        print('Current manager recovery backups verified before migration: ' + str(backup), flush=True)
        for name, path, target in entries[3:]:
            owned(path)
            if inventory(path) != digests[name] or present(target):
                raise RuntimeError('current manager source/destination changed: ' + str(path))
            path.rename(target)
            progress['completed'].append(name)
            progress['pending'].remove(name)
            save()
        if inventory(config) != digests['config'] or inventory(scope) != digests['selection'] or inventory(state) != digests['installation']:
            raise RuntimeError('current config/selection/installation changed during migration; review manually')
        atomic(config, updated)
        progress['completed'].append('config')
        progress['pending'].remove('config')
        save()
        migrated = {**record, 'cache': str(current_cache), 'persistent_state': str(current_state)}
        atomic(state, json.dumps(migrated, indent=2) + '\n')
        progress['completed'].append('installation')
        progress['pending'].remove('installation')
        progress['stage'] = 'complete'
        save()
        print('Current manager data migration complete; recovery backup: ' + str(backup), flush=True)
    except Exception as error:
        fail('incomplete retirement: current manager migration failed: ' + str(error) + '; completed=' + ', '.join(progress['completed']) + '; pending=' + ', '.join(progress['pending']) + '; backup=' + str(backup) + '; inspect/restore recorded snapshots and reconcile manager-migration.json manually; no unsafe retry or backup overwrite')

def legacy_plan(record):
    git_overrides = ('GIT_DIR', 'GIT_WORK_TREE', 'GIT_COMMON_DIR', 'GIT_INDEX_FILE', 'GIT_OBJECT_DIRECTORY', 'GIT_ALTERNATE_OBJECT_DIRECTORIES', 'GIT_CONFIG', 'GIT_CONFIG_PARAMETERS')
    overrides = [k for k, v in os.environ.items() if v and (k.startswith('CHEZMOI_') or k in ('DOT_SOURCE', 'DOT_CONFIG', *git_overrides) or (k == 'GIT_CONFIG_COUNT' and v != '0'))]
    # Overrides are discovered, not guessed or sourced. They would remain active
    # in the parent shell, so require owner removal before claiming retirement.
    if overrides:
        fail('migration needed: management overrides ' + ', '.join(sorted(overrides)) + '; incomplete retirement: review/unset them and encode custom paths in TOML/JSON before retrying; no old hooks executed')
    paths = {}
    sources = set()
    active = manager_layout(record)
    excluded = {config, checkout, *active.values()} if active else set()
    config_dirs = {config.parent, home / '.config/chezmoi'}
    caches = {home / '.cache/chezmoi', pathlib.Path(os.environ.get('XDG_CACHE_HOME', str(home / '.cache'))) / 'chezmoi'}
    states = {directory / name for directory in config_dirs for name in ('chezmoi.boltdb', 'chezmoistate.boltdb')}
    def add(path, kind):
        path = pathlib.Path(path)
        if path in excluded:
            return
        if not path.is_absolute() or '..' in path.parts:
            fail('incomplete retirement: custom manager paths must be absolute, without traversal')
        containers = {home, home / '.config', home / '.local', home / '.local/share', home / '.local/state', home / '.cache'}
        containers.update(pathlib.Path(os.environ[k]) for k in ('XDG_CONFIG_HOME', 'XDG_DATA_HOME', 'XDG_CACHE_HOME', 'XDG_STATE_HOME') if os.environ.get(k))
        protected = [home / name for name in ('.ssh', '.gnupg', '.aws', '.kube', '.password-store', '.git-credentials', '.gitignore', '.gitconfig', '.netrc', '.codex', '.agents', '.zshenv', '.zsh_history', '.bash_history', '.config/gh', '.config/opencode', '.config/secrets.env', '.local/share/keyrings', '.local/state/dot')]
        protected.extend(startup)
        protected.extend(pathlib.Path(os.environ.get('XDG_CONFIG_HOME', str(home / '.config'))) / name for name in ('gh', 'opencode', 'secrets.env', 'git'))
        if path in containers or any(path == p or path.is_relative_to(p) or p.is_relative_to(path) for p in protected):
            fail(f'incomplete retirement: protected live path: {path}; left untouched; review manually')
        if present(path):
            owned(path)
            # A path declared in old configuration is not independent evidence
            # of manager ownership. Only conventional default/XDG namespaces
            # are automatically supported; arbitrary custom data needs review.
            if kind == 'chezmoi cache':
                if path not in caches or not path.is_dir():
                    fail(f'incomplete retirement: cache manager ownership is ambiguous: {path}; left untouched; review manually')
                reject_nested_repositories(path)
            elif kind == 'chezmoi state':
                if path not in states or not path.is_file():
                    fail(f'incomplete retirement: state manager ownership is ambiguous: {path}; left untouched; review manually')
            paths[path] = kind
    add(home / '.git', 'HOME Git metadata (owner-authorized; live files and .gitignore preserved)')
    for directory in sorted(config_dirs):
        reject_symlink_redirects(directory)
        for suffix in ('toml', 'json', 'yaml', 'yml', 'jsonc'):
            old = directory / ('chezmoi.' + suffix)
            if old in excluded or not present(old):
                continue
            add(old, 'chezmoi config')
            if suffix not in ('toml', 'json'):
                fail(f'incomplete retirement: unsupported config format: {old}; convert to TOML/JSON manually without executing hooks')
            data = tomllib.loads(old.read_text()) if suffix == 'toml' else json.loads(old.read_text())
            if not isinstance(data, dict):
                fail('incomplete retirement: manager configuration must be an object')
            for key, kind in (('sourceDir', 'source'), ('cacheDir', 'cache'), ('persistentState', 'state')):
                if key in data:
                    if not isinstance(data[key], str):
                        fail('incomplete retirement: invalid custom manager path: ' + key)
                    path = pathlib.Path(data[key])
                    if key == 'sourceDir':
                        sources.add(path)
                    else:
                        add(path, 'chezmoi ' + kind)
            working = data.get('workingTree')
            if working and working != data.get('sourceDir'):
                if working != str(home):
                    fail('incomplete retirement: separate workingTree ownership is ambiguous; review manually')
        for name in ('chezmoi.boltdb', 'chezmoistate.boltdb'):
            add(directory / name, 'chezmoi state')
    sources.update({home / '.local/share/chezmoi', pathlib.Path(os.environ.get('XDG_DATA_HOME', str(home / '.local/share'))) / 'chezmoi'})
    for directory in caches:
        add(directory, 'chezmoi cache')
    for directory in sorted(sources):
        if directory == home or directory in excluded:
            continue  # Never HOME, its tracked files, or its .gitignore.
        if not present(directory):
            continue
        owned(directory)
        if directory == checkout or directory in checkout.parents or home.is_relative_to(directory):
            fail('incomplete retirement: source overlaps HOME/new checkout; review manually')
        # A configured path is not proof an arbitrary project belongs to chezmoi.
        defaults = {home / '.local/share/chezmoi', pathlib.Path(os.environ.get('XDG_DATA_HOME', str(home / '.local/share'))) / 'chezmoi'}
        if directory not in defaults or not directory.is_dir() or (not any(p.name.startswith(('dot_', 'private_dot_')) or p.name in ('.chezmoiignore', '.chezmoi.toml.tmpl', '.chezmoi.json.tmpl', '.chezmoidata.toml', '.chezmoidata.json') for p in directory.iterdir()) and any(directory.iterdir())):
            fail(f'incomplete retirement: source ownership is ambiguous: {directory}; review manually')
        reject_nested_repositories(directory, allow_root=True)
        metadata = directory / '.git'
        if present(metadata):
            verify_git_metadata(metadata, source=True)
        add(directory, 'verified chezmoi source (including dirty/untracked work)')
    for path in paths:
        if path == checkout or path in checkout.parents or state.is_relative_to(path):
            fail(f'incomplete retirement: retirement path overlaps new installation: {path}')
        if path.name == '.git':
            verify_git_metadata(path)
    # Collapse contained entries so each path has exactly one snapshot/move.
    roots = [p for p in paths if not any(q != p and q in p.parents for q in paths)]
    return [(p, paths[p], inventory(p)) for p in sorted(roots)]

def git_integrations(roots):
    changes, seen = [], set()
    def references(value):
        value = value.replace('${HOME}', str(home)).replace('$HOME', str(home)).replace('~/', str(home) + '/')
        return any(re.search(r'(?<![\w./-])' + re.escape(str(path)) + r'(?=$|[/\s\"\';])', value) for path in roots)
    def is_include(key):
        return key == 'include.path' or (key.startswith('includeif.') and key.endswith('.path'))
    def is_integration(key):
        return key == 'core.hookspath' or key.startswith('alias.') or is_include(key)
    def include_path(path, value):
        include = pathlib.Path(value.replace('~/', str(home) + '/', 1))
        include = include if include.is_absolute() else path.parent / include
        return pathlib.Path(os.path.normpath(include))
    def entries(path):
        reject_symlink_redirects(path)
        result = subprocess.run(['git', 'config', '--file', str(path), '--no-includes', '--null', '--list'], capture_output=True, text=True)
        if result.returncode:
            fail(f'incomplete retirement: cannot parse Git configuration: {path}; review manually')
        return [entry.partition('\n')[::2] for entry in result.stdout.split('\0') if entry]
    def verify_retired_include(path, visited):
        if path in visited or not present(path):
            return
        visited.add(path)
        for key, value in entries(path):
            if not is_integration(key) or not references(value):
                fail(f'incomplete retirement: legacy include also supplies preserved Git settings ({key}): {path}; relocate those settings manually before retrying')
            if is_include(key):
                verify_retired_include(include_path(path, value), visited)
    def read(path):
        if path in seen or not present(path):
            return
        seen.add(path)
        for key, value in entries(path):
            integration = is_integration(key)
            if references(value) or (is_include(key) and references(str(include_path(path, value)))) or (key.startswith('includeif.') and references(key)):
                if not integration:
                    fail(f'incomplete retirement: preserved Git setting {key} references a retired path; review manually: {path}')
                if is_include(key):
                    verify_retired_include(include_path(path, value), set())
                owned(path)
                changes.append((path, key, value))
            elif integration and re.search(r'chezmoi|dotfiles|GIT_DIR|GIT_WORK_TREE', value, re.I):
                fail(f'incomplete retirement: ambiguous Git integration {key} in {path}; review/remove manually')
            elif is_include(key):
                read(include_path(path, value))
    global_files = [home / '.gitconfig', pathlib.Path(os.environ.get('XDG_CONFIG_HOME', str(home / '.config'))) / 'git/config']
    if os.environ.get('GIT_CONFIG_GLOBAL'):
        global_files.append(pathlib.Path(os.environ['GIT_CONFIG_GLOBAL']))
    for path in global_files:
        read(path)
    return list(dict.fromkeys(changes))

def automation_inventory():
    paths = []
    for directory, pattern in ((home / '.config/systemd/user', '*'), (home / 'Library/LaunchAgents', '*.plist'), (home / '.config/cron', '*')):
        if any(part.is_symlink() for part in [directory, *directory.parents] if part.is_relative_to(home)):
            paths.append(str(directory) + ' (symlink redirect: inspect manually)')
            continue
        if directory.is_dir():
            for path in sorted(directory.glob(pattern)):
                if path.is_symlink():
                    paths.append(str(path) + ' (symlink: inspect manually)')
                elif path.is_file() and (re.search('chezmoi|dotfiles', path.name, re.I) or re.search('chezmoi|dotfiles', path.read_text(errors='replace'), re.I)):
                    paths.append(str(path))
    for path in (home / '.crontab', home / '.config/crontab'):
        if present(path):
            paths.append(str(path))
    for path in paths:
        print('Owner action required: ' + path + '; inspect and disable/remove legacy automation yourself before adoption. Left untouched.', flush=True)
    print('Service/cron runtime state and remote machines were not queried or changed; automation retirement remains unverified.', flush=True)
    return paths

def retire_legacy(record):
    journal = home / '.local/state/dot/retirement.json'
    owned(journal)
    previous = json.loads(journal.read_text()) if journal.exists() else None
    if previous and (previous.get('version') != 1 or not isinstance(previous.get('backup'), str) or not isinstance(previous.get('completed'), list) or not isinstance(previous.get('pending'), list)):
        fail('incomplete retirement: invalid retirement journal; review manually; backups left untouched')
    if previous and previous.get('stage') != 'complete':
        fail('incomplete retirement: prior attempt needs manual recovery; completed=' + ', '.join(previous.get('completed', [])) + '; pending=' + ', '.join(previous.get('pending', [])) + '; backup=' + previous['backup'] + '; restore snapshots to the recorded original paths, reconcile retirement.json, then retry (no backup overwritten)')
    plan = legacy_plan(record)
    roots = [p for p, _, _ in plan]
    if previous:
        roots += [pathlib.Path(entry['path']) for entry in previous.get('plan', [])]
    changes = git_integrations(roots)
    if not plan:
        if changes:
            fail('incomplete retirement: Git integrations referencing previously retired paths reappeared; review manually; prior backup preserved')
        if previous:
            automation_inventory()
        return
    if record is not None:
        fail('migration needed: incomplete retirement: newly discovered legacy metadata on installed target: ' + ', '.join(str(p) for p, _, _ in plan) + '; review manually; active selection and prior backups preserved; no repeated retirement')
    if action != 'preflight':
        fail('incomplete retirement: legacy metadata appeared after approved preflight; rerun')
    backup = home / '.local/state/dot' / ('legacy-backup-' + uuid.uuid4().hex)
    owned(backup)
    print('WARNING: migration needed: legacy HOME/chezmoi setup. Exact retirement plan:', flush=True)
    for index, (path, kind, _) in enumerate(plan):
        print(f'  {path} -> {backup}/originals/{index} ({kind}); private snapshot {backup}/snapshots/{index}', flush=True)
    files = list(dict.fromkeys(path for path, _, _ in changes))
    for index, path in enumerate(files):
        keys = ', '.join(key for file, key, _ in changes if file == path)
        print(f'  Git entries only: {path}: {keys}; private backup {backup}/git/{index}', flush=True)
    print('HOME, live tracked files, HOME .gitignore, unrelated repositories, credentials, signing and unselected configs are preserved. No hooks or old sync commands will run.', flush=True)
    print('Owner must retire old sync timers/cron/launch automation manually; installer does not change service runtime state or remote machines.', flush=True)
    automation = automation_inventory()
    replacement_consent('Separately approve private backup and legacy retirement?', 'cancelled legacy replacement')
    if previous:
        fail('incomplete retirement: new legacy metadata after completed retirement; review manually; prior backup preserved')
    pending = [str(p) for p, _, _ in plan] + [f'Git entries: {p}' for p in files]
    progress = {'version': 1, 'backup': str(backup), 'stage': 'backup', 'completed': [], 'pending': pending, 'automation_owner_actions': automation, 'plan': [{'path': str(p), 'snapshot': f'snapshots/{i}', 'original': f'originals/{i}'} for i, (p, _, _) in enumerate(plan)], 'git_plan': [{'path': str(p), 'snapshot': f'git/{i}', 'keys': [key for file, key, _ in changes if file == p]} for i, p in enumerate(files)]}
    def save():
        atomic(journal, json.dumps(progress, indent=2) + '\n')
    try:
        backup.mkdir(parents=True, mode=0o700)
        for name in ('snapshots', 'originals', 'git'):
            (backup / name).mkdir(mode=0o700)
        save()
        for index, (path, _, digest) in enumerate(plan):
            target = backup / 'snapshots' / str(index)
            if path.is_dir():
                shutil.copytree(path, target, symlinks=True)
            else:
                shutil.copy2(path, target, follow_symlinks=False)
            if inventory(target) != digest or inventory(path) != digest:
                raise RuntimeError('source changed during backup: ' + str(path))
        for index, path in enumerate(files):
            shutil.copy2(path, backup / 'git' / str(index))
            if path.read_bytes() != (backup / 'git' / str(index)).read_bytes():
                raise RuntimeError('Git config changed during backup: ' + str(path))
        # Strip group/other permissions, without following symlink destinations.
        for root, dirs, names in os.walk(backup, followlinks=False):
            for item in [pathlib.Path(root), *(pathlib.Path(root) / n for n in dirs + names)]:
                if not item.is_symlink():
                    item.chmod(item.stat().st_mode & 0o700)
        progress['stage'] = 'retirement'
        save()
        print('Private recovery backups verified before retirement: ' + str(backup), flush=True)
        for index, path in enumerate(files):
            candidate = backup / 'git' / (str(index) + '.new')
            shutil.copy2(backup / 'git' / str(index), candidate)
            for file, key, value in changes:
                if file == path:
                    subprocess.run(['git', 'config', '--file', str(candidate), '--fixed-value', '--unset-all', key, value], check=True, capture_output=True)
            if path.read_bytes() != (backup / 'git' / str(index)).read_bytes():
                raise RuntimeError('Git config changed during backup: ' + str(path))
            atomic(path, candidate.read_text())
            label = 'Git entries: ' + str(path)
            progress['completed'].append(label)
            progress['pending'].remove(label)
            save()
        for index, (path, _, digest) in enumerate(plan):
            owned(path)
            if inventory(path) != digest:
                raise RuntimeError('source changed before retirement: ' + str(path))
            # Same-filesystem rename only: no recursive deletion or source-wide
            # Git operation. Cross-device layouts fail with their backup intact.
            path.rename(backup / 'originals' / str(index))
            progress['completed'].append(str(path))
            progress['pending'].remove(str(path))
            save()
        progress['stage'] = 'complete'
        save()
        print('Legacy metadata retirement complete; recovery backup: ' + str(backup), flush=True)
    except Exception as error:
        diagnostic = f'Git configuration edit failed (exit {error.returncode}; private values withheld)' if isinstance(error, subprocess.CalledProcessError) else str(error)
        recovery = 'backup stage failed; active retirement has not started; inspect partial backups and permissions manually' if progress['stage'] == 'backup' else 'restore snapshots/recorded Git configs manually before retrying'
        fail('incomplete retirement: ' + diagnostic + '; completed=' + ', '.join(progress['completed']) + '; pending=' + ', '.join(progress['pending']) + '; recovery backup=' + str(backup) + '; ' + recovery + '; no automatic rollback or unsafe removal retry')

def check_checkout_and_launcher():
    if launcher.exists() and (not launcher.is_file() or launcher.read_text() != expected):
        fail('unrelated launcher conflict; left untouched: ' + str(launcher))
    if checkout.exists():
        reject_symlink_redirects(checkout / '.git')
        if not (checkout / '.git').is_dir() or pathlib.Path(git('rev-parse', '--show-toplevel')).resolve() != checkout.resolve() or git('remote', 'get-url', 'origin') != source:
            fail('unrelated checkout conflict; left untouched: ' + str(checkout))
        git('symbolic-ref', '--quiet', '--short', 'HEAD')
        if not git('rev-parse', '--abbrev-ref', '@{upstream}').startswith('origin/'):
            fail('checkout upstream is not origin; left untouched')
    if engine.exists():
        reject_symlink_redirects(engine / '.git')
        if not (engine / '.git').is_dir() or subprocess.check_output(['git', '-C', str(engine), 'remote', 'get-url', 'origin'], text=True).strip() != os.environ['DOT_ENGINE_URL']:
            fail('unrelated public engine checkout conflict; left untouched: ' + str(engine))
        if not (engine / 'tools/dot/dot').is_file(): fail('public engine checkout is incomplete; left untouched')
    for candidate in (home / '.local/share/dot',):
        if candidate.exists(): fail('legacy installation detected at ' + str(candidate) + '; explicit split migration required; left untouched')

def check_contract():
    for path in (engine / 'tools/dot/dot', engine / 'tools/dot/contract.py'):
        reject_symlink_redirects(path)
        if not path.is_file():
            fail('public engine entrypoint is missing; no install metadata or launcher changed')
    if not os.access(engine / 'tools/dot/dot', os.X_OK):
        fail('public engine CLI is not executable; no install metadata or launcher changed')
    if not (engine / 'tools/dot/dot').is_file() or not (engine / 'tools/dot/contract.py').is_file():
        fail('public engine contract checker is missing; no install metadata or launcher changed')
    if not (checkout / 'dot-contract.json').is_file():
        fail('private source contract is missing; no install metadata or launcher changed')
    contract_runner = ['python3', str(engine / 'tools/dot/contract.py'), 'check', '--engine', str(engine / 'tools/dot'), '--source', str(checkout)]
    subprocess.run(contract_runner, check=True)
    config_data = tomllib.loads(config.read_text()) if config.exists() else {}
    if record is not None and config.exists() and config_data.get('data', {}).get('dotEngine') != str(engine / 'tools/dot'):
        fail('installed engine contract path conflicts with config; explicit migration required')

def check_startup_markers():
    for path in startup:
        text = path.read_text() if path.exists() else ''
        if text.count(begin) != text.count(end) or text.count(begin) > 1 or (begin in text and text.index(begin) > text.index(end)):
            fail('malformed PATH markers; left untouched: ' + str(path))

def configure_chezmoi(record):
    if not (engine / 'tools/dot/dot').is_file() or not os.access(engine / 'tools/dot/dot', os.X_OK): fail('public engine lacks an executable dot implementation')
    if record is None:
        if not (checkout / 'home/.chezmoiignore').is_file():
            fail('checkout lacks dedicated home/ source; no unscoped installer fallback')
        subprocess.run([str(engine / 'tools/dot/dot'), 'scope', 'init'], env={**os.environ, 'DOT_SOURCE': str(checkout), 'DOT_ENGINE': str(engine / 'tools/dot')}, check=True)
        name = subprocess.check_output(['git', 'config', 'user.name'], text=True).strip()
        email = subprocess.check_output(['git', 'config', 'user.email'], text=True).strip()
        atomic(config, 'sourceDir = ' + json.dumps(str(checkout / 'home')) + '\ncacheDir = ' + json.dumps(str(current_cache)) + '\npersistentState = ' + json.dumps(str(current_state)) + '\n[data]\nname = ' + json.dumps(name) + '\nemail = ' + json.dumps(email) + '\nprofile = ' + json.dumps(profile) + '\ndotEngine = ' + json.dumps(str(engine / 'tools/dot')) + '\n')
    elif (checkout / 'home').is_dir() and tomllib.loads(config.read_text()).get('sourceDir') == str(checkout):
        sys.path.insert(0, str(engine / 'tools/dot'))
        sys.dont_write_bytecode = True
        import layout
        proposal = layout.proposed(checkout, home, config)
        layout.display(proposal)
        replacement_consent('Approve this source layout migration with private backups and unchanged selection?', 'source migration canceled')
        layout.approved(checkout, home, config, proposal, proposal['id'])
    active_source = checkout / 'home' if (checkout / 'home').is_dir() else checkout
    subprocess.run(['chezmoi', '--source', str(active_source), '--destination', str(home), '--config', str(config), 'status'], check=True)

def install_launcher_and_path():
    atomic(launcher, expected, 0o755)
    for path in startup:
        text = path.read_text() if path.exists() else ''
        if begin in text:
            first, rest = text.split(begin, 1)
            _, last = rest.split(end, 1)
            # Replace the marker's own line ending, not unrelated blank lines.
            text = first + block + (last[1:] if last.startswith('\n') else last)
        else:
            text += ('\n' if text and not text.endswith('\n') else '') + block
        atomic(path, text, path.stat().st_mode & 0o777 if path.exists() else 0o600)
    print('Installed launcher: ' + str(launcher))
    print('Open a new Bash/Zsh shell, or activate this terminal with:')
    print('  ' + path_assignment)
    print('The installer cannot modify its parent shell; your default shell was not changed.')

record = installation_record()
if checkout == engine or checkout in engine.parents or engine in checkout.parents:
    fail('engine and private source must be independent, non-nested paths')
for path in (checkout, engine):
    if '..' in path.parts or path == home or path in home.parents:
        fail('checkout path must be bounded and must not contain HOME')
for marker in ('home', 'dot-contract.json', 'tools/dot/manifest.json', 'tools/dot/groups.json'):
    path = engine / marker
    if path.exists() or path.is_symlink():
        fail('engine payload contains private-role marker; refusing public engine classification')
check_checkout_and_launcher()
check_startup_markers()
if (home / '.git').exists() or (home / '.git').is_symlink():
    fail('legacy installation detected: HOME Git metadata; explicit migration required; left untouched')
if engine.exists() and checkout.exists():
    check_contract()
if config.exists():
    tracker = tomllib.loads(config.read_text())
    recorded_engine = tracker.get('data', {}).get('dotEngine')
    if recorded_engine != str(engine / 'tools/dot'):
        fail('existing tracker dotEngine does not match installed engine; explicit dot init review required')
    if tracker.get('sourceDir') != str(checkout / 'home'):
        fail('existing tracker sourceDir does not match private source; explicit migration required')
if action == 'preflight':
    print('Read-only layout detection passed; no tracker, launcher, metadata, or dependencies were created.')
elif action == 'configure':
    print('Tracker creation is owned by dot init; bootstrap will leave first-run setup pending.')
elif action == 'launcher':
    atomic(launcher, expected, 0o755)
else:
    fail('unknown local setup action')
PY
}
# Common migration seam. Never initialize/evaluate legacy config or Git hooks.
migration_gate() { PHASE=migration; }
migration_gate
[ "$STAGE" != migration ] || { echo 'Migration is owned by #27; bootstrap performs read-only detection only.'; exit 0; }
PHASE=identity
cd "$HOME"
for field in name email; do
  value="$(git config "user.$field" || true)"
  if [ -z "$value" ]; then
    "$YES" && fail "--yes requires existing Git user.$field; configure it and retry"
    ask "Git user.$field:"
    [ -n "$ANSWER" ] || fail "Git user.$field cannot be empty"
    git config --global "user.$field" "$ANSWER"
  fi
done
PHASE=access
compatible gh || fail 'missing/incompatible prerequisite gh; dot init owns dependency setup; bootstrap does not install dependencies'
export GIT_TERMINAL_PROMPT=0 GH_PROMPT_DISABLED=1
check_source_access() {
  local metadata
  if ! metadata="$(gh api "repos/$REPOSITORY" 2>/dev/null)"; then
    ACCESS_PROBLEM='private repository API access denied'; return 1
  fi
  if ! python3 -c 'import json,sys; d=json.loads(sys.argv[1]); sys.exit(not (isinstance(d,dict) and d.get("full_name")==sys.argv[2] and d.get("private") is True and isinstance(d.get("default_branch"),str) and bool(d["default_branch"])))' "$metadata" "$REPOSITORY" >/dev/null 2>&1; then
    ACCESS_PROBLEM='private repository identity/visibility mismatch; no repository fallback permitted'; return 1
  fi
  if ! gh auth setup-git --hostname github.com >/dev/null 2>&1; then
    ACCESS_PROBLEM='GitHub CLI Git credential integration failed'; return 1
  fi
  if ! git ls-remote "$SOURCE_URL" HEAD >/dev/null 2>&1; then
    ACCESS_PROBLEM='private Git access denied despite API access'; return 1
  fi
}
if ! check_source_access; then
  "$YES" && fail "$ACCESS_PROBLEM; --yes requires existing access to the private repository and working Git credentials; run gh auth switch --hostname github.com or gh auth login --hostname github.com --git-protocol https --web, then retry"
  echo "$ACCESS_PROBLEM. Choose account switch or HTTPS browser login; no SSH keys will be generated."
  ask 'Authentication [switch/login/cancel]:'
  case "$ANSWER" in
    switch) ask 'GitHub username:'; gh auth switch --hostname github.com --user "$ANSWER" <&3 >&3 2>&3 ;;
    login) (unset GH_PROMPT_DISABLED; gh auth login --hostname github.com --git-protocol https --web --skip-ssh-key <&3 >&3 2>&3) ;;
    *) fail 'cancelled authentication' ;;
  esac
  check_source_access || fail "authorization denied after authentication: $ACCESS_PROBLEM; request repository access or switch accounts and retry"
fi
PHASE=engine
if [ -e "$HOME/.local/share/dot" ]; then fail "legacy installation detected at $HOME/.local/share/dot; migration belongs to #27; left untouched"; fi
if [ -f "$HOME/.local/state/dot/install.json" ]; then
  python3 - "$HOME/.local/state/dot/install.json" "$DIRECTORY" "$ENGINE_DIRECTORY" <<'PYRECORD'
import json,sys
record=json.load(open(sys.argv[1]))
if record.get('directory') != sys.argv[2] or record.get('engine') != sys.argv[3]:
    raise SystemExit('dot installer: installed paths conflict; migration needed; left untouched')
PYRECORD
fi
if [ -f "$HOME/.local/bin/dot" ] && ! grep -Fq 'DOT_ENGINE=' "$HOME/.local/bin/dot"; then fail 'unrelated launcher conflict; left untouched'; fi
if [ -f "$HOME/.local/state/dot/install.json" ] && { [ ! -d "$ENGINE_DIRECTORY" ] || [ ! -d "$DIRECTORY" ]; }; then fail 'installation record exists but a split checkout is missing; explicit recovery required'; fi
if [ -d "$HOME/.local/share/chezmoi" ] || [ -f "${XDG_CONFIG_HOME:-$HOME/.config}/chezmoi/chezmoi.toml" ]; then
  echo 'Existing chezmoi tracker detected read-only; dot init will verify identity and dotEngine compatibility. Bootstrap will not create or modify tracker state.'
fi
if [ -d "$ENGINE_DIRECTORY" ] && [ -d "$DIRECTORY" ]; then
  [ -f "$HOME/.local/state/dot/install.json" ] && [ -f "$HOME/.local/bin/dot" ] || fail 'partial/unrecorded split installation conflict; explicit recovery required; left untouched'
  if [ -n "$BROWSER_URL" ] && { [ -L "$ENGINE_DIRECTORY/bin" ] || [ -L "$ENGINE_DIRECTORY/bin/dot-browser" ]; }; then fail 'browser destination symlink conflict; left untouched'; fi
  if [ -e "$ENGINE_DIRECTORY/home" ] || [ -e "$ENGINE_DIRECTORY/dot-contract.json" ] || [ -e "$ENGINE_DIRECTORY/tools/dot/manifest.json" ] || [ -e "$ENGINE_DIRECTORY/tools/dot/groups.json" ]; then fail 'engine payload contains private-role marker; refusing public engine classification'; fi
  if [ -n "$ENGINE_URL" ]; then
    actual_engine_url="$(git -C "$ENGINE_DIRECTORY" remote get-url origin 2>/dev/null || true)"
    [ "$actual_engine_url" = "$ENGINE_URL" ] || fail 'installed engine remote does not match requested fixture/repository; left untouched'
  fi
  if [ -e "$ENGINE_DIRECTORY/home" ] || [ -e "$ENGINE_DIRECTORY/dot-contract.json" ] || [ -e "$ENGINE_DIRECTORY/tools/dot/manifest.json" ] || [ -e "$ENGINE_DIRECTORY/tools/dot/groups.json" ]; then fail 'engine payload contains private-role marker; refusing public engine classification'; fi
  if [ -f "$HOME/.local/bin/dot" ] && ! grep -Fq 'DOT_ENGINE=' "$HOME/.local/bin/dot"; then fail 'unrelated launcher conflict; left untouched'; fi
  export DOT_ENGINE_URL="$ENGINE_URL"
  local_setup preflight
  if [ -f "$HOME/.local/state/dot/install.json" ]; then
    python3 - "$HOME/.local/state/dot/install.json" "$DIRECTORY" "$ENGINE_DIRECTORY" <<'PYRECORD'
import json,sys
r=json.load(open(sys.argv[1]))
if r.get('directory') != sys.argv[2] or r.get('engine') != sys.argv[3]: raise SystemExit('dot installer: installation record mismatch; left untouched')
PYRECORD
  fi
  if [ -n "$BROWSER_URL" ]; then
    [ ! -L "$ENGINE_DIRECTORY/bin" ] && [ ! -L "$ENGINE_DIRECTORY/bin/dot-browser" ] || fail 'browser destination symlink conflict; left untouched'
    [ ! -e "$ENGINE_DIRECTORY/bin/dot-browser" ] || [ -f "$ENGINE_DIRECTORY/bin/dot-browser" ] || fail 'browser destination is not a regular file; left untouched'
    prepare_browser
    [ ! -L "$ENGINE_DIRECTORY/bin" ] && [ ! -L "$ENGINE_DIRECTORY/bin/dot-browser" ] || fail 'browser destination symlink conflict; left untouched'
    mkdir -p "$ENGINE_DIRECTORY/bin"
    python3 - "$HOME/.local/state/dot/install.json" "$BROWSER_SHA256" "$OWNED_BROWSER_TMP" "$ENGINE_DIRECTORY/bin/dot-browser" <<'PYBROWSER'
import json,os,pathlib,stat,sys,tempfile
path=pathlib.Path(sys.argv[1]); target=pathlib.Path(sys.argv[4]); before=path.read_bytes(); record=json.loads(before)
old=target.read_bytes() if target.exists() else None
oldmode=stat.S_IMODE(target.stat().st_mode) if old is not None else None
recordmode=stat.S_IMODE(path.stat().st_mode)
record['browser']={'path':'bin/dot-browser','sha256':sys.argv[2],'mode':493}
def write(destination,data,mode):
    fd,tmp=tempfile.mkstemp(dir=destination.parent)
    try:
        with os.fdopen(fd,'wb') as stream: stream.write(data)
        os.chmod(tmp,mode); os.replace(tmp,destination)
    finally:
        if os.path.exists(tmp): os.unlink(tmp)
try:
    write(target,pathlib.Path(sys.argv[3]).read_bytes(),0o755)
    write(path,(json.dumps(record,indent=2)+'\n').encode(),recordmode)
except OSError:
    if old is None: target.unlink(missing_ok=True)
    else: write(target,old,oldmode)
    if path.read_bytes()!=before: write(path,before,recordmode)
    raise SystemExit('dot installer: browser/record promotion failed; previous artifact restored')
PYBROWSER
    rm -f "$OWNED_BROWSER_TMP"
    OWNED_BROWSER_TMP=''
  fi
  if [ -n "$BROWSER_URL" ]; then echo 'Matching split installation detected; reviewed browser artifact installed; configuration unchanged.'; else echo 'Matching split installation detected; no changes made.'; fi
  echo 'Tracker initialization and any dependencies remain pending for explicit dot init.'
  exit 0
fi
if [ -e "$ENGINE_DIRECTORY" ] || [ -e "$DIRECTORY" ]; then fail 'partial/unrecorded split installation conflict; paths left untouched'; fi
if [ -e "$HOME/.local/bin/dot" ]; then fail 'unrelated launcher conflict; left untouched'; fi
if ! "$YES"; then confirm "Install public CLI at $ENGINE_DIRECTORY and private source at $DIRECTORY? Bootstrap will not install dependencies or initialize chezmoi."; fi
export DOT_ENGINE_URL="$ENGINE_URL"
local_setup preflight
if [ -L "$HOME" ] || [ -L "$(dirname "$ENGINE_DIRECTORY")" ] || [ -L "$(dirname "$DIRECTORY")" ] || [ -L "$HOME/.local" ] || [ -L "$HOME/.local/share" ]; then fail 'install parent symlink conflict; left untouched'; fi
if [ -e "$HOME/.local/bin" ] && [ -L "$HOME/.local/bin" ]; then fail 'launcher directory symlink conflict; left untouched'; fi
for startup_path in "$HOME/.bash_profile" "$HOME/.bash_login" "$HOME/.profile" "$HOME/.bashrc" "${ZDOTDIR:-$HOME}/.zprofile" "${ZDOTDIR:-$HOME}/.zshrc"; do
  [ ! -L "$startup_path" ] || fail "startup path symlink conflict: $startup_path; left untouched"
done
if [ -L "$ENGINE_DIRECTORY" ] || [ -L "$DIRECTORY" ]; then fail 'checkout path symlink conflict; left untouched'; fi
OWNED_CHECKOUT_TMP="$(mktemp -d "${TMPDIR:-/var/tmp}/dot-checkout.XXXXXXXX")"
git -c core.hooksPath=/dev/null -c init.templateDir= clone "$ENGINE_URL" "$OWNED_CHECKOUT_TMP/engine"
git -c core.hooksPath=/dev/null -c init.templateDir= clone "$SOURCE_URL" "$OWNED_CHECKOUT_TMP/private"
# Bound executable entrypoints before executing the reviewed checker; all API
# negotiation remains in contract.py, the same seam used by healthy CLI/update.
python3 -B - "$OWNED_CHECKOUT_TMP/engine" "$OWNED_CHECKOUT_TMP/private" <<'PYCANDIDATE'
import os,pathlib,subprocess,sys
engine=pathlib.Path(sys.argv[1]).resolve(); source=pathlib.Path(sys.argv[2]).resolve()
for marker in ('home','dot-contract.json','tools/dot/manifest.json','tools/dot/groups.json'):
    path=engine/marker
    if path.exists() or path.is_symlink():
        raise SystemExit('dot installer: engine payload contains private-role marker; no engine installed')
for name in ('dot','contract.py'):
    path=engine/'tools/dot'/name
    if path.resolve()!=path or not path.is_file():
        raise SystemExit('dot installer: candidate contract entrypoint is missing or escaping; no engine installed')
if not os.access(engine/'tools/dot/dot',os.X_OK):
    raise SystemExit('dot installer: candidate contract CLI is not executable; no engine installed')
subprocess.run([sys.executable,'-B',str(engine/'tools/dot/contract.py'),'check','--engine',str(engine/'tools/dot'),'--source',str(source)],check=True,stdout=subprocess.DEVNULL)
PYCANDIDATE
# Recheck requested paths immediately before promoting the validated candidate.
prepare_browser
local_setup preflight
[ ! -e "$ENGINE_DIRECTORY" ] && [ ! -L "$ENGINE_DIRECTORY" ] && [ ! -e "$DIRECTORY" ] && [ ! -L "$DIRECTORY" ] || fail 'checkout paths changed during retrieval; left untouched'
mkdir -p "$(dirname "$ENGINE_DIRECTORY")" "$(dirname "$DIRECTORY")"
mv "$OWNED_CHECKOUT_TMP/engine" "$ENGINE_DIRECTORY"
mv "$OWNED_CHECKOUT_TMP/private" "$DIRECTORY"
export DOT_ENGINE_URL="$ENGINE_URL"
if [ -e "$ENGINE_DIRECTORY/home" ] || [ -e "$ENGINE_DIRECTORY/dot-contract.json" ] || [ -e "$ENGINE_DIRECTORY/tools/dot/manifest.json" ] || [ -e "$ENGINE_DIRECTORY/tools/dot/groups.json" ]; then fail 'engine payload contains private-role marker; refusing to classify or record it as public'; fi
local_setup preflight
if [ -n "$BROWSER_URL" ]; then
  [ ! -L "$ENGINE_DIRECTORY/bin" ] && [ ! -L "$ENGINE_DIRECTORY/bin/dot-browser" ] || fail 'browser destination symlink conflict; left untouched'
  [ ! -e "$ENGINE_DIRECTORY/bin/dot-browser" ] || [ -f "$ENGINE_DIRECTORY/bin/dot-browser" ] || fail 'browser destination is not a regular file; left untouched'
  command -v shasum >/dev/null || fail 'shasum is required to verify browser artifact'
  for path in "$ENGINE_DIRECTORY/bin" "$ENGINE_DIRECTORY/bin/dot-browser"; do
  [ ! -L "$path" ] || fail "browser destination symlink conflict: $path; left untouched"
  done
  if [ -e "$ENGINE_DIRECTORY/bin" ] && [ ! -d "$ENGINE_DIRECTORY/bin" ]; then fail "browser destination is not a directory: $ENGINE_DIRECTORY/bin; left untouched"; fi
  mkdir -p "$ENGINE_DIRECTORY/bin"
  for path in "$ENGINE_DIRECTORY/bin" "$ENGINE_DIRECTORY/bin/dot-browser"; do
    if [ -L "$path" ]; then fail "browser destination symlink conflict: $path; left untouched"; fi
  done
  [ ! -L "$ENGINE_DIRECTORY/bin/dot-browser" ] || fail "browser destination symlink conflict: $ENGINE_DIRECTORY/bin/dot-browser; left untouched"
  mkdir -p "$ENGINE_DIRECTORY/bin"
  mv -f "$OWNED_BROWSER_TMP" "$ENGINE_DIRECTORY/bin/dot-browser"
  OWNED_BROWSER_TMP=''
fi
mkdir -p "$HOME/.local/bin" "$HOME/.local/state/dot"
python3 - "$HOME" "$DIRECTORY" "$ENGINE_DIRECTORY" "$SOURCE_URL" "$ENGINE_URL" "${XDG_CONFIG_HOME:-$HOME/.config}" "$BROWSER_SHA256" <<'PYRECORD'
import json,os,pathlib,sys,tempfile
home,source,engine=map(pathlib.Path,sys.argv[1:4]); config=pathlib.Path(sys.argv[6])/'chezmoi/chezmoi.toml'
state=home/'.local/state/dot/install.json'
record={'version':2,'directory':str(source),'engine':str(engine),'source':sys.argv[4],'engine_source':sys.argv[5],'config':str(config)}
if sys.argv[7]: record['browser']={'path':'bin/dot-browser','sha256':sys.argv[7],'mode':493}
fd,tmp=tempfile.mkstemp(dir=state.parent)
with os.fdopen(fd,'w') as stream:
    json.dump(record,stream,indent=2); stream.write('\n')
os.chmod(tmp,0o600); os.replace(tmp,state)
PYRECORD
local_setup launcher
echo 'CLI and private source installed. No packages, tracker, cache, or chezmoi state were created. Run dot init for informed first-run setup.'
