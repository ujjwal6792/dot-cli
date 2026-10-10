#!/usr/bin/env bats
load bootstrap-helper

setup() {
  bootstrap_setup
  cat > "$FIXTURE_BIN/chezmoi" <<'SH'
#!/bin/bash
echo "chezmoi $*" >> "$TRACE"
[ "$*" = --version ] || exit 90
[ "${CHEZMOI_MISSING:-no}" != yes ] || [ -e "$HOME/bottle-chezmoi" ] || exit 1
echo 'chezmoi version fixture'
SH
  chmod +x "$FIXTURE_BIN/chezmoi"
  printf '#!/bin/bash\necho "gum version fixture"\n' > "$FIXTURE_BIN/gum"
  chmod +x "$FIXTURE_BIN/gum"
  install_dot --yes
  : > "$TRACE"
  SOCKET="init-$$-$BATS_TEST_NUMBER"
}

teardown() { tmux -L "$SOCKET" kill-server 2>/dev/null || true; }
pane() { tmux -L "$SOCKET" capture-pane -p -S -200 -t ui; }
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
start_init() {
  local command
  printf -v command 'trap : INT; %q init %s; echo UI_EXIT=$?; sleep 30' "$HOME/.local/bin/dot" "$*"
  tmux -L "$SOCKET" new-session -d -s ui -x 180 -y 60 "$command"
}

@test "guided tracker creation needs separate consent and repeat detection preserves exact bytes" {
  start_init
  answer 'Create this tracker' y
  answer 'Setup action' ''
  wait_for UI_EXIT=0
  [ -f "$HOME/.config/chezmoi/chezmoi.toml" ]
  grep -F "sourceDir = \"$HOME/.local/share/dot-config/home\"" "$HOME/.config/chezmoi/chezmoi.toml"
  grep -F 'name = "Fixture User"' "$HOME/.config/chezmoi/chezmoi.toml"
  cp "$HOME/.config/chezmoi/chezmoi.toml" "$BATS_TEST_TMPDIR/created"
  cp "$HOME/.local/state/dot/install.json" "$BATS_TEST_TMPDIR/record"
  run "$HOME/.local/bin/dot" init --check
  [ "$status" -eq 0 ]
  [[ "$output" == *'Compatible tracker detected'* ]]
  cmp "$BATS_TEST_TMPDIR/created" "$HOME/.config/chezmoi/chezmoi.toml"
  cmp "$BATS_TEST_TMPDIR/record" "$HOME/.local/state/dot/install.json"
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
  [ ! -e "$HOME/.local/state/dot/chezmoi-state.boltdb" ]
  [ ! -e "$HOME/.config/eza/theme.yml" ]
}

@test "Finish default does not select files or invoke prebuilt browser or package install" {
  start_init
  answer 'Create this tracker' y
  answer 'Setup action' ''
  wait_for UI_EXIT=0
  [[ "$(pane)" == *'Finish; adoption scope unchanged'* ]]
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
  [ ! -e "$HOME/.config/eza/theme.yml" ]
  ! grep -E '^brew (list|install)' "$TRACE"
}

@test "init refuses mismatched tracker without executing hooks or changing machine data" {
  mkdir -p "$HOME/.config/chezmoi"
  printf 'sourceDir="/unrelated/home"\n[data]\ndotEngine="/unrelated/engine"\nname="Keep me"\n[hooks.read-source-state.pre]\ncommand="touch"\nargs=["%s/hook-ran"]\n' "$HOME" > "$HOME/.config/chezmoi/chezmoi.toml"
  cp "$HOME/.config/chezmoi/chezmoi.toml" "$BATS_TEST_TMPDIR/before"
  run "$HOME/.local/bin/dot" init --check
  [ "$status" -ne 0 ]
  [[ "$output" == *'/unrelated/home'* ]]
  [[ "$output" == *'migration'* ]]
  cmp "$BATS_TEST_TMPDIR/before" "$HOME/.config/chezmoi/chezmoi.toml"
  [ ! -e "$HOME/hook-ran" ]
  [ ! -s "$TRACE" ]
}

