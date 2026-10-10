#!/usr/bin/env bats
load bootstrap-helper

setup() {
  bootstrap_setup
  ubuntu_fixture
}

@test "Ubuntu missing Gum leaves runtime provisioning for init" {
  ubuntu_packages
  missing_gum
  run install_dot --yes
  [ "$status" -eq 0 ]
  [[ "$output" == *'Run dot init'* ]]
  [[ "$(cat "$TRACE")" != *'apt-get'* ]]
  [ ! -e "$HOME/dependency-installed" ]
  [[ "$(cat "$TRACE")" != *'brew '* ]]
  [ -x "$HOME/.local/bin/dot" ]
}

@test "Ubuntu reuses compatible tools and completes common empty-scoped installation on both architectures" {
  for architecture in amd64 arm64; do
    export FAKE_DEB_ARCH="$architecture"
    if [ "$architecture" = amd64 ]; then export FAKE_ARCH=x86_64; else export FAKE_ARCH=aarch64; fi
    run install_dot --yes --directory "$HOME/install with spaces"
    [ "$status" -eq 0 ]
    [[ "$output" == *'No packages'* || "$output" == *'Matching split installation'* ]]
    [ -x "$HOME/.local/bin/dot" ]
    run bash -c '"$HOME/.local/bin/dot" status'
    [ "$status" -ne 0 ]
    [[ "$output" == *'run dot init'* ]]
    [ ! -e "$HOME/.local/state/dot/adoption.json" ]
    [ ! -e "$HOME/.config/eza/theme.yml" ]
    [[ "$(cat "$TRACE")" != *'apt-get'* ]]
    [[ "$(cat "$TRACE")" != *'brew '* ]]
  done
}

@test "Ubuntu releases distributions and architectures are validated before transactions" {
  for release in 20.04 23.10 unknown '24.04;touch malicious'; do
    export FAKE_UBUNTU="$release"
    run install_dot --yes
    [ "$status" -ne 0 ]
    [ ! -s "$TRACE" ]
  done
  export FAKE_UBUNTU=24.04 FAKE_DISTRO=debian
  run install_dot --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'unsupported Linux distribution'* ]]
  export FAKE_DISTRO=ubuntu
  for architecture in i386 riscv64 'amd64:x86_64'; do
    export FAKE_DEB_ARCH="$architecture"
    run install_dot --yes
    [ "$status" -ne 0 ]
    [[ "$output" == *'unsupported Ubuntu architecture'* ]]
  done
  export FAKE_DEB_ARCH=amd64 FAKE_ARCH=aarch64
  run install_dot --yes
  [ "$status" -ne 0 ]
  [ ! -s "$TRACE" ]
}

@test "Ubuntu 22.04 refuses unsuitable Python without changing packages and reuses a compatible interpreter" {
  export REAL_PYTHON="$(command -v python3)" FAKE_UBUNTU=22.04
  cat > "$FIXTURE_BIN/python3" <<'SH'
#!/bin/bash
if [ "$1" = -c ] && [ "${PYTHON_COMPATIBLE:-no}" != yes ]; then exit 1; fi
exec "$REAL_PYTHON" "$@"
SH
  chmod +x "$FIXTURE_BIN/python3"
  run install_dot --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'missing/incompatible prerequisite python3'* ]]
  [ ! -s "$TRACE" ]
  export PYTHON_COMPATIBLE=yes
  run install_dot --yes
  [ "$status" -eq 0 ]
  [ -x "$HOME/.local/bin/dot" ]
  [[ "$(cat "$TRACE")" != *'apt-get'* ]]
}

@test "known-broken GitHub CLI never creates an apt repository" {
  ubuntu_packages
  for version in 2.45.0 2.46.2; do
    rm -f "$HOME/gh-installed"
    export FAKE_GH_VERSION="$version"
    run install_dot --yes --stage dependencies
    [ "$status" -eq 0 ]
    [ ! -e "$HOME/system/etc/apt/sources.list.d/dot-github-cli.list" ]
    [[ "$(cat "$TRACE")" != *'apt-get'* ]]
    run install_dot --yes
    [ "$status" -ne 0 ]
    [[ "$output" == *'missing/incompatible prerequisite gh'* ]]
    [[ "$(cat "$TRACE")" != *'apt-key'* ]]
    [[ "$(cat "$TRACE")" != *'brew '* ]]
  done
}

@test "GitHub CLI capability failure remains incompatible even after package success" {
  ubuntu_packages
  export GH_CAPABILITY_FAIL=yes
  run install_dot --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'missing/incompatible prerequisite gh'* ]]
  [ ! -e "$HOME/.local/bin/dot" ]
}

@test "Ubuntu refuses absent base commands without an apt transaction" {
  ubuntu_packages
  export REAL_PYTHON="$(command -v python3)" REAL_GIT="$(command -v git)" CURL_MISSING=yes
  cat > "$FIXTURE_BIN/python3" <<'SH'
#!/bin/bash
if [ "$1" = -c ] && [ ! -e "$HOME/python-installed" ]; then exit 1; fi
exec "$REAL_PYTHON" "$@"
SH
  cat > "$FIXTURE_BIN/git" <<'SH'
#!/bin/bash
if [ "$1" = --version ] && [ ! -e "$HOME/git-installed" ]; then exit 1; fi
exec "$REAL_GIT" "$@"
SH
  chmod +x "$FIXTURE_BIN/python3" "$FIXTURE_BIN/git"
  run install_dot --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'missing/incompatible prerequisite git'* ]]
  [[ "$(cat "$TRACE")" != *'apt-get'* ]]
  [ ! -e "$HOME/.local/bin/dot" ]
  [[ "$(cat "$TRACE")" != *'add-apt-repository'* ]]
  [[ "$(cat "$TRACE")" != *'upgrade '* ]]
}

