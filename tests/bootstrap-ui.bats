#!/usr/bin/env bats
load bootstrap-helper

setup() {
  command -v tmux >/dev/null || skip 'tmux required'
  bootstrap_setup
  SOCKET="bootstrap-ui-$$-$BATS_TEST_NUMBER"
}
teardown() { tmux -L "$SOCKET" kill-server 2>/dev/null || true; }
pane() { tmux -L "$SOCKET" capture-pane -p -S -100 -t ui; }
wait_for() {
  local i
  for i in {1..150}; do
    [[ "$(pane)" == *"$1"* ]] && return 0
    sleep 0.1
  done
  pane >&2; return 1
}
answer() {
  wait_for "$1"
  tmux -L "$SOCKET" send-keys -t ui -l "$2"
  tmux -L "$SOCKET" send-keys -t ui Enter
}
start_bootstrap() {
  local command
  printf -v command 'cat %q | bash -s -- --source-url %q --engine-url %q; code=$?; echo UI_EXIT=$code; echo UI_DONE; sleep 30' "$BOOT" "$FIXTURE_REMOTE" "$FIXTURE_ENGINE_REMOTE"
  tmux -L "$SOCKET" new-session -d -s ui -x 150 -y 50 "$command"
}
finish_optional() {
  wait_for UI_DONE
}

@test "installed adoption-only before init defaults Finish without package or authentication actions" {
  install_dot --yes >/dev/null
  : > "$TRACE"
  missing_gum
  export PACKAGE_FAIL=yes ACCESS_DENIED=yes FAKE_OS=FreeBSD
  local command
  printf -v command 'bash %q --stage adoption; code=$?; echo UI_EXIT=$code; echo UI_DONE; sleep 30' "$BOOT"
  tmux -L "$SOCKET" new-session -d -s ui -x 150 -y 50 "$command"
  finish_optional
  [[ "$(pane)" == *'UI_EXIT=0'* ]]
  [[ "$(pane)" == *'Finish selected; no changes made'* ]]
  [ ! -e "$HOME/.config/eza/theme.yml" ]
  [ ! -e "$HOME/.config/opencode/opencode.json" ]
  [ ! -e "$HOME/.gemini/skills" ]
  [ ! -s "$TRACE" ]
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
  [ -z "$(git -C "$HOME/.local/share/dot-config" status --porcelain)" ]
}

@test "piped bootstrap reads missing identity and checkout consent from its real terminal" {
  git config --global --unset user.name
  start_bootstrap
  answer 'Git user.name:' 'Terminal Fixture'
  answer 'Install public CLI' y
  finish_optional
  [[ "$(pane)" == *'UI_EXIT=0'* ]]
  [ "$(git config --global user.name)" = 'Terminal Fixture' ]
  [ -x "$HOME/.local/bin/dot" ]
  [ ! -e "$HOME/.zsh_history" ]
}

@test "piped HOME Git detection refuses retirement and preserves metadata" {
  git init -q "$HOME"
  printf 'keep live\n' > "$HOME/.gitignore"
  start_bootstrap
  answer 'Install public CLI' y
  wait_for UI_DONE
  [[ "$(pane)" == *'UI_EXIT=1'* ]]
  [[ "$(pane)" == *'explicit migration required'* ]]
  [ -d "$HOME/.git" ]
  [ "$(cat "$HOME/.gitignore")" = 'keep live' ]
  [ ! -e "$HOME/.local/state/dot/retirement.json" ]
  [ ! -e "$HOME/.local/bin/dot" ]
}

@test "piped checkout consent cannot authorize HOME Git retirement" {
  git init -q "$HOME"
  printf 'unselected shell\n' > "$HOME/.zshrc"
  start_bootstrap
  answer 'Install public CLI' y
  finish_optional
  [[ "$(pane)" == *'UI_EXIT=1'* ]]
  [ -e "$HOME/.git" ]
  [ "$(head -n 1 "$HOME/.zshrc")" = 'unselected shell' ]
  [ ! -e "$HOME/.local/bin/dot" ]
  [ ! -e "$HOME/.config/eza/theme.yml" ]
}

@test "piped dependency cancellation performs no package transaction" {
  missing_gum
  start_bootstrap
  answer 'Install public CLI' n
  wait_for UI_DONE
  [[ "$(pane)" == *'cancelled'* ]]
  [[ "$(pane)" == *'UI_EXIT=1'* ]]
  [[ "$(cat "$TRACE")" != *'brew install'* ]]
}

@test "matching install preserves local work without fetching or prompts" {
  install_dot --yes
  printf 'keep work\n' > "$HOME/.local/share/dot-config/local-work"
  start_bootstrap
  finish_optional
  [[ "$(pane)" == *'UI_EXIT=0'* ]]
  [ "$(cat "$HOME/.local/share/dot-config/local-work")" = 'keep work' ]
}

@test "browser HTTPS login rechecks private access without SSH key generation" {
  export ACCESS_DENIED=yes
  start_bootstrap
  answer 'Authentication [switch/login/cancel]:' login
  answer 'Install public CLI' y
  finish_optional
  [[ "$(pane)" == *'UI_EXIT=0'* ]]
  [[ "$(cat "$TRACE")" == *'gh auth login --hostname github.com --git-protocol https --web --skip-ssh-key'* ]]
  [ ! -e "$HOME/.ssh" ]
}

@test "failed browser login reports failure and creates no checkout" {
  export ACCESS_DENIED=yes LOGIN_FAIL=yes
  start_bootstrap
  answer 'Authentication [switch/login/cancel]:' login
  wait_for UI_DONE
  [[ "$(pane)" == *'UI_EXIT=1'* ]]
  [[ "$(pane)" == *'incomplete during access'* ]]
  [ ! -e "$HOME/.local/share/dot" ]
}

