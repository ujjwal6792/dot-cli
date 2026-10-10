#!/usr/bin/env bats
load bootstrap-helper

setup() {
  bootstrap_setup
  arch_fixture
}

@test "Arch installs split launcher without tracker or package operations" {
  git config --global user.signingkey fixture-key
  run install_dot --yes --directory "$HOME/install with spaces"
  [ "$status" -eq 0 ]
  [[ "$output" == *'No packages'* ]]
  [ "$(git config --global user.signingkey)" = fixture-key ]
  [ "$(git -C "$HOME/install with spaces" rev-parse --abbrev-ref '@{upstream}')" = origin/main ]
  for shell in bash zsh; do
    run "$shell" -c '"$HOME/.local/bin/dot" status'
    [ "$status" -ne 0 ]
    [[ "$output" == *'run dot init'* ]]
  done
  [ ! -e "$HOME/.config/eza/theme.yml" ]
  [[ "$(cat "$TRACE")" != *'sudo '* ]]
  [[ "$(cat "$TRACE")" != *'brew '* ]]
}

@test "Arch refuses incompatible prerequisites without a system upgrade" {
  missing_gum
  export FAKE_GH_VERSION=2.45.7 REAL_PYTHON="$(command -v python3)" REAL_CHEZMOI="$(command -v chezmoi)"
  cat > "$FIXTURE_BIN/python3" <<'SH'
#!/bin/bash
if [ "$1" = -c ] && [ ! -e "$HOME/python-installed" ]; then exit 1; fi
exec "$REAL_PYTHON" "$@"
SH
  cat > "$FIXTURE_BIN/chezmoi" <<'SH'
#!/bin/bash
[ -e "$HOME/chezmoi-installed" ] || exit 1
exec "$REAL_CHEZMOI" "$@"
SH
  chmod +x "$FIXTURE_BIN/python3" "$FIXTURE_BIN/chezmoi"
  run install_dot --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'missing/incompatible prerequisite python3'* ]]
  [[ "$(cat "$TRACE")" != *'pacman '* ]]
  [ ! -e "$HOME/.local/bin/dot" ]
  [ ! -e "$HOME/.config/eza/theme.yml" ]
  [[ "$(cat "$TRACE")" != *'apt-get'* ]]
  [[ "$(cat "$TRACE")" != *'brew '* ]]
}

@test "Arch rejects unsupported architectures and derivatives before any transaction" {
  for architecture in aarch64 arm64 i686 riscv64; do
    export FAKE_ARCH="$architecture"
    run install_dot --yes
    [ "$status" -ne 0 ]
    [[ "$output" == *'unsupported Arch architecture'* ]]
    [ ! -s "$TRACE" ]
  done
  export FAKE_ARCH=x86_64 FAKE_DISTRO=cachyos
  run install_dot --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'unsupported Linux distribution'* ]]
  [ ! -s "$TRACE" ]
}

@test "Arch base command detection never repairs packages" {
  export REAL_GIT="$(command -v git)" REAL_CURL="$(command -v curl)"
  cat > "$FIXTURE_BIN/bash" <<'SH'
#!/bin/bash
if [ "$1" = -c ] && [ ! -e "$HOME/bash-installed" ]; then exit 1; fi
exec /bin/bash "$@"
SH
  cat > "$FIXTURE_BIN/git" <<'SH'
#!/bin/bash
if [ "$1" = --version ] && [ ! -e "$HOME/git-installed" ]; then exit 1; fi
exec "$REAL_GIT" "$@"
SH
  cat > "$FIXTURE_BIN/curl" <<'SH'
#!/bin/bash
if [ "$1" = --version ] && [ ! -e "$HOME/curl-installed" ]; then exit 1; fi
exec "$REAL_CURL" "$@"
SH
  chmod +x "$FIXTURE_BIN/bash" "$FIXTURE_BIN/git" "$FIXTURE_BIN/curl"
  run install_dot --yes --stage dependencies
  [ "$status" -eq 0 ]
  [[ "$output" == *'Missing/incompatible prerequisite: bash'* ]]
  [[ "$(cat "$TRACE")" != *'pacman '* ]]
  [ ! -e "$HOME/.local/share/dot" ]
  [[ "$(cat "$TRACE")" != *'gh api '* ]]
}

@test "Arch package manager failure flags never cause a package transaction" {
  missing_gum
  for boundary in SUDO_DENIED PACKAGE_FAIL VERIFY_FAIL; do
    export "$boundary=yes"
    run install_dot --yes
    [ "$status" -eq 0 ]
    [ -x "$HOME/.local/bin/dot" ]
    [[ "$(cat "$TRACE")" != *'pacman '* ]]
    [[ "$(cat "$TRACE")" != *'sudo '* ]]
    [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
    unset "$boundary"
  done
}

@test "Arch rejects both known broken GitHub CLI series and verifies command capabilities" {
  for version in 2.45.9 2.46.8; do
    rm -f "$HOME/gh-installed"
    export FAKE_GH_VERSION="$version"
    run install_dot --yes --stage dependencies
    [ "$status" -eq 0 ]
    [[ "$(cat "$TRACE")" != *'pacman '* ]]
    run install_dot --yes
    [ "$status" -ne 0 ]
    [[ "$output" == *'missing/incompatible prerequisite gh'* ]]
  done
  export GH_CAPABILITY_FAIL=yes
  run install_dot --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'missing/incompatible prerequisite gh'* ]]
  [ ! -e "$HOME/.local/share/dot" ]
}

@test "Arch preserves identity access legacy gates and never updates private checkout" {
  git config --global --unset user.email
  run install_dot --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'--yes requires existing Git user.email'* ]]
  git config --global user.email fixture@example.org
  export ACCESS_DENIED=yes
  run install_dot --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'--yes requires existing access'* ]]
  export ACCESS_DENIED=no
  run install_dot --yes
  [ "$status" -eq 0 ]
  git clone -q "$FIXTURE_REMOTE" "$HOME/editor"
  printf 'not adopted\n' > "$HOME/editor/dot_arch-new"
  git -C "$HOME/editor" add .
  git -C "$HOME/editor" commit -qm advance
  git -C "$HOME/editor" push -q
  before="$(git -C "$HOME/.local/share/dot-config" rev-parse HEAD)"
  run install_dot --yes --existing keep
  [ "$status" -eq 0 ]
  [ "$(git -C "$HOME/.local/share/dot-config" rev-parse HEAD)" = "$before" ]
  run install_dot --yes --existing update
  [ "$status" -eq 0 ]
  [ "$(git -C "$HOME/.local/share/dot-config" rev-parse HEAD)" = "$before" ]
  [ ! -e "$HOME/.arch-new" ]
  mkdir "$HOME/.git"
  run install_dot --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'explicit migration required'* ]]
  [ -d "$HOME/.git" ]
}
