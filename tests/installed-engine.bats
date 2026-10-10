#!/usr/bin/env bats

setup() {
  export HOME="$BATS_TEST_TMPDIR/home"
  ENGINE="$BATS_TEST_TMPDIR/dot-cli/tools/dot"
  SOURCE="$BATS_TEST_TMPDIR/dot-config"
  mkdir -p "$HOME" "$ENGINE" "$SOURCE/home" "$SOURCE/tools/dot"
  cp "$BATS_TEST_DIRNAME/../tools/dot/"*.py "$BATS_TEST_DIRNAME/../tools/dot/engine.json" "$BATS_TEST_DIRNAME/../tools/dot/dot" "$ENGINE/"
  printf '{"version":1,"entries":{},"profiles":{}}\n' > "$SOURCE/tools/dot/manifest.json"
  printf '{"protocol":1,"engine_api":{"min":1,"max":1},"manifest_version":1,"approval_version":2,"manifest":"tools/dot/manifest.json","home_source":"home","required_adapters":["intent","scope","gitconfig","skills","layout"]}\n' > "$SOURCE/dot-contract.json"
  chmod +x "$ENGINE/dot"
  cat > "$HOME/.config.toml" <<EOF
[data]
dotEngine = "$ENGINE"
EOF
}

@test "installed dot requires explicit independent source and valid contract" {
  run env DOT_ENGINE="$ENGINE" "$ENGINE/dot" status
  [ "$status" -ne 0 ]
  [[ "$output" == *'DOT_SOURCE is required'* ]]
  run env DOT_ENGINE="$ENGINE" DOT_SOURCE="$SOURCE" DOT_CONFIG="$HOME/.config.toml" "$ENGINE/dot" status
  [ "$status" -ne 0 ]
  [[ "$output" != *'Traceback'* ]]
}

@test "public engine direct entrypoint refuses implicit source before chezmoi or git" {
  rm -f "$ENGINE/../../dot-contract.json"
  run env -u DOT_ENGINE -u DOT_SOURCE -u DOT_CONFIG "$ENGINE/dot" status
  [ "$status" -ne 0 ]
  [[ "$output" == *'public engine requires explicit DOT_ENGINE and private DOT_SOURCE'* ]]
  [[ "$output" != *'Traceback'* ]]
}

@test "installed dot rejects a config engine path that disagrees with DOT_ENGINE" {
  cat > "$HOME/.config.toml" <<EOF
[data]
dotEngine = "/other/engine"
EOF
  run env DOT_ENGINE="$ENGINE" DOT_SOURCE="$SOURCE" DOT_CONFIG="$HOME/.config.toml" "$ENGINE/dot" status
  [ "$status" -ne 0 ]
  [[ "$output" == *'dotEngine does not match'* ]]
}