@test "Ubuntu missing tracker runtime is not installed on either supported architecture" {
  ubuntu_packages
  export REAL_CHEZMOI="$(command -v chezmoi)"
  cat > "$FIXTURE_BIN/chezmoi" <<'SH'
#!/bin/bash
[ -e "$HOME/chezmoi-installed" ] || exit 1
exec "$REAL_CHEZMOI" "$@"
SH
  chmod +x "$FIXTURE_BIN/chezmoi"
  for architecture in amd64 arm64; do
    rm -f "$HOME/chezmoi-installed" "$HOME/dependency-installed"
    missing_gum
    export FAKE_DEB_ARCH="$architecture" FAKE_GH_VERSION=2.80.0
    rm -f "$HOME/gh-installed"
    if [ "$architecture" = amd64 ]; then export FAKE_ARCH=x86_64; else export FAKE_ARCH=aarch64; fi
    # Separate fixture targets represent two machines; do not rewrite a target's
    # existing architecture-specific apt source to pretend it changed hardware.
    if [ "$architecture" = arm64 ] && [ -d "$HOME/system" ]; then mv "$HOME/system" "$HOME/amd64-system"; fi
    run install_dot --yes
    [ "$status" -eq 0 ]
    [[ "$(cat "$TRACE")" != *'apt-get'* ]]
    [ ! -e "$HOME/chezmoi-installed" ]
    run "$HOME/.local/bin/dot" status
    [ "$status" -ne 0 ]
    [[ "$output" == *'run dot init'* ]]
    [ ! -e "$HOME/.config/eza/theme.yml" ]
  done
}

@test "Ubuntu package manager failure flags never trigger package operations" {
  ubuntu_packages
  missing_gum
  for boundary in SUDO_DENIED APT_UPDATE_FAIL APT_SIMULATE_FAIL PACKAGE_FAIL VERIFY_FAIL; do
    export "$boundary=yes"
    run install_dot --yes
    [ "$status" -eq 0 ]
    [ -x "$HOME/.local/bin/dot" ]
    [[ "$(cat "$TRACE")" != *'apt-get'* ]]
    [[ "$(cat "$TRACE")" != *'sudo '* ]]
    unset "$boundary"
  done
}

@test "Ubuntu does not download runtime packages during bootstrap" {
  ubuntu_packages
  missing_gum
  for boundary in DOWNLOAD_FAIL CHECKSUM_FAIL; do
    export "$boundary=yes"
    run install_dot --yes
    [ "$status" -eq 0 ]
    [ ! -e "$HOME/dependency-installed" ]
    [ -x "$HOME/.local/bin/dot" ]
    [[ "$(cat "$TRACE")" != *'apt-get --yes --no-remove --no-install-recommends install /'* ]]
    [[ "$(cat "$TRACE")" != *'curl '* ]]
    unset "$boundary"
  done
}

@test "Ubuntu incompatible GitHub CLI is refused without repository writes" {
  ubuntu_packages
  export FAKE_GH_VERSION=2.45.0
  for boundary in CHECKSUM_FAIL REPOSITORY_FAIL; do
    export "$boundary=yes"
    run install_dot --yes
    [ "$status" -ne 0 ]
    [ ! -e "$HOME/gh-installed" ]
    [ ! -e "$HOME/.local/bin/dot" ]
    [[ "$output" != *'dot installation complete'* ]]
    unset "$boundary"
  done
  export FAKE_GH_VERSION=2.80.0
  run install_dot --yes
  [ "$status" -eq 0 ]
  [ -x "$HOME/.local/bin/dot" ]
}

@test "Ubuntu conflicting apt sources and redirected key paths are left untouched" {
  ubuntu_packages
  export FAKE_GH_VERSION=2.45.0
  mkdir -p "$HOME/system/etc/apt/sources.list.d" "$HOME/system/etc/apt/keyrings"
  printf 'unrelated source\n' > "$HOME/system/etc/apt/sources.list.d/dot-github-cli.list"
  run install_dot --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'missing/incompatible prerequisite gh'* ]]
  [ "$(cat "$HOME/system/etc/apt/sources.list.d/dot-github-cli.list")" = 'unrelated source' ]
  rm "$HOME/system/etc/apt/sources.list.d/dot-github-cli.list"
  printf 'unrelated key\n' > "$HOME/destination"
  ln -s "$HOME/destination" "$HOME/system/etc/apt/keyrings/dot-github-cli.gpg"
  run install_dot --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'missing/incompatible prerequisite gh'* ]]
  [ "$(cat "$HOME/destination")" = 'unrelated key' ]
  [ ! -e "$HOME/gh-installed" ]
}

@test "Ubuntu unattended identity authentication and management conflicts preserve the common gates" {
  ubuntu_packages
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
  mkdir -p "$HOME/.config/chezmoi"
  printf 'sourceDir="/legacy/source"\n' > "$HOME/.config/chezmoi/chezmoi.toml"
  run install_dot --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'explicit dot init review required'* ]]
  [ ! -e "$HOME/.local/bin/dot" ]
  [[ "$(cat "$TRACE")" != *'apt-get'* ]]
}

@test "Ubuntu future supported releases reuse dependencies without accidentally selecting Homebrew" {
  for release in 24.10 25.04 26.04; do
    export FAKE_UBUNTU="$release"
    run install_dot --yes --stage dependencies
    [ "$status" -eq 0 ]
    [[ "$output" == *'Dependency detection only'* ]]
  done
  [[ "$(cat "$TRACE")" != *'brew '* ]]
}