@test "account switching is terminal driven and rechecks repository access" {
  export ACCESS_DENIED=yes
  start_bootstrap
  answer 'Authentication [switch/login/cancel]:' switch
  answer 'GitHub username:' fixture-account
  answer 'Install public CLI' y
  finish_optional
  [[ "$(pane)" == *'UI_EXIT=0'* ]]
  [[ "$(cat "$TRACE")" == *'gh auth switch --hostname github.com --user fixture-account'* ]]
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
}

@test "authentication without authorization stops once instead of retrying login" {
  export ACCESS_DENIED=yes API_STILL_DENIED=yes
  start_bootstrap
  answer 'Authentication [switch/login/cancel]:' login
  wait_for UI_DONE
  [[ "$(pane)" == *'UI_EXIT=1'* ]]
  [[ "$(pane)" == *'authorization denied after authentication'* ]]
  [ ! -e "$HOME/.local/share/dot" ]
  [ "$(grep -c '^gh auth login ' "$TRACE")" -eq 1 ]
}

@test "bootstrap consent never includes a transitive package plan" {
  missing_gum
  export BREW_CLOSURE=yes
  start_bootstrap
  wait_for 'Install public CLI'
  [[ "$(pane)" != *'lib-leaf lib-middle gum'* ]]
  answer 'Install public CLI' n
  wait_for UI_DONE
  [[ "$(pane)" == *'UI_EXIT=1'* ]]
  [[ "$(cat "$TRACE")" != *'brew install'* ]]
}

@test "Git-only access denial offers browser recovery and rechecks Git authorization" {
  deny_git_access
  start_bootstrap
  answer 'Authentication [switch/login/cancel]:' login
  answer 'Install public CLI' y
  finish_optional
  [[ "$(pane)" == *'UI_EXIT=0'* ]]
  [ -x "$HOME/.local/bin/dot" ]
  [ "$(grep -c '^gh auth login ' "$TRACE")" -eq 1 ]
}

@test "Git-only denial remaining after login stops once without cloning" {
  deny_git_access
  export GIT_STILL_DENIED=yes
  start_bootstrap
  answer 'Authentication [switch/login/cancel]:' login
  wait_for UI_DONE
  [[ "$(pane)" == *'UI_EXIT=1'* ]]
  [[ "$(pane)" == *'private Git access denied despite API access'* ]]
  [ ! -e "$HOME/.local/share/dot" ]
  [ "$(grep -c '^gh auth login ' "$TRACE")" -eq 1 ]
}

@test "Ubuntu piped setup refuses broken gh without sudo or repository changes" {
  ubuntu_fixture
  ubuntu_packages
  missing_gum
  export FAKE_GH_VERSION=2.45.0
  start_bootstrap
  wait_for UI_DONE
  [[ "$(pane)" == *'missing/incompatible prerequisite gh'* ]]
  [[ "$(cat "$TRACE")" != *'sudo '* ]]
  [[ "$(cat "$TRACE")" != *'apt-get '* ]]
  [[ "$(pane)" == *'UI_EXIT=1'* ]]
  [[ "$(cat "$TRACE")" != *'sudo '* ]]
  [ ! -e "$HOME/.local/bin/dot" ]
}

@test "Ubuntu checkout consent never authorizes an apt transaction" {
  ubuntu_fixture
  ubuntu_packages
  missing_gum
  start_bootstrap
  wait_for 'Install public CLI'
  [[ "$(pane)" != *'Inst fixture-dependency'* ]]
  [[ "$(cat "$TRACE")" != *'apt-get --yes '* ]]
  answer 'Install public CLI' n
  wait_for UI_DONE
  [[ "$(pane)" == *'UI_EXIT=1'* ]]
  [[ "$(cat "$TRACE")" != *'apt-get --yes '* ]]
  [ ! -e "$HOME/.local/bin/dot" ]
}

@test "Arch checkout cancellation invokes neither sudo nor pacman" {
  arch_fixture
  missing_gum
  start_bootstrap
  wait_for 'Install public CLI'
  [[ "$(pane)" != *'FULL SYSTEM UPGRADE'* ]]
  [[ "$(cat "$TRACE")" != *'sudo '* ]]
  answer 'Install public CLI' n
  wait_for UI_DONE
  [[ "$(pane)" == *'UI_EXIT=1'* ]]
  [[ "$(cat "$TRACE")" != *'pacman '* ]]
  [ ! -e "$HOME/.local/bin/dot" ]
}

@test "Arch default No at checkout consent stops without package effects" {
  arch_fixture
  missing_gum
  start_bootstrap
  answer 'Install public CLI' ''
  wait_for UI_DONE
  [[ "$(pane)" == *'UI_EXIT=1'* ]]
  [[ "$(pane)" == *'cancelled'* ]]
  [[ "$(cat "$TRACE")" != *'--noconfirm'* ]]
  [ ! -e "$HOME/.local/share/dot" ]
}

@test "Arch browser recovery installs split tooling without tracker or package effects" {
  arch_fixture
  missing_gum
  export ACCESS_DENIED=yes
  start_bootstrap
  answer 'Authentication [switch/login/cancel]:' login
  answer 'Install public CLI' y
  finish_optional
  [[ "$(pane)" == *'UI_EXIT=0'* ]]
  [ -x "$HOME/.local/bin/dot" ]
  [ ! -e "$HOME/.config/eza/theme.yml" ]
  run "$HOME/.local/bin/dot" scope show
  [ "$status" -ne 0 ]
  [[ "$output" == *'run dot init'* ]]
  [[ "$(cat "$TRACE")" != *'pacman '* ]]
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
}