@test "installed init detects missing tracker read-only with actual paths" {
  run "$HOME/.local/bin/dot" init --check
  [ "$status" -eq 0 ]
  [[ "$output" == *"$HOME/.local/share/dot-config/home"* ]]
  [[ "$output" == *"$HOME/.config/chezmoi/chezmoi.toml"* ]]
  [[ "$output" == *'Tracker pending'* ]]
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
  [ ! -s "$TRACE" ]
}

@test "init explicit source and config arguments override environment paths" {
  source="$HOME/.local/share/dot-config"
  config="$HOME/.config/chezmoi/chezmoi.toml"
  run env DOT_SOURCE="$HOME/unrelated-source" DOT_CONFIG="$HOME/unrelated-config" \
    "$HOME/.local/bin/dot" init --source "$source" --config "$config" --check
  [ "$status" -eq 0 ]
  [[ "$output" == *"Private source: $source/home"* ]]
  [[ "$output" == *"Actual config: $config"* ]]
  [ ! -e "$config" ]
}

@test "private retrieval is explicit and tracker creation remains a separate approval" {
  source="$HOME/.local/share/dot-config"
  config="$HOME/.config/chezmoi/chezmoi.toml"
  rm -rf "$source"
  python3 - "$HOME/.local/state/dot/install.json" <<'PY'
import json,sys
p=sys.argv[1]
d=json.load(open(p))
d['source']='https://github.com/ujjwal6792/dotfiles'
json.dump(d,open(p,'w'))
PY
  cat > "$FIXTURE_BIN/gh" <<'SH'
#!/bin/bash
echo "gh $*" >> "$TRACE"
case "$1" in
  --version) echo 'gh version 2.80.0' ;;
  auth) exit 0 ;;
  api) printf '{"full_name":"ujjwal6792/dotfiles","private":true}\n' ;;
  repo)
    [ "$2" = clone ] && [ "$3" = ujjwal6792/dotfiles ] || exit 9
    git clone -q "$FIXTURE_REMOTE" "$4" || exit
    git -C "$4" remote set-url origin https://github.com/ujjwal6792/dotfiles
    ;;
  *) exit 9 ;;
esac
SH
  chmod +x "$FIXTURE_BIN/gh"
  printf -v command 'trap : INT; %q init --source %q --config %q; echo UI_EXIT=$?; sleep 30' "$HOME/.local/bin/dot" "$source" "$config"
  tmux -L "$SOCKET" new-session -d -s ui -x 180 -y 60 "$command"
  answer 'Retrieve this exact private repository' y
  answer 'Create this tracker' n
  wait_for UI_EXIT=0
  [ -d "$source/.git" ]
  [ "$(git -C "$source" remote get-url origin)" = 'https://github.com/ujjwal6792/dotfiles' ]
  [[ "$(pane)" == *'Private checkout retrieved'* ]]
  [ ! -e "$config" ]
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
  ! grep -E '^gh repo create|chezmoi .* (init|apply)' "$TRACE"
}

@test "noninteractive init retrieval fails closed before private API access" {
  source="$HOME/.local/share/dot-config"
  rm -rf "$source"
  run "$HOME/.local/bin/dot" init --source "$source" --config "$HOME/.config/chezmoi/chezmoi.toml"
  [ "$status" -eq 0 ]
  [[ "$output" == *'retrieval declined'* ]]
  [ ! -e "$source" ]
  ! grep -E '^gh api|^gh repo clone' "$TRACE"
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
}

