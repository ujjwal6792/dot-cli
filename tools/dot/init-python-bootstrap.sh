#!/usr/bin/env bash
# Python-runtime bootstrap used only by `dot init`; every transaction is
# separately reviewed in a real terminal. Bash 3.2-compatible syntax.
set -euo pipefail

engine="$1"
shift
find_compatible_python() {
  for candidate in python3 python3.16 python3.15 python3.14 python3.13 python3.12 python3.11; do
    candidate_path="$(command -v "$candidate" 2>/dev/null || true)"
    case "$candidate_path" in /*) ;; *) continue ;; esac
    if "$candidate_path" -c 'import sys,tomllib; sys.exit(sys.version_info < (3,11))' >/dev/null 2>&1; then
      printf '%s\n' "$candidate_path"
      return 0
    fi
  done
  return 1
}

PYTHON_BIN="$(find_compatible_python || true)"
if [ -z "$PYTHON_BIN" ]; then
  system="$(uname -s)"
  case "$system" in
    Darwin)
      [ "$(uname -m)" = arm64 ] || { echo 'Python bootstrap supports Apple Silicon only; no package operation attempted.' >&2; exit 1; }
      macos_major="$(sw_vers -productVersion | cut -d. -f1)"
      case "$macos_major" in ''|*[!0-9]*) echo 'Could not verify macOS version; no package operation attempted.' >&2; exit 1 ;; esac
      [ "$macos_major" -ge 15 ] || { echo 'Python bootstrap requires macOS 15+; no package operation attempted.' >&2; exit 1; }
      manager="$(command -v brew || true)"
      [ -n "$manager" ] || { echo 'Python 3.11+ is missing and Homebrew is unavailable. Install a supported package manager explicitly, then rerun dot init.' >&2; exit 1; }
      "$manager" --version >/dev/null
      query=("$manager" install --dry-run --force-bottle -- python@3.11)
      operation=("$manager" install --force-bottle -- python@3.11)
      warning='Homebrew may install dependencies. This exact operation is limited to Python 3.11+; no private package list is read.'
      ;;
    Linux)
      os_release="${DOT_INIT_OS_RELEASE_FILE:-/etc/os-release}"
      if [ -r "$os_release" ]; then
        distro="$(. "$os_release"; printf '%s' "${ID:-}")"
        distro_version="$(. "$os_release"; printf '%s' "${VERSION_ID:-}")"
      else
        distro=''
      fi
      case "$distro" in
        ubuntu)
          case "$distro_version" in 24.04|24.10|25.*|26.*|27.*|28.*|29.*|30.*) ;; *) echo "Python bootstrap requires supported Ubuntu 24.04+; detected ${distro_version:-unknown}. No transaction attempted." >&2; exit 1 ;; esac
          architecture="$(uname -m)"
          native_arch="$(dpkg --print-architecture 2>/dev/null || true)"
          case "$native_arch:$architecture" in amd64:x86_64|arm64:aarch64|arm64:arm64) ;; *) echo 'Unsupported Ubuntu native architecture; no transaction attempted.' >&2; exit 1 ;; esac
          manager="$(command -v apt-get || true)"
          sudo="$(command -v sudo || true)"
          [ -n "$manager" ] && [ -n "$sudo" ] || { echo 'Ubuntu Python bootstrap needs apt-get and sudo; no transaction was attempted.' >&2; exit 1; }
          query=("$sudo" "$manager" -s install --no-remove --no-install-recommends -- python3.11)
          operation=("$sudo" "$manager" install --no-remove --no-install-recommends -- python3.11)
          warning='Ubuntu configured apt repositories only. Dependencies and maintainer scripts may run; no repository refresh, removal, or full upgrade.'
          ;;
        arch)
          [ "$(uname -m)" = x86_64 ] || { echo 'Official Arch x86_64 is required; no transaction attempted.' >&2; exit 1; }
          manager="$(command -v pacman || true)"
          sudo="$(command -v sudo || true)"
          [ -n "$manager" ] && [ -n "$sudo" ] || { echo 'Arch Python bootstrap needs pacman and sudo; no transaction was attempted.' >&2; exit 1; }
          query=("$sudo" "$manager" -Sp --print-format '%n %v' -- core/python)
          operation=("$sudo" "$manager" -Syu -- core/python)
          warning='FULL SYSTEM UPGRADE: pacman -Syu upgrades unrelated packages and may run scripts/hooks. Review Arch news, configuration, reboot, and .pacnew needs. Pacman keeps its own transaction confirmation.'
          ;;
        *) echo "Python bootstrap unsupported on Linux distribution: ${distro:-unknown}; no package operation attempted." >&2; exit 1 ;;
      esac
      ;;
    *) echo "Python bootstrap unsupported on $system; no package operation attempted." >&2; exit 1 ;;
  esac

  if [ ! -r /dev/tty ] || [ ! -w /dev/tty ]; then
    echo 'Python installation requires an interactive terminal; no package operation attempted.' >&2
    exit 1
  fi
  resolution="$("${query[@]}")" || { echo 'Could not resolve the Python package transaction; nothing installed.' >&2; exit 1; }
  printf 'Python 3.11+ is required by the dot engine.\n%s\nReviewed command: ' "$warning"
  printf '%q ' "${operation[@]}"
  printf '\nRead-only manager resolution:\n%s\n' "$resolution"
  printf 'Install Python 3.11+ with this exact operation? [y/N] ' >/dev/tty
  IFS= read -r answer </dev/tty || answer=''
  case "$answer" in y|Y|yes|YES) ;; *) echo 'Python installation declined; no package operation performed.' >&2; exit 1 ;; esac
  current="$("${query[@]}")" || { echo 'Could not revalidate the Python transaction; nothing installed.' >&2; exit 1; }
  [ "$current" = "$resolution" ] || { echo 'Python package transaction changed after review; nothing installed. Review again.' >&2; exit 1; }
  "${operation[@]}"
  hash -r
  PYTHON_BIN="$(find_compatible_python || true)"
  [ -n "$PYTHON_BIN" ] || { echo 'Python 3.11+ not available after install; inspect manager output and PATH, then rerun dot init.' >&2; exit 1; }
fi

DOT_SOURCE="${DOT_SOURCE:-$HOME/.local/share/dot-config}"
DOT_CONFIG="${DOT_CONFIG:-${XDG_CONFIG_HOME:-$HOME/.config}/chezmoi/chezmoi.toml}"
DOT_ENGINE="${DOT_ENGINE:-$engine}"
DOT_PYTHON="$PYTHON_BIN"
export DOT_SOURCE DOT_CONFIG DOT_ENGINE DOT_PYTHON
exec "$PYTHON_BIN" -B "$engine/init.py" "$@"
