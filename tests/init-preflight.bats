#!/usr/bin/env bats

setup() {
  export HOME="$BATS_TEST_TMPDIR/home"
  ENGINE="$BATS_TEST_TMPDIR/dot-cli/tools/dot"
  mkdir -p "$HOME" "$ENGINE"
  cp "$BATS_TEST_DIRNAME/../tools/dot/dot" "$ENGINE/dot"
  chmod +x "$ENGINE/dot"
}

@test "help and version work without Python or private checkout" {
  mkdir -p "$BATS_TEST_TMPDIR/bin"
  ln -s "$(command -v bash)" "$BATS_TEST_TMPDIR/bin/bash"
  ln -s "$(command -v dirname)" "$BATS_TEST_TMPDIR/bin/dirname"
  ln -s "$(command -v cat)" "$BATS_TEST_TMPDIR/bin/cat"
  run env PATH="$BATS_TEST_TMPDIR/bin" DOT_ENGINE="$ENGINE" "$ENGINE/dot" --help
  [ "$status" -eq 0 ]
  [[ "$output" == *'dot init'* ]]
  run env PATH="$BATS_TEST_TMPDIR/bin" DOT_ENGINE="$ENGINE" "$ENGINE/dot" --version
  [ "$status" -eq 0 ]
}

@test "init check without Python reports pending prerequisites read-only" {
  mkdir -p "$BATS_TEST_TMPDIR/bin"
  ln -s "$(command -v bash)" "$BATS_TEST_TMPDIR/bin/bash"
  ln -s "$(command -v dirname)" "$BATS_TEST_TMPDIR/bin/dirname"
  ln -s "$(command -v cat)" "$BATS_TEST_TMPDIR/bin/cat"
  run env -u DOT_SOURCE -u DOT_CONFIG PATH="$BATS_TEST_TMPDIR/bin" HOME="$HOME" DOT_ENGINE="$ENGINE" "$ENGINE/dot" init --check
  [ "$status" -eq 0 ]
  [[ "$output" == *'Python 3.11+'* ]]
  [[ "$output" == *'Private checkout: not configured'* ]]
  [[ "$output" == *'Tracker: not configured'* ]]
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
  [ ! -e "$HOME/.local/state/dot/install.json" ]
  run env -u DOT_SOURCE -u DOT_CONFIG PATH="$BATS_TEST_TMPDIR/bin" HOME="$HOME" DOT_ENGINE="$ENGINE" "$ENGINE/dot" init
  [ "$status" -ne 0 ]
  [[ "$output" == *'requires Python 3.11+'* ]]
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
  [ ! -e "$HOME/.local/share/dot-config" ]
}