@test "approved private retrieval denied by GitHub never creates or substitutes a repository" {
  source="$HOME/.local/share/dot-config"
  rm -rf "$source"
  cat > "$FIXTURE_BIN/gh" <<'SH'
#!/bin/bash
echo "gh $*" >> "$TRACE"
case "$1" in
  --version) echo 'gh version 2.80.0' ;;
  auth) exit 0 ;;
  api) echo 'private access denied' >&2; exit 1 ;;
  repo) echo 'unexpected clone' >&2; exit 9 ;;
  *) exit 9 ;;
esac
SH
  chmod +x "$FIXTURE_BIN/gh"
  printf -v command 'trap : INT; %q init --source %q --config %q; echo UI_EXIT=$?; sleep 30' "$HOME/.local/bin/dot" "$source" "$HOME/.config/chezmoi/chezmoi.toml"
  tmux -L "$SOCKET" new-session -d -s ui -x 180 -y 60 "$command"
  answer 'Retrieve this exact private repository' y
  wait_for UI_EXIT=1
  [[ "$(pane)" == *'private repository access denied'* ]]
  [[ "$(pane)" != *'Create this tracker'* ]]
  [ ! -e "$source" ]
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
  ! grep -E '^gh repo create|^gh repo clone' "$TRACE"
}

@test "missing dependency refusal leaves tracker pending and performs no install" {
  export CHEZMOI_MISSING=yes
  start_init
  answer 'Install dependency chezmoi' n
  wait_for UI_EXIT=0
  [[ "$(pane)" == *'Remaining dependency: chezmoi'* ]]
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
  [ ! -e "$HOME/bottle-chezmoi" ]
  ! grep -E '^brew install --force-bottle ' "$TRACE"
}

@test "dependency install reviews exact manager operation but does not authorize tracker creation" {
  export CHEZMOI_MISSING=yes
  start_init
  answer 'Install dependency chezmoi' y
  answer 'Create this tracker' n
  wait_for UI_EXIT=0
  [[ "$(pane)" == *'Homebrew'* ]]
  [[ "$(pane)" == *'dependencies'* ]]
  [ -e "$HOME/bottle-chezmoi" ]
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
  grep -E '^brew install --force-bottle -- chezmoi$' "$TRACE"
  ! grep -E '^chezmoi .* (init|apply)' "$TRACE"
}

@test "init Install missing consumes private desired plan with independent exact approval" {
  mkdir -p "$HOME/.local/share/dot-config/packages"
  printf 'brew "fixture-app"\n' > "$HOME/.local/share/dot-config/packages/Brewfile"
  start_init
  answer 'Create this tracker' y
  answer 'Setup action' packages
  wait_for 'INSTALL MISSING'
  wait_for 'Approve exact package plan ID'
  id="$(pane | sed -n 's/^Plan ID: \([a-f0-9]*\).*/\1/p' | tail -1)"
  [ "${#id}" -eq 64 ]
  answer 'Approve exact package plan ID' "$id"
  answer 'Setup action' ''
  wait_for UI_EXIT=0
  grep -E '^brew install --formula -- fixture-app$' "$TRACE"
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
  [ ! -e "$HOME/.config/eza/theme.yml" ]
}

@test "optional rounds invoke installed prebuilt browser with explicit identity and no Go build" {
  mkdir -p "$HOME/.local/share/dot-cli/bin"
  printf '#!/bin/bash\nprintf "browser %%s\\n" "$*" >> "$TRACE"\n' > "$HOME/.local/share/dot-cli/bin/dot-browser"
  chmod +x "$HOME/.local/share/dot-cli/bin/dot-browser"
  start_init
  answer 'Create this tracker' y
  answer 'Setup action' rounds
  wait_for 'Browser returned'
  answer 'Setup action' ''
  wait_for UI_EXIT=0
  grep -F -- "--engine $HOME/.local/share/dot-cli/tools/dot --source $HOME/.local/share/dot-config --home $HOME --rounds" "$TRACE"
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
}

