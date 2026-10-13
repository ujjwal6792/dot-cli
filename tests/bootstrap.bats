#!/usr/bin/env bats
load bootstrap-helper

@test "adoption-only unattended entry never installs authenticates or selects configuration" {
  run install_dot --yes
  [ "$status" -eq 0 ]
  : > "$TRACE"
  missing_gum
  export PACKAGE_FAIL=yes ACCESS_DENIED=yes
  run install_dot --stage adoption --yes
  [ "$status" -eq 0 ]
  [[ "$output" == *'does not select or approve'* ]]
  [ ! -s "$TRACE" ]
  [ ! -e "$HOME/.config/eza/theme.yml" ]
}

@test "bootstrap installs a shell-callable launcher with empty scope and preserves identity" {
  git config --global commit.gpgsign true
  git config --global user.signingkey fixture-key
  git config --global credential.https://unrelated.example.helper fixture-helper
  printf 'keep shell\n' > "$HOME/.zshrc"
  printf 'keep bash\n' > "$HOME/.bashrc"
  run install_dot --yes --directory "$HOME/install with spaces"
  [ "$status" -eq 0 ]
  [ -x "$HOME/.local/bin/dot" ]
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
  [ "$(git config --global user.name)" = 'Fixture User' ]
  [ "$(git config --global commit.gpgsign)" = true ]
  [ "$(git config --global user.signingkey)" = fixture-key ]
  [ "$(git config --global credential.https://unrelated.example.helper)" = fixture-helper ]
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
  [ "$(git -C "$HOME/install with spaces" rev-parse --abbrev-ref '@{upstream}')" = origin/main ]
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
  [ ! -e "$HOME/.config/eza/theme.yml" ]
  [ "$(head -n 1 "$HOME/.zshrc")" = 'keep shell' ]
  [[ "$(cat "$TRACE")" != *'brew install'* ]]
  [[ "$(cat "$TRACE")" != *'gh api repos/'* ]]
  [[ "$(cat "$TRACE")" != *'gh auth setup-git --hostname'* ]]
}

@test "Intel and older macOS are rejected even with every dependency present" {
  export FAKE_ARCH=x86_64
  run install_dot --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'Intel/Rosetta macOS is unsupported'* ]]
  [ ! -s "$TRACE" ]
  export FAKE_ARCH=arm64 FAKE_MACOS=14.7
  run install_dot --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'macOS 15+ is required'* ]]
  [ ! -s "$TRACE" ]
}

@test "unsupported OS and root execution fail before any dependency transaction" {
  export FAKE_OS=FreeBSD
  run install_dot --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'unsupported operating system'* ]]
  export FAKE_OS=Darwin FAKE_UID=0
  run install_dot --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'do not run the whole installer as root'* ]]
  [ ! -s "$TRACE" ]
}

@test "missing dependency is reported without package installation" {
  missing_gum
  run install_dot --yes
  [ "$status" -eq 0 ]
  [[ "$output" == *'dot init'* ]]
  [ -x "$HOME/.local/bin/dot" ]
  [[ "$(cat "$TRACE")" != *'brew install'* ]]
  [[ "$(cat "$TRACE")" != *sudo* ]]
}

@test "package manager availability does not imply bootstrap package operations" {
  missing_gum
  export VERIFY_FAIL=yes
  run install_dot --yes
  [ "$status" -eq 0 ]
  [[ "$(cat "$TRACE")" != *'brew install'* ]]
}

@test "unattended installation requires identity and real private Git access" {
  git config --global --unset user.email
  run bash -c 'cd "$HOME"; git config --show-origin --get user.email'
  [ "$status" -ne 0 ]
  run install_dot --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'--yes requires existing Git user.email'* ]]
  git config --global user.email fixture@example.org
  export ACCESS_DENIED=yes
  deny_git_access
  run install_dot --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'--yes requires existing access'* ]]
  [ ! -e "$HOME/.local/bin/dot" ]
  [ ! -e "$HOME/.local/share/dot-cli" ]
  [ ! -e "$HOME/.local/share/dot-config" ]
  export ACCESS_DENIED=no
  run bash "$BOOT" --yes --source-url "$BATS_TEST_TMPDIR/not-a-repository"
  [ "$status" -ne 0 ]
  [[ "$output" == *'private Git access denied'* ]]
}

