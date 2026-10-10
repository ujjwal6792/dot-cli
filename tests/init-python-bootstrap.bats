#!/usr/bin/env bats

setup() {
  command -v tmux >/dev/null || skip 'tmux required'
  export HOME="$BATS_TEST_TMPDIR/home" TRACE="$BATS_TEST_TMPDIR/trace"
  ENGINE="$BATS_TEST_TMPDIR/dot-cli/tools/dot"
  FIXTURE_BIN="$BATS_TEST_TMPDIR/bin"
  SOCKET="python-init-$$-$BATS_TEST_NUMBER"
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

teardown() { tmux -L "$SOCKET" kill-server 2>/dev/null || true; }
pane() { tmux -L "$SOCKET" capture-pane -p -S -100 -t ui; }
wait_for() {
  local i
  for i in {1..100}; do [[ "$(pane)" == *"$1"* ]] && return 0; sleep 0.1; done
  pane >&2; cat "$TRACE" >&2; ls -la "$FIXTURE_BIN" >&2; return 1
}
answer() {
  wait_for "$1"
  tmux -L "$SOCKET" send-keys -t ui -l "$2"
  tmux -L "$SOCKET" send-keys -t ui Enter
}
start_init() {
  local command
  printf -v command 'env PATH=%q DOT_ENGINE=%q DOT_SOURCE=%q DOT_CONFIG=%q HOME=%q TRACE=%q FIXTURE_BIN=%q FAKE_SYSTEM=%q FAKE_ARCH=%q DOT_INIT_OS_RELEASE_FILE=%q %q init; code=$?; echo UI_EXIT=$code; sleep 30' "$FIXTURE_BIN:/usr/bin:/bin" "$ENGINE" "$HOME/private" "$HOME/config.toml" "$HOME" "$TRACE" "$FIXTURE_BIN" "${FAKE_SYSTEM:-Darwin}" "${FAKE_ARCH:-arm64}" "${OS_RELEASE:-}" "$ENGINE/dot"
  tmux -L "$SOCKET" new-session -d -s ui -x 160 -y 50 "$command"
}

@test "Python bootstrap reviews before exact mocked macOS install and rechecks runtime" {
  cat > "$FIXTURE_BIN/brew" <<'SH'
#!/bin/sh
echo "brew $*" >> "$TRACE"
echo "PATH=$PATH FIXTURE_BIN=$FIXTURE_BIN" >> "$TRACE"
case "$1" in
  --version) echo 'Homebrew fixture' ;;
  install)
    if [ "$2" = --dry-run ]; then echo 'Python 3.11 bottle plan'; else
      cat > "$FIXTURE_BIN/python3" <<'PY'
#!/bin/sh
if [ "$1" = -c ]; then echo '3.12.1'; exit 0; fi
echo "$*" > "$TRACE.delegated"
exit 77
PY
      chmod +x "$FIXTURE_BIN/python3"
    fi ;;
  *) exit 91 ;;
esac
SH
  chmod +x "$FIXTURE_BIN/brew"
  start_init
  answer 'Install Python 3.11+' y
  wait_for UI_EXIT=77
  [ "$(cat "$TRACE.delegated")" = "-B $ENGINE/init.py" ]
  grep -F 'brew install --force-bottle -- python@3.11' "$TRACE"
  [ ! -e "$HOME/.local/state/dot/install.json" ]
}

@test "canceling Python bootstrap runs no package manager transaction" {
  cat > "$FIXTURE_BIN/brew" <<'SH'
#!/bin/sh
echo "brew $*" >> "$TRACE"
SH
  chmod +x "$FIXTURE_BIN/brew"
  start_init
  answer 'Install Python 3.11+' n
  wait_for UI_EXIT=1
  [[ "$(pane)" == *'declined'* ]]
  ! grep -F 'brew install --force-bottle' "$TRACE"
  [ ! -e "$HOME/.local/state/dot/install.json" ]
}

@test "Ubuntu bootstrap reviews a named apt transaction and never refreshes repositories" {
  export FAKE_SYSTEM=Linux FAKE_ARCH=x86_64 OS_RELEASE="$BATS_TEST_TMPDIR/os-release"
  printf 'ID=ubuntu\nVERSION_ID="24.04"\n' > "$OS_RELEASE"
  cat > "$FIXTURE_BIN/dpkg" <<'SH'
#!/bin/sh
echo amd64
SH
  cat > "$FIXTURE_BIN/sudo" <<'SH'
#!/bin/sh
echo "sudo $*" >> "$TRACE"
exec "$@"
SH
  cat > "$FIXTURE_BIN/apt-get" <<'SH'
#!/bin/sh
echo "apt-get $*" >> "$TRACE"
case "$1" in
  -s) echo 'Inst python3.11 (fixture repository)' ;;
  install)
    cat > "$FIXTURE_BIN/python3" <<'PY'
#!/bin/sh
if [ "$1" = -c ]; then exit 0; fi
echo "$*" > "$TRACE.delegated"
exit 77
PY
    chmod +x "$FIXTURE_BIN/python3" ;;
  *) exit 91 ;;
esac
SH
  chmod +x "$FIXTURE_BIN/dpkg" "$FIXTURE_BIN/sudo" "$FIXTURE_BIN/apt-get"
  start_init
  answer 'Install Python 3.11+' y
  wait_for UI_EXIT=77
  [[ "$(pane)" == *'configured apt repositories only'* ]]
  grep -F 'apt-get install --no-remove --no-install-recommends -- python3.11' "$TRACE"
  ! grep -E 'apt-get (update|upgrade|dist-upgrade)' "$TRACE"
  [ "$(cat "$TRACE.delegated")" = "-B $ENGINE/init.py" ]
}

@test "Arch bootstrap discloses full system upgrade and retains pacman confirmation" {
  export FAKE_SYSTEM=Linux FAKE_ARCH=x86_64 OS_RELEASE="$BATS_TEST_TMPDIR/os-release"
  printf 'ID=arch\n' > "$OS_RELEASE"
  cat > "$FIXTURE_BIN/sudo" <<'SH'
#!/bin/sh
echo "sudo $*" >> "$TRACE"
exec "$@"
SH
  cat > "$FIXTURE_BIN/pacman" <<'SH'
#!/bin/sh
echo "pacman $*" >> "$TRACE"
case "$1" in
  -Sp) echo 'core/python fixture-version' ;;
  -Syu)
    printf 'Proceed with reviewed Python transaction? [y/N] '
    read -r answer
    [ "$answer" = y ] || exit 1
    cat > "$FIXTURE_BIN/python3" <<'PY'
#!/bin/sh
if [ "$1" = -c ]; then exit 0; fi
echo "$*" > "$TRACE.delegated"
exit 77
PY
    chmod +x "$FIXTURE_BIN/python3" ;;
  *) exit 91 ;;
esac
SH
  chmod +x "$FIXTURE_BIN/sudo" "$FIXTURE_BIN/pacman"
  start_init
  answer 'Install Python 3.11+' y
  answer 'Proceed with reviewed Python transaction' y
  wait_for UI_EXIT=77
  [[ "$(pane)" == *'FULL SYSTEM UPGRADE'* ]]
  grep -F 'pacman -Syu -- core/python' "$TRACE"
  ! grep -E 'noconfirm|--noconfirm' "$TRACE"
  [ "$(cat "$TRACE.delegated")" = "-B $ENGINE/init.py" ]
}