@test "public export includes runnable init without private package manifests" {
  run python3 "$REPO/scripts/export-public.py" --output "$BATS_TEST_TMPDIR/public"
  [ "$status" -eq 0 ]
  [ -f "$BATS_TEST_TMPDIR/public/tools/dot/init.py" ]
  [ -f "$BATS_TEST_TMPDIR/public/tools/dot/init-python-bootstrap.sh" ]
  [ ! -e "$BATS_TEST_TMPDIR/public/packages" ]
  run env DOT_ENGINE="$BATS_TEST_TMPDIR/public/tools/dot" DOT_SOURCE="$HOME/.local/share/dot-config" DOT_CONFIG="$HOME/.config/chezmoi/chezmoi.toml" "$BATS_TEST_TMPDIR/public/tools/dot/dot" init --help
  [ "$status" -eq 0 ]
  [[ "$output" == *'--check'* ]]
}

@test "Ubuntu upstream dependency checksum failure never installs or creates tracker" {
  ubuntu_fixture
  ubuntu_packages
  export CHEZMOI_MISSING=yes
  start_init
  wait_for 'https://github.com/twpayne/chezmoi/releases/download/v2.73.0/'
  answer 'Install dependency chezmoi' y
  wait_for UI_EXIT=1
  [[ "$(pane)" == *'checksum mismatch'* ]]
  ! grep -E '^apt-get .*install' "$TRACE"
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
}

@test "alternate tracker formats and dangling config symlinks block creation without deleting data" {
  mkdir -p "$HOME/.config/chezmoi"
  printf '{"sourceDir":"/other"}\n' > "$HOME/.config/chezmoi/chezmoi.json"
  run "$HOME/.local/bin/dot" init --check
  [ "$status" -ne 0 ]
  [[ "$output" == *'alternate tracker'* ]]
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
  [ -f "$HOME/.config/chezmoi/chezmoi.json" ]
}

@test "bootstrap optional setup routes through init and the installed prebuilt rounds" {
  start_init
  answer 'Create this tracker' y
  answer 'Setup action' ''
  wait_for UI_EXIT=0
  tmux -L "$SOCKET" kill-server
  mkdir -p "$HOME/.local/share/dot-cli/bin"
  printf '#!/bin/bash\necho "prebuilt rounds $*" >> "$TRACE"\n' > "$HOME/.local/share/dot-cli/bin/dot-browser"
  chmod +x "$HOME/.local/share/dot-cli/bin/dot-browser"
  command="$(printf 'bash %q --stage adoption; echo UI_EXIT=$?; sleep 30' "$BOOT")"
  tmux -L "$SOCKET" new-session -d -s ui -x 180 -y 60 "$command"
  wait_for UI_EXIT=0
  [[ "$(pane)" == *'Compatible tracker detected'* ]]
  grep -F -- '--rounds' "$TRACE"
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
}

@test "missing Homebrew provisioning refusal cannot authorize dependency install" {
  missing_brew
  export CHEZMOI_MISSING=yes
  start_init
  answer 'Provision Homebrew' n
  wait_for UI_EXIT=0
  [ ! -e "$HOME/homebrew-installed" ]
  [ ! -e "$HOME/bottle-chezmoi" ]
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
  ! grep -E '^curl .*https://' "$TRACE"
}

@test "known broken GitHub CLI requires reviewed replacement before tracker setup" {
  export FAKE_GH_VERSION=2.45.0
  start_init
  answer 'Install dependency gh' n
  wait_for UI_EXIT=0
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
  [ ! -e "$HOME/bottle-gh" ]
}

@test "tracker or installation identity changed during dependency review rejects mutation" {
  export CHEZMOI_MISSING=yes
  start_init
  wait_for 'Install dependency chezmoi'
  printf '{}\n' > "$HOME/.local/state/dot/install.json"
  answer 'Install dependency chezmoi' y
  wait_for UI_EXIT=1
  [ ! -e "$HOME/bottle-chezmoi" ]
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
}