@test "existing tracker and HOME git are detected or preserved without retirement" {
  mkdir -p "$HOME/.config/chezmoi"
  printf 'sourceDir="/old/source"\n[hooks.read-source-state.pre]\ncommand="touch"\nargs=["%s/hook-ran"]\n' "$HOME" > "$HOME/.config/chezmoi/chezmoi.toml"
  run install_dot --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'Existing chezmoi tracker detected read-only'* ]]
  [ ! -e "$HOME/hook-ran" ]
  [ ! -e "$HOME/.local/bin/dot" ]
  [ -e "$HOME/.config/chezmoi/chezmoi.toml" ]
  mv "$HOME/.config/chezmoi" "$HOME/legacy-kept"
  git init -q "$HOME"
  run install_dot --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'legacy installation detected'* ]]
  [ -d "$HOME/.git" ]
}

@test "conflicting checkout launcher and symlink startup files are never overwritten" {
  mkdir -p "$HOME/.local/share/dot-config"
  printf 'unrelated\n' > "$HOME/.local/share/dot-config/user-file"
  run install_dot --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'partial/unrecorded split installation conflict'* ]]
  [ "$(cat "$HOME/.local/share/dot-config/user-file")" = unrelated ]
  run install_dot --yes --directory "$HOME/other"
  [ "$status" -eq 0 ]
  printf 'other launcher\n' > "$HOME/.local/bin/dot"
  run install_dot --yes --directory "$HOME/other"
  [ "$status" -ne 0 ]
  [[ "$output" == *'unrelated launcher conflict'* || "$output" == *'unrelated launcher conflict; left untouched'* ]]
  [ "$(cat "$HOME/.local/bin/dot")" = 'other launcher' ]
}

@test "reruns keep selected scope and startup contents and support custom ZDOTDIR" {
  export ZDOTDIR="$HOME/zsh startup" XDG_CONFIG_HOME="$HOME/config directory"
  mkdir -p "$ZDOTDIR"
  printf 'existing login\n' > "$HOME/.profile"
  run install_dot --yes
  [ "$status" -eq 0 ]
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
  [ ! -e "$HOME/config directory/chezmoi/chezmoi.toml" ]
  printf 'startup before\n' > "$HOME/zsh-before"
  run install_dot --yes
  [ "$status" -eq 0 ]
  [ ! -e "$HOME/zsh startup/.zprofile" ]
  [ "$(cat "$HOME/zsh-before")" = 'startup before' ]
  [ ! -e "$HOME/.bash_profile" ]
  [ ! -e "$HOME/zsh startup/.zshrc" ]
  [ ! -e "$HOME/.config/eza/theme.yml" ]
}

@test "repeat setup detects installed layout without fetching or changing either checkout" {
  install_dot --yes
  printf 'local shell\n' > "$HOME/.zshrc"
  before="$(git -C "$HOME/.local/share/dot-config" rev-parse HEAD)"
  engine_before="$(git -C "$HOME/.local/share/dot-cli" rev-parse HEAD)"
  run install_dot --yes
  [ "$status" -eq 0 ]
  [ "$(git -C "$HOME/.local/share/dot-config" rev-parse HEAD)" = "$before" ]
  [ "$(git -C "$HOME/.local/share/dot-cli" rev-parse HEAD)" = "$engine_before" ]
  [ "$(head -n 1 "$HOME/.zshrc")" = 'local shell' ]
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
}

@test "missing dependencies are detected without invoking Homebrew" {
  missing_gum
  missing_brew
  run install_dot --yes
  [ "$status" -eq 0 ]
  [[ "$output" == *'No packages'* ]]
  [ ! -e "$HOME/homebrew-installed" ]
  [[ "$(cat "$TRACE")" != *'brew install'* ]]
  [ -x "$HOME/.local/bin/dot" ]
}

@test "bootstrap never provisions Homebrew or packages" {
  missing_gum
  missing_brew
  # External checksum verifier fixture: the preceding test uses real shasum
  # against corrupt bytes; this route simulates an approved upstream artifact.
  cat > "$FIXTURE_BIN/shasum" <<'SH'
#!/bin/bash
echo 'ece8385d14080da35eb3c1f72cf43b71be5d1a3838e554e4bb748b9888b15cec  fixture'
SH
  chmod +x "$FIXTURE_BIN/shasum"
  run install_dot --yes
  [ "$status" -eq 0 ]
  [ ! -e "$HOME/homebrew-installed" ]
  [[ "$(cat "$TRACE")" != *'brew install'* ]]
  [[ "$(cat "$TRACE")" != *'fixture Homebrew provisioning'* ]]
  [[ "$(cat "$TRACE")" != *'brew install'* ]]
}

