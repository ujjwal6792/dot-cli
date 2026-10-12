#!/usr/bin/env bash
# sync-skills.sh — agy skills router (ADR 0006).
#
# Mirrors the canonical skills library (~/.agents/skills) into agy's skills
# directory (~/.gemini/skills), replacing prior contents exactly. opencode
# and codex read the canonical library natively (verified on the pinned
# versions); agy cannot, so it gets a full mirror.
#
# Run directly, via `dot run sync-skills`, or by chezmoi on every apply
# (run_after_sync-skills.sh.tmpl), so the mirror self-heals instead of
# drifting.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [ -n "${DOT_ENGINE:-}" ]; then
  [ -n "${DOT_SOURCE:-}" ] || { echo 'sync-skills: explicit DOT_SOURCE required with public engine' >&2; exit 1; }
  python3 -B "$DOT_ENGINE/contract.py" check --engine "$DOT_ENGINE" --source "$DOT_SOURCE" >/dev/null || exit 1
  exec python3 -B "$DOT_ENGINE/skills.py" "$DOT_SOURCE" "$HOME"
fi
STATE_PATHS="$SCRIPT_DIR/../tools/dot/state_paths.py"
scope_version="$(python3 -B "$STATE_PATHS" approval-version)"
if [ "$scope_version" = 2 ] || [ "$scope_version" = 3 ]; then
  exec python3 -B "$SCRIPT_DIR/../tools/dot/skills.py" "$SCRIPT_DIR/.." "$HOME"
fi
if [ -d "$SCRIPT_DIR/../home" ] || [ "$scope_version" != 0 ]; then
  selected="$(python3 "$SCRIPT_DIR/../tools/dot/scope.py" skills "$SCRIPT_DIR/.." "$HOME")"
  [ "$selected" = yes ] || { echo 'sync-skills: outside adoption scope' >&2; exit 1; }
fi

canonical="$HOME/.agents/skills"
dest="$HOME/.gemini/skills"

if [ ! -d "$canonical" ]; then
  echo "sync-skills: canonical skills library not found: $canonical" >&2
  exit 1
fi

stage="$(mktemp -d "${TMPDIR:-/tmp}/sync-skills.XXXXXX")"
old="$(mktemp -du "${TMPDIR:-/tmp}/sync-skills-old.XXXXXX")"
trap 'rm -rf "$stage" "$old"' EXIT

# Copy canonical -> staging, excluding macOS Finder metadata.
tar -C "$canonical" --exclude='.DS_Store' -cf - . | tar -xf - -C "$stage"

# Swap staging into place, then drop the previous contents.
mkdir -p "$(dirname "$dest")"
if [ -e "$dest" ] || [ -L "$dest" ]; then
  mv "$dest" "$old"
fi
mv "$stage" "$dest"
rm -rf "$old"
trap - EXIT

echo "sync-skills: mirrored $canonical -> $dest"
