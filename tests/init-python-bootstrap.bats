#!/usr/bin/env bats

setup() {
  export HOME="$BATS_TEST_TMPDIR/home" TRACE="$BATS_TEST_TMPDIR/trace"
  ENGINE="$BATS_TEST_TMPDIR/dot-cli/tools/dot"
  FIXTURE_BIN="$BATS_TEST_TMPDIR/bin"
  mkdir -p "$HOME" "$ENGINE" "$FIXTURE_BIN"
  cp "$BATS_TEST_DIRNAME/../tools/dot/dot" "$ENGINE/dot"
  cp "$BATS_TEST_DIRNAME/../tools/dot/init-python-bootstrap.sh" "$ENGINE/init-python-bootstrap.sh"
  chmod +x "$ENGINE/dot"
  : > "$TRACE"
  ln -s "$(command -v bash)" "$FIXTURE_BIN/bash"
  ln -s "$(command -v dirname)" "$FIXTURE_BIN/dirname"
  cat > "$FIXTURE_BIN/uname" <<'SH'
#!/bin/sh
case "$1" in -s) echo "${FAKE_SYSTEM:-Darwin}" ;; -m) echo "${FAKE_ARCH:-arm64}" ;; esac
SH
  cat > "$FIXTURE_BIN/sw_vers" <<'SH'
#!/bin/sh
echo 15.0
SH
  chmod +x "$FIXTURE_BIN/uname" "$FIXTURE_BIN/sw_vers"
}

@test "missing Python stops with guidance and runs no package manager transaction" {
  cat > "$FIXTURE_BIN/brew" <<'SH'
#!/bin/sh
echo "brew $*" >> "$TRACE"
exit 91
SH
  chmod +x "$FIXTURE_BIN/brew"
  run env PATH="$FIXTURE_BIN:/usr/bin:/bin" DOT_ENGINE="$ENGINE" DOT_SOURCE="$HOME/private" DOT_CONFIG="$HOME/config.toml" HOME="$HOME" "$ENGINE/dot" init
  [ "$status" -ne 0 ]
  [[ "$output" == *'Python 3.11+ is missing'* ]]
  ! grep -E 'brew install|apt-get|pacman' "$TRACE"
  [ ! -e "$HOME/.local/state/dot/install.json" ]
  [ ! -e "$HOME/private" ]
}

@test "a compatible versioned Python is preferred over an older python3 without installing" {
  cat > "$FIXTURE_BIN/python3" <<'SH'
#!/bin/sh
case "$*" in *'sys.version_info'*) exit 1 ;; *) echo 'Python 3.9.6' ;; esac
SH
  cat > "$FIXTURE_BIN/python3.12" <<'SH'
#!/bin/sh
if [ "$1" = -c ]; then exit 0; fi
echo "$*" > "$TRACE.delegated"
exit 77
SH
  cat > "$FIXTURE_BIN/brew" <<'SH'
#!/bin/sh
echo "brew $*" >> "$TRACE"
exit 91
SH
  chmod +x "$FIXTURE_BIN/python3" "$FIXTURE_BIN/python3.12" "$FIXTURE_BIN/brew"
  run env PATH="$FIXTURE_BIN:/usr/bin:/bin" DOT_ENGINE="$ENGINE" DOT_SOURCE="$HOME/private" DOT_CONFIG="$HOME/config.toml" HOME="$HOME" "$ENGINE/dot" init
  [ "$status" -eq 77 ]
  [ "$(cat "$TRACE.delegated")" = "-B $ENGINE/init.py" ]
  ! grep -F 'brew install' "$TRACE"
}
