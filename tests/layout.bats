#!/usr/bin/env bats
setup() {
  REPO="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
  export HOME="$BATS_TEST_TMPDIR/home" DOT_SOURCE="$BATS_TEST_TMPDIR/source"
  export DOT_ENGINE="$REPO/tools/dot"
  export DOT_CONFIG="$HOME/.config/chezmoi/chezmoi.toml"
  mkdir -p "$HOME/.config/chezmoi" "$DOT_SOURCE/home" "$DOT_SOURCE/tools/dot"
  cp "$REPO/tools/dot/"*.py "$DOT_SOURCE/tools/dot/"
  cp "$BATS_TEST_DIRNAME/fixtures/chezmoiignore.tmpl" "$DOT_SOURCE/home/.chezmoiignore"
  printf '{"protocol":1,"engine_api":{"min":1,"max":1},"manifest_version":1,"approval_version":2,"manifest":"tools/dot/manifest.json","home_source":"home","required_adapters":["intent","scope","gitconfig","skills","layout"]}\n' > "$DOT_SOURCE/dot-contract.json"
  printf '{"version":1,"entries":{},"profiles":{}}\n' > "$DOT_SOURCE/tools/dot/manifest.json"
  git init -q "$DOT_SOURCE"
  git -C "$DOT_SOURCE" -c user.name=Test -c user.email=test@example.org add .
  git -C "$DOT_SOURCE" -c user.name=Test -c user.email=test@example.org commit -qm fixture
  DOT="$REPO/tools/dot/dot"
  printf '[data]\nname="Test"\nemail="test@example.org"\nprofile="personal"\ndotEngine="%s"\n' "$DOT_ENGINE" > "$DOT_CONFIG"
  "$DOT" scope init
  printf 'sourceDir = "%s" # preserve comment\r\n[data]\r\nname="Test"\r\nemail="test@example.org"\r\ndotEngine="%s"\r\n' "$DOT_SOURCE" "$DOT_ENGINE" > "$DOT_CONFIG"
  PLAN="$BATS_TEST_TMPDIR/layout.json"
}
approve_layout() {
  local token
  token="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["id"])' "$PLAN")"
  "$DOT" layout apply "$PLAN" --approve "$token"
}
@test "source migration is previewed approved recoverable and preserves exact selection and settings" {
  cp "$HOME/.local/state/dot/adoption.json" "$BATS_TEST_TMPDIR/selection"
  cp "$DOT_CONFIG" "$BATS_TEST_TMPDIR/config"
  "$DOT" layout plan --out "$PLAN"
  cmp "$DOT_CONFIG" "$BATS_TEST_TMPDIR/config"
  run "$DOT" layout apply "$PLAN"
  [ "$status" -ne 0 ]
  approve_layout
  cmp "$HOME/.local/state/dot/adoption.json" "$BATS_TEST_TMPDIR/selection"
  python3 - "$DOT_CONFIG" "$DOT_SOURCE" <<'PY'
import pathlib, sys, tomllib
p, root = sys.argv[1:]
raw = pathlib.Path(p).read_bytes()
assert b'# preserve comment\r\n[data]\r\n' in raw
assert tomllib.loads(raw.decode())['sourceDir'] == root + '/home'
PY
  [ ! -e "$HOME/.zshrc" ]
  run "$DOT" layout plan
  [ "$status" -eq 0 ]
  [[ "$output" == *'already migrated'* ]]
  run find "$HOME/.local/state/dot/layout-recovery" -name config -exec cmp '{}' "$BATS_TEST_TMPDIR/config" ';'
  [ "$status" -eq 0 ]
}
@test "stale migration and symlink metadata or missing boundary are rejected without adoption" {
  "$DOT" layout plan --out "$PLAN"
  printf '\n# changed after preview\n' >> "$DOT_CONFIG"
  run approve_layout
  [ "$status" -ne 0 ]
  [[ "$output" == *'stale/unapproved'* ]]
  rm "$HOME/.local/state/dot/adoption.json"
  run "$DOT" layout plan
  [ "$status" -ne 0 ]
  [[ "$output" == *'explicit existing adoption boundary'* ]]
  [ ! -e "$HOME/.zshrc" ]
}