@test "render failure diagnoses actual tracker config and home source without deleting machine data" {
  start_init
  answer 'Create this tracker' y
  answer 'Setup action' ''
  wait_for UI_EXIT=0
  printf 'preserve database\n' > "$HOME/.local/state/dot/chezmoi-state.boltdb"
  run "$HOME/.local/bin/dot" status
  [ "$status" -ne 0 ]
  [[ "$output" == *"--source \"$HOME/.local/share/dot-config/home\""* ]]
  [[ "$output" == *"--config \"$HOME/.config/chezmoi/chezmoi.toml\""* ]]
  [[ "$output" == *'do not delete machine data'* ]]
  [ "$(cat "$HOME/.local/state/dot/chezmoi-state.boltdb")" = 'preserve database' ]
}

@test "public remote identity mismatch is not silently accepted by init" {
  git -C "$HOME/.local/share/dot-cli" remote set-url origin /unrelated-public
  run "$HOME/.local/bin/dot" init --check
  [ "$status" -ne 0 ]
  [[ "$output" == *'public remote identity'* ]]
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
}

@test "unsupported platform stops before tracker creation or any provisioning" {
  export FAKE_OS=FreeBSD
  run "$HOME/.local/bin/dot" init
  [ "$status" -ne 0 ]
  [[ "$output" == *'unsupported platform'* ]]
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
  [ ! -s "$TRACE" ]
}

@test "package cancel never installs and changed private desired source invalidates approval" {
  mkdir -p "$HOME/.local/share/dot-config/packages"
  printf 'brew "fixture-app"\n' > "$HOME/.local/share/dot-config/packages/Brewfile"
  start_init
  answer 'Create this tracker' y
  answer 'Setup action' packages
  wait_for 'Approve exact package plan ID'
  id="$(pane | sed -n 's/^Plan ID: \([a-f0-9]*\).*/\1/p' | tail -1)"
  answer 'Approve exact package plan ID' ''
  wait_for 'Install missing canceled'
  [ ! -e "$HOME/bottle-fixture-app" ]
  answer 'Setup action' ''
  wait_for UI_EXIT=0
  tmux -L "$SOCKET" kill-server
  start_init
  answer 'Setup action' packages
  wait_for 'Approve exact package plan ID'
  id="$(pane | sed -n 's/^Plan ID: \([a-f0-9]*\).*/\1/p' | tail -1)"
  printf 'brew "different-fixture"\n' > "$HOME/.local/share/dot-config/packages/Brewfile"
  answer 'Approve exact package plan ID' "$id"
  wait_for UI_EXIT=1
  [[ "$(pane)" == *'stale plan'* ]]
  [ ! -e "$HOME/bottle-fixture-app" ]
  [ ! -e "$HOME/bottle-different-fixture" ]
}

@test "init refuses Brewfile Ruby without evaluating it or touching version pins" {
  mkdir -p "$HOME/.local/share/dot-config/packages"
  printf 'system("touch %s/ruby-ran")\n' "$HOME" > "$HOME/.local/share/dot-config/packages/Brewfile"
  cp "$HOME/.local/share/dot-config/packages/pins.toml" "$BATS_TEST_TMPDIR/pins-before"
  start_init
  answer 'Create this tracker' y
  answer 'Setup action' packages
  wait_for UI_EXIT=1
  [[ "$(pane)" == *'unsupported source syntax'* ]]
  [ ! -e "$HOME/ruby-ran" ]
  cmp "$BATS_TEST_TMPDIR/pins-before" "$HOME/.local/share/dot-config/packages/pins.toml"
}

@test "denied private access does not switch or recreate source during init" {
  export ACCESS_DENIED=yes API_STILL_DENIED=yes
  run "$HOME/.local/bin/dot" init --check
  [ "$status" -eq 0 ]
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
  [ ! -s "$TRACE" ]
  [ "$(git -C "$HOME/.local/share/dot-config" remote get-url origin)" = "$FIXTURE_REMOTE" ]
  [ ! -e "$HOME/logged-in" ]
}