@test "startup symlinks and malformed markers fail without touching their destinations" {
  printf 'unrelated startup\n' > "$HOME/destination"
  ln -s "$HOME/destination" "$HOME/.bashrc"
  run install_dot --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'symlink'* ]]
  [ "$(cat "$HOME/destination")" = 'unrelated startup' ]
  unlink "$HOME/.bashrc"
  printf '# >>> dot installer PATH >>>\n' > "$HOME/.bashrc"
  run install_dot --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'malformed PATH markers'* || "$output" == *'unrelated checkout conflict'* ]]
  [ ! -e "$HOME/.local/bin/dot" ]
}

@test "dependency and migration stages are read-only and adoption never mutates targets" {
  for stage in dependencies migration adoption; do
    run install_dot --yes --stage "$stage"
    [ "$status" -eq 0 ]
    [ ! -e "$HOME/.local/share/dot-config" ]
    [ ! -e "$HOME/.local/share/dot-cli" ]
    [ ! -e "$HOME/.local/bin/dot" ]
    [ ! -e "$HOME/.local/state/dot/adoption.json" ]
    [[ "$(cat "$TRACE")" != *'brew install'* ]]
  done
}

@test "without a terminal interactive bootstrap gives actionable unattended guidance" {
  run python3 - "$BOOT" "$FIXTURE_REMOTE" <<'PY'
import os, subprocess, sys
result = subprocess.run(['bash', sys.argv[1], '--source-url', sys.argv[2]], preexec_fn=os.setsid)
sys.exit(result.returncode)
PY
  [ "$status" -ne 0 ]
  [[ "$output" == *'no controlling terminal; rerun with --yes'* ]]
  [ ! -e "$HOME/.local/bin/dot" ]
}

@test "incompatible Python is detected without package provisioning" {
  export REAL_PYTHON="$(command -v python3)"
  cat > "$FIXTURE_BIN/python3" <<'SH'
#!/bin/bash
if [ "$1" = -c ] && [ ! -e "$HOME/dependency-installed" ]; then exit 1; fi
exec "$REAL_PYTHON" "$@"
SH
  chmod +x "$FIXTURE_BIN/python3"
  run install_dot --yes
  [ "$status" -ne 0 ]
  [[ "$(cat "$TRACE")" != *'brew install'* ]]
}

@test "known-broken GitHub CLI never triggers a macOS bottle transaction" {
  for version in 2.45.0 2.46.2; do
    rm -f "$HOME/bottle-gh"
    export FAKE_GH_VERSION="$version"
    run install_dot --yes --stage dependencies
    [ "$status" -eq 0 ]
    [[ "$(cat "$TRACE")" != *'brew install'* ]]
    [[ "$(cat "$TRACE")" != *'sudo '* ]]
  done
}

@test "unsupported Homebrew layout is irrelevant because bootstrap never invokes Homebrew" {
  missing_gum
  export BREW_PREFIX="$HOME/unsupported-brew-layout"
  run install_dot --yes
  [ "$status" -eq 0 ]
  [[ "$(cat "$TRACE")" != *'brew install'* ]]
  [[ "$(cat "$TRACE")" != *'brew install'* ]]
}

@test "legacy XDG state is preserved and management overrides do not trigger retirement" {
  export XDG_DATA_HOME="$HOME/custom data" XDG_CACHE_HOME="$HOME/custom cache"
  mkdir -p "$XDG_DATA_HOME/chezmoi" "$XDG_CACHE_HOME/chezmoi"
  run install_dot --yes
  [ "$status" -eq 0 ]
  [ -d "$XDG_DATA_HOME/chezmoi" ]
  [ -d "$XDG_CACHE_HOME/chezmoi" ]
  export CHEZMOI_SOURCE_DIR="$HOME/unknown source"
  run install_dot --yes
  [ "$status" -eq 0 ]
  [ -d "$XDG_DATA_HOME/chezmoi" ]
}

@test "bootstrap never resolves package dependency closures" {
  missing_gum
  export BREW_CLOSURE=yes BREW_DUPLICATE_ROOT=yes
  run install_dot --yes
  [ "$status" -eq 0 ]
  [[ "$(cat "$TRACE")" != *'brew install'* ]]
}

