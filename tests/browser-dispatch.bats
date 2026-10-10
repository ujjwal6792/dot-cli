#!/usr/bin/env bats
setup() {
  REPO="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
  export HOME="$BATS_TEST_TMPDIR/home" DOT_SOURCE="$BATS_TEST_TMPDIR/private" DOT_ENGINE="$REPO/tools/dot"
  mkdir -p "$HOME" "$DOT_SOURCE/home" "$DOT_SOURCE/tools/dot" "$BATS_TEST_TMPDIR/bin"
  printf '{"protocol":1,"engine_api":{"min":1,"max":1},"manifest_version":1,"approval_version":2,"manifest":"tools/dot/manifest.json","home_source":"home","required_adapters":["intent","scope","gitconfig","skills","layout"]}\n' > "$DOT_SOURCE/dot-contract.json"
  printf '{"version":1,"entries":{},"profiles":{}}\n' > "$DOT_SOURCE/tools/dot/manifest.json"
  export DOT_CONFIG="$HOME/tracker.toml"
  printf 'sourceDir="%s/home"\n[data]\ndotEngine="%s"\n' "$DOT_SOURCE" "$DOT_ENGINE" > "$DOT_CONFIG"
  export DOT_BROWSER="$BATS_TEST_TMPDIR/bin/dot-browser"
  cat > "$DOT_BROWSER" <<'SH'
#!/bin/sh
printf 'browser'
for arg in "$@"; do printf ' <%s>' "$arg"; done
printf '\n'
SH
  chmod +x "$DOT_BROWSER"
}
@test "browse delegates to independent browser before legacy dependency checks" {
  run "$REPO/tools/dot/dot" browse --mode track
  [ "$status" -eq 0 ]
  [[ "$output" == 'browser <--engine>'*' <--source>'*' <--home>'*' <--mode> <track>' ]]
}
@test "browse refuses absent private source and missing artifact without fallback" {
  run env -u DOT_SOURCE "$REPO/tools/dot/dot" browse
  [ "$status" -ne 0 ]
  [[ "$output" == *'explicit DOT_SOURCE'* ]]
  rm "$DOT_BROWSER"
  run "$REPO/tools/dot/dot" browse
  [ "$status" -ne 0 ]
  [[ "$output" == *'install a compatible dot-browser artifact'* ]]
}
