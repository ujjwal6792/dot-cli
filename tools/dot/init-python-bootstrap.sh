#!/usr/bin/env bash
# Python-runtime bootstrap used only by `dot init`. It never installs anything:
# with no compatible Python it stops with guidance. Bash 3.2-compatible syntax.
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
  echo 'Python 3.11+ is missing; install it using your platform package manager, then rerun dot init. No dependency was installed.' >&2
  exit 1
fi

DOT_SOURCE="${DOT_SOURCE:-$HOME/.local/share/dot-config}"
DOT_CONFIG="${DOT_CONFIG:-${XDG_CONFIG_HOME:-$HOME/.config}/chezmoi/chezmoi.toml}"
DOT_ENGINE="${DOT_ENGINE:-$engine}"
DOT_PYTHON="$PYTHON_BIN"
export DOT_SOURCE DOT_CONFIG DOT_ENGINE DOT_PYTHON
exec "$PYTHON_BIN" -B "$engine/init.py" "$@"