@test "dangling tracker symlink fails before creating any machine data" {
  mkdir -p "$HOME/.config/chezmoi"
  ln -s "$HOME/missing-sentinel" "$HOME/.config/chezmoi/chezmoi.toml"
  run "$HOME/.local/bin/dot" init --check
  [ "$status" -ne 0 ]
  [[ "$output" == *'symlink path conflict'* ]]
  [ -L "$HOME/.config/chezmoi/chezmoi.toml" ]
  [ ! -e "$HOME/missing-sentinel" ]
  [ ! -s "$TRACE" ]
}

@test "blanket yes is not dependency tracker package or adoption approval" {
  run "$HOME/.local/bin/dot" init --yes
  [ "$status" -ne 0 ]
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
  [ ! -s "$TRACE" ]
}

@test "tracker appearing after review is preserved instead of overwritten" {
  start_init
  wait_for 'Create this tracker'
  mkdir -p "$HOME/.config/chezmoi"
  printf 'sourceDir="/different"\n[data]\nname="preserve"\n' > "$HOME/.config/chezmoi/chezmoi.toml"
  cp "$HOME/.config/chezmoi/chezmoi.toml" "$BATS_TEST_TMPDIR/late-tracker"
  answer 'Create this tracker' y
  wait_for UI_EXIT=1
  cmp "$BATS_TEST_TMPDIR/late-tracker" "$HOME/.config/chezmoi/chezmoi.toml"
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
}

@test "Arch dependency review discloses full upgrade and retains native confirmation" {
  ubuntu_fixture
  export FAKE_DISTRO=arch FAKE_GH_VERSION=2.45.0
  cat > "$FIXTURE_BIN/pacman" <<'SH'
#!/bin/bash
echo "pacman $*" >> "$TRACE"
case "$1" in
  -Sp) echo 'github-cli fixture-version'; exit 0 ;;
  -Syu) printf 'Native transaction confirmation [y/N] '; read -r answer; [ "$answer" = y ] || exit 1; touch "$HOME/gh-installed" ;;
  *) exit 90 ;;
esac
SH
  cat > "$FIXTURE_BIN/sudo" <<'SH'
#!/bin/bash
echo "sudo $*" >> "$TRACE"
[ "$1" = "$FIXTURE_BIN/pacman" ] || exit 90
exec "$@"
SH
  chmod +x "$FIXTURE_BIN/pacman" "$FIXTURE_BIN/sudo"
  start_init
  wait_for 'FULL SYSTEM UPGRADE'
  answer 'Install dependency gh' y
  answer 'Native transaction confirmation' y
  answer 'Create this tracker' n
  wait_for UI_EXIT=0
  grep -F 'pacman -Syu -- extra/github-cli' "$TRACE"
  ! grep -E 'noconfirm|yay|pin' "$TRACE"
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
}

@test "Ubuntu dependency apt review uses named packages with no repository edits or full upgrade" {
  ubuntu_fixture
  export FAKE_GH_VERSION=2.45.0
  cat > "$FIXTURE_BIN/apt-get" <<'SH'
#!/bin/bash
echo "apt-get $*" >> "$TRACE"
if [ "$1" = -s ]; then echo 'Inst gh (fixture-version fixture-repository)'; exit 0; fi
[ "$1" = install ] || exit 90
touch "$HOME/gh-installed"
SH
  cat > "$FIXTURE_BIN/sudo" <<'SH'
#!/bin/bash
echo "sudo $*" >> "$TRACE"
[ "$1" = "$FIXTURE_BIN/apt-get" ] || exit 90
exec "$@"
SH
  chmod +x "$FIXTURE_BIN/apt-get" "$FIXTURE_BIN/sudo"
  start_init
  answer 'Install dependency gh' y
  answer 'Create this tracker' n
  wait_for UI_EXIT=0
  grep -F 'apt-get install --no-remove --no-install-recommends -- gh' "$TRACE"
  ! grep -E 'upgrade|apt-get update|pin' "$TRACE"
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
}