@test "missing package bottles are not queried or installed by bootstrap" {
  missing_gum
  export BREW_CLOSURE=yes UNBOTTLED_PACKAGE=lib-middle
  run install_dot --yes
  [ "$status" -eq 0 ]
  [[ "$(cat "$TRACE")" != *'brew install'* ]]
  [ ! -e "$HOME/bottle-gum" ]
  [ ! -e "$HOME/source-build" ]
  [ -x "$HOME/.local/bin/dot" ]
  [[ "$(cat "$TRACE")" != *'would build implicit dependency'* ]]
}

@test "unexpected package dependencies are not resolved by bootstrap" {
  missing_gum
  export UNEXPECTED_DEPENDENCY=yes
  run install_dot --yes
  [ "$status" -eq 0 ]
  [[ "$(cat "$TRACE")" != *'brew install'* ]]
  [[ "$(cat "$TRACE")" != *'brew install'* ]]
}

@test "unexported literal ZDOTDIR is discovered without executing startup code" {
  mkdir -p "$HOME/zsh startup"
  printf 'ZDOTDIR="$HOME/zsh startup"\n' > "$HOME/.zshenv"
  run install_dot --yes
  [ "$status" -eq 0 ]
  [ ! -e "$HOME/zsh startup/.zprofile" ]
  [ ! -e "$HOME/.zprofile" ]
  run env DOT_ENGINE="$HOME/.local/share/dot-cli/tools/dot" DOT_SOURCE="$HOME/.local/share/dot-config" \
    "$HOME/.local/bin/dot" contract check --engine "$HOME/.local/share/dot-cli/tools/dot" \
    --source "$HOME/.local/share/dot-config"
  [ "$status" -eq 0 ]
  [[ "$output" == *'"protocol": 1'* ]]
}

@test "dynamic ZDOTDIR requires an explicit path and startup code is never executed" {
  printf 'touch "$HOME/startup-executed"\nZDOTDIR="$HOME/zsh startup"\n' > "$HOME/.zshenv"
  run install_dot --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'cannot safely discover ZDOTDIR'* ]]
  [ ! -e "$HOME/startup-executed" ]
  run install_dot --yes --zsh-directory "$HOME/zsh startup"
  [ "$status" -eq 0 ]
  [ ! -e "$HOME/zsh startup/.zprofile" ]
  [ ! -e "$HOME/startup-executed" ]
}

@test "ambiguous quoting and unset variables cannot redirect static ZDOTDIR discovery" {
  printf 'ZDOTDIR="~/wrong-location"\n' > "$HOME/.zshenv"
  run install_dot --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'ZDOTDIR must be absolute'* ]]
  printf 'ZDOTDIR="${UNDEFINED_FIXTURE_VARIABLE}/zsh"\n' > "$HOME/.zshenv"
  run install_dot --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'cannot safely discover ZDOTDIR'* ]]
  [ ! -e "$HOME/.local/bin/dot" ]
}

@test "setup leaves shell startup bytes unchanged across repeats" {
  install_dot --yes
  printf '\n\n# unrelated trailing content\n\n' >> "$HOME/.bashrc"
  cp "$HOME/.bashrc" "$HOME/startup-before"
  run install_dot --yes
  [ "$status" -eq 0 ]
  cmp "$HOME/startup-before" "$HOME/.bashrc"
}

@test "embedded hash characters in unexported ZDOTDIR are not mistaken for comments" {
  printf 'ZDOTDIR=$HOME/zsh#config # a real trailing comment\n' > "$HOME/.zshenv"
  run install_dot --yes
  [ "$status" -eq 0 ]
  [ ! -e "$HOME/zsh#config/.zprofile" ]
  [ ! -e "$HOME/zsh/.zprofile" ]
  [ ! -e "$HOME/zsh#config/.zprofile" ]
}

@test "a quoted command word cannot masquerade as a ZDOTDIR assignment" {
  printf '"ZDOTDIR=%s/not-an-assignment"\n' "$HOME" > "$HOME/.zshenv"
  run install_dot --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'cannot safely discover ZDOTDIR'* ]]
  [ ! -e "$HOME/not-an-assignment/.zprofile" ]
  [ ! -e "$HOME/.local/bin/dot" ]
}
