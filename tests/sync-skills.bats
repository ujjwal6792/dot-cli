#!/usr/bin/env bats
# External-behavior tests for scripts/sync-skills.sh (the agy skills router).
# Runs the router against a fixture HOME and asserts on exit codes, stdout,
# and resulting filesystem state only.

setup() {
  FIXTURE_TMP="$(cd -P "$BATS_TEST_TMPDIR" && pwd)"
  FIXTURE_HOME="$BATS_TEST_TMPDIR/home"
  CANONICAL="$FIXTURE_HOME/.agents/skills"
  DEST="$FIXTURE_HOME/.gemini/skills"
  mkdir -p "$CANONICAL/skill-a" "$CANONICAL/skill-b/nested"
  printf -- '---\nname: skill-a\n---\n' >"$CANONICAL/skill-a/SKILL.md"
  printf 'body\n' >"$CANONICAL/skill-b/nested/file.txt"
  ROUTER="$BATS_TEST_DIRNAME/../scripts/sync-skills.sh"
  # Exercise the historical whole-library router against a synthetic legacy
  # source, not the real repository's independent-engine contract.
  export DOT_SOURCE="$BATS_TEST_TMPDIR/legacy-source"
  mkdir -p "$DOT_SOURCE/tools/dot"
  printf '{"skills":{"risk":"Safe","os":["darwin","linux"],"paths":[".agents/skills","sync-skills.sh",".gemini/skills"]}}\n' > "$DOT_SOURCE/tools/dot/groups.json"
  cp "$BATS_TEST_DIRNAME/../tools/dot/"*.py "$DOT_SOURCE/tools/dot/"
  mkdir -p "$DOT_SOURCE/scripts"
  cp "$ROUTER" "$DOT_SOURCE/scripts/"
  ROUTER="$DOT_SOURCE/scripts/sync-skills.sh"
  # Public fixture adoption (v1 groups with the full skills path set); the
  # router mirroring under test uses the legacy groups path, not dot CLI.
  mkdir -p "$FIXTURE_HOME/.local/state/dot"
  printf '{"version":1,"groups":{"skills":[".agents/skills","sync-skills.sh",".gemini/skills"]}}\n' > "$FIXTURE_HOME/.local/state/dot/adoption.json"
}

run_router() {
  # Router mirroring uses the legacy groups path (DOT_ENGINE unset), matching
  # private behavior; DOT_ENGINE was needed above only for public scope setup.
  HOME="$FIXTURE_HOME" env -u DOT_ENGINE -u DOT_CONFIG bash "$ROUTER" "$@"
}

@test "mirrors the canonical library into the gemini skills dir" {
  run run_router
  [ "$status" -eq 0 ]
  [ -f "$DEST/skill-a/SKILL.md" ]
  [ -f "$DEST/skill-b/nested/file.txt" ]
  diff -r "$CANONICAL" "$DEST"
}

@test "replaces prior contents exactly" {
  mkdir -p "$DEST/stale-skill"
  printf 'stale\n' >"$DEST/stale-skill/SKILL.md"
  printf 'loose\n' >"$DEST/stray.txt"
  run run_router
  [ "$status" -eq 0 ]
  [ ! -e "$DEST/stale-skill" ]
  [ ! -e "$DEST/stray.txt" ]
  diff -r "$CANONICAL" "$DEST"
}

@test "is idempotent" {
  run run_router
  [ "$status" -eq 0 ]
  run run_router
  [ "$status" -eq 0 ]
  diff -r "$CANONICAL" "$DEST"
}

@test "replaces a symlinked gemini skills dir with a real mirror" {
  mkdir -p "$FIXTURE_HOME/.gemini"
  ln -s "$CANONICAL" "$DEST"
  run run_router
  [ "$status" -eq 0 ]
  [ -d "$DEST" ]
  [ ! -L "$DEST" ]
  diff -r "$CANONICAL" "$DEST"
}

@test "does not mirror .DS_Store files" {
  printf 'junk\n' >"$CANONICAL/.DS_Store"
  printf 'junk\n' >"$CANONICAL/skill-b/nested/.DS_Store"
  run run_router
  [ "$status" -eq 0 ]
  [ ! -e "$DEST/.DS_Store" ]
  [ ! -e "$DEST/skill-b/nested/.DS_Store" ]
}

@test "fails when the canonical library is missing" {
  rm -rf "$CANONICAL"
  run run_router
  [ "$status" -ne 0 ]
  [[ "$output" == *"not found"* ]]
  [ ! -e "$DEST" ]
}

@test "rejects symlinked DOT_STATE_DIR without reading its approval or mirroring" {
  outside="$FIXTURE_TMP/outside-state"
  mkdir -p "$outside"
  printf '{"version":1,"groups":{"skills":[".agents/skills","sync-skills.sh",".gemini/skills"]}}\n' >"$outside/adoption.json"
  ln -s "$outside" "$FIXTURE_TMP/state-link"
  export DOT_STATE_DIR="$FIXTURE_TMP/state-link"
  run run_router
  [ "$status" -ne 0 ]
  [[ "$output" == *'state directory must not traverse symlinks'* ]]
  [ ! -e "$DEST" ]
  grep -Fq '"skills"' "$outside/adoption.json"
  [ ! -e "$FIXTURE_HOME/.agents/skills.bak" ]
}

@test "rejects malformed explicit DOT_STATE_DIR instead of falling back to default approval" {
  for invalid in relative ""; do
    export DOT_STATE_DIR="$invalid"
    run run_router
    [ "$status" -ne 0 ]
    [ ! -e "$DEST" ]
  done
}

@test "valid isolated DOT_STATE_DIR selects isolated approval, not legacy approval" {
  isolated="$FIXTURE_TMP/isolated state"
  mkdir -p "$isolated"
  printf '{"version":1,"groups":{"skills":[".agents/skills","sync-skills.sh",".gemini/skills"]}}\n' >"$isolated/adoption.json"
  printf '{"version":1,"groups":{}}\n' >"$FIXTURE_HOME/.local/state/dot/adoption.json"
  export DOT_STATE_DIR="$isolated"
  run run_router
  [ "$status" -eq 0 ]
  [[ "$output" == *'sync-skills: mirrored'* ]]
  diff -r "$CANONICAL" "$DEST"
  [ ! -e "$FIXTURE_HOME/.agents/skills.bak" ]
}