@test "init enforces supported native OS release and architecture before creation" {
  export FAKE_MACOS=14.0
  run "$HOME/.local/bin/dot" init
  [ "$status" -ne 0 ]
  [[ "$output" == *'macOS 15+'* ]]
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
}

@test "incompatible contract diagnostics identify actual source and config before mutation" {
  printf '{}\n' > "$HOME/.local/share/dot-config/dot-contract.json"
  run "$HOME/.local/bin/dot" init --check
  [ "$status" -ne 0 ]
  [[ "$output" == *"$HOME/.local/share/dot-config/home"* ]]
  [[ "$output" == *"$HOME/.config/chezmoi/chezmoi.toml"* ]]
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
  [ ! -s "$TRACE" ]
}

@test "missing machine identity is prompted locally without changing Git configuration" {
  git config --global --unset user.name
  git config --global --unset user.email
  start_init
  answer 'Tracker full name' 'Local Fixture'
  answer 'Tracker email' 'local@example.invalid'
  answer 'Create this tracker' y
  answer 'Setup action' ''
  wait_for UI_EXIT=0
  grep -F 'name = "Local Fixture"' "$HOME/.config/chezmoi/chezmoi.toml"
  run git config --global user.name
  [ "$status" -ne 0 ]
  run git config --global user.email
  [ "$status" -ne 0 ]
}

@test "dependency refusal reports every remaining requirement without bundling installs" {
  export CHEZMOI_MISSING=yes
  missing_gum
  start_init
  answer 'Install dependency chezmoi' n
  wait_for UI_EXIT=0
  [[ "$(pane)" == *'Remaining dependencies: chezmoi, gum'* ]]
  [ ! -e "$HOME/bottle-chezmoi" ]
  [ ! -e "$HOME/dependency-installed" ]
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
}

@test "malformed installation metadata fails with actionable diagnostics instead of a traceback" {
  printf '[]\n' > "$HOME/.local/state/dot/install.json"
  run "$HOME/.local/bin/dot" init --check
  [ "$status" -ne 0 ]
  [[ "$output" == *'invalid installation record'* ]]
  [[ "$output" != *'Traceback'* ]]
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
}

@test "missing private origin reports identity recovery and does not create a replacement" {
  git -C "$HOME/.local/share/dot-config" remote remove origin
  run "$HOME/.local/bin/dot" init --check
  [ "$status" -ne 0 ]
  [[ "$output" == *'Git identity detection failed'* ]]
  [[ "$output" != *'Traceback'* ]]
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
  [ -d "$HOME/.local/share/dot-config/.git" ]
}

@test "unrecorded default tracker blocks creation rather than switching away from existing data" {
  mkdir -p "$HOME/.local/share/chezmoi"
  printf 'machine="preserve"\n' > "$HOME/.local/share/chezmoi/.chezmoidata.toml"
  run "$HOME/.local/bin/dot" init --check
  [ "$status" -ne 0 ]
  [[ "$output" == *"$HOME/.local/share/chezmoi"* ]]
  [[ "$output" == *'existing unrecorded tracker'* ]]
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
  [ "$(cat "$HOME/.local/share/chezmoi/.chezmoidata.toml")" = 'machine="preserve"' ]
  [ ! -s "$TRACE" ]
}

@test "interrupting tracker consent cancels cleanly without creating machine data" {
  start_init
  wait_for 'Create this tracker'
  tmux -L "$SOCKET" send-keys -t ui C-c
  wait_for UI_EXIT=130
  [[ "$(pane)" == *'init canceled'* ]]
  [[ "$(pane)" != *'Traceback'* ]]
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
}
