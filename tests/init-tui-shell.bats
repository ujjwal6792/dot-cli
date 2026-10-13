#!/usr/bin/env bats
# Ticket #12 (spec #10): TUI shell + entrypoints + init --check.
# External behavior only.

setup() {
  REPO="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
  export HOME="$BATS_TEST_TMPDIR/home"
  ENGINE="$BATS_TEST_TMPDIR/engine/tools/dot"
  mkdir -p "$HOME" "$ENGINE" "$BATS_TEST_TMPDIR/bin"
  cp "$REPO/tools/dot/dot" "$ENGINE/dot"
  cp "$REPO/tools/dot/"*.py "$ENGINE/"
  cp "$REPO/tools/dot/engine.json" "$ENGINE/engine.json"
  cp "$REPO/tools/dot/init-python-bootstrap.sh" "$ENGINE/init-python-bootstrap.sh"
  chmod +x "$ENGINE/dot"
  export DOT_ENGINE="$ENGINE"
}

@test "bare dot launches the TUI browser without gum or python" {
  mkdir -p "$ENGINE/../../bin"
  printf '#!/bin/sh\necho "tui-launched $*" >> "$BATS_TEST_TMPDIR/tui-trace"\n' > "$ENGINE/../../bin/dot-browser"
  chmod +x "$ENGINE/../../bin/dot-browser"
  : > "$BATS_TEST_TMPDIR/tui-trace"
  # No gum, no python3, no tty assertions: bare dot must exec the browser.
  run env -u DOT_SOURCE -u DOT_CONFIG PATH="/usr/bin:/bin" HOME="$HOME" DOT_ENGINE="$ENGINE" DOT_BROWSER="$ENGINE/../../bin/dot-browser" "$ENGINE/dot"
  [ "$status" -eq 0 ]
  grep -q tui-launched "$BATS_TEST_TMPDIR/tui-trace"
}

@test "help shows complete usage plus bare-dot menu note" {
  run env DOT_ENGINE="$ENGINE" "$ENGINE/dot" -h
  [ "$status" -eq 0 ]
  for verb in status diff push pull add remove run update doctor scope intent layout; do
    [[ "$output" == *"$verb"* ]]
  done
  [[ "$output" == *"no arguments"* ]]
  [[ "$output" == *"TUI"* ]]
}

@test "version and uninstall and health flags work without python or network" {
  mkdir -p "$BATS_TEST_TMPDIR/empty-bin"
  ln -sf "$(command -v bash)" "$BATS_TEST_TMPDIR/empty-bin/bash"
  ln -sf "$(command -v dirname)" "$BATS_TEST_TMPDIR/empty-bin/dirname"
  ln -sf "$(command -v sed)" "$BATS_TEST_TMPDIR/empty-bin/sed"
  ln -sf "$(command -v cat)" "$BATS_TEST_TMPDIR/empty-bin/cat"
  for flag in -v -u --uninstall --health; do
    run env PATH="$BATS_TEST_TMPDIR/empty-bin" HOME="$HOME" DOT_ENGINE="$ENGINE" "$ENGINE/dot" "$flag"
    [ "$status" -eq 0 ] || { echo "flag $flag failed: $output" >&2; return 1; }
  done
  run env PATH="$BATS_TEST_TMPDIR/empty-bin" HOME="$HOME" DOT_ENGINE="$ENGINE" "$ENGINE/dot" -v
  [[ "$output" == *'dot '* ]]
}

@test "init --check reports old-user verdict read-only offline" {
  source="$HOME/.local/share/dot-config"
  mkdir -p "$source/home" "$source/tools/dot"
  printf '{"protocol":1,"engine_api":{"min":1,"max":1},"manifest_version":1,"approval_version":2,"manifest":"tools/dot/manifest.json","home_source":"home","required_adapters":["intent","scope","gitconfig","skills","layout"]}\n' > "$source/dot-contract.json"
  printf '{"version":1,"entries":{},"profiles":{}}\n' > "$source/tools/dot/manifest.json"
  git init -q "$source"
  git -C "$source" config user.name Fixture
  git -C "$source" config user.email fixture@example.invalid
  git -C "$source" add .
  git -C "$source" commit -qm fixture
  git -C "$source" remote add origin "$source"
  config="$HOME/.config/chezmoi/chezmoi.toml"
  mkdir -p "$(dirname "$config")"
  printf 'sourceDir="%s/home"\n[data]\ndotEngine="%s"\n' "$source" "$ENGINE" > "$config"
  before="$(find "$source" "$config" -newer "$ENGINE/dot" 2>/dev/null | sort)"
  run env DOT_ENGINE="$ENGINE" DOT_SOURCE="$source" DOT_CONFIG="$config" "$ENGINE/dot" init --check
  [ "$status" -eq 0 ]
  [[ "$output" == *'old-user'* ]]
  after="$(find "$source" "$config" -newer "$ENGINE/dot" 2>/dev/null | sort)"
  [ "$before" = "$after" ]
}

@test "init --check reports new-user verdict and creates nothing offline" {
  source="$HOME/.local/share/dot-config"
  config="$HOME/.config/chezmoi/chezmoi.toml"
  rm -rf "$source" "$config"
  run env DOT_ENGINE="$ENGINE" DOT_SOURCE="$source" DOT_CONFIG="$config" "$ENGINE/dot" init --check
  [ "$status" -eq 0 ]
  [[ "$output" == *'new-user'* ]]
  [ ! -e "$source" ]
  [ ! -e "$config" ]
}
