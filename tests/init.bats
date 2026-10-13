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

@test "noninteractive tracker creation preserves exact bytes and installer state" {
  cp "$HOME/.local/state/dot/install.json" "$BATS_TEST_TMPDIR/install-before"
  run "$HOME/.local/bin/dot" init
  [ "$status" -eq 0 ]
  [[ "$output" == *'Finish; adoption scope unchanged'* ]]
  [ -f "$HOME/.config/chezmoi/chezmoi.toml" ]
  grep -F "sourceDir = \"$HOME/.local/share/dot-config/home\"" "$HOME/.config/chezmoi/chezmoi.toml"
  grep -F 'name = "Fixture User"' "$HOME/.config/chezmoi/chezmoi.toml"
  run grep -F 'profile' "$HOME/.config/chezmoi/chezmoi.toml"
  [ "$status" -ne 0 ]
  cp "$HOME/.config/chezmoi/chezmoi.toml" "$BATS_TEST_TMPDIR/created"
  cp "$HOME/.local/state/dot/install.json" "$BATS_TEST_TMPDIR/record"
  run "$HOME/.local/bin/dot" init --check
  [ "$status" -eq 0 ]
  [[ "$output" == *'Compatible tracker detected'* ]]
  cmp "$BATS_TEST_TMPDIR/created" "$HOME/.config/chezmoi/chezmoi.toml"
  cmp "$BATS_TEST_TMPDIR/record" "$HOME/.local/state/dot/install.json"
  cmp "$BATS_TEST_TMPDIR/install-before" "$HOME/.local/state/dot/install.json"
  [ "$(stat -f %Lp "$HOME/.local/state/dot/connection.json")" = 600 ]
  python3 - "$HOME/.local/state/dot/connection.json" "$HOME/.local/share/dot-config" "$HOME/.config/chezmoi/chezmoi.toml" <<'PY'
import json,sys
d=json.load(open(sys.argv[1]))
assert d == {'connection_version':1,'private_source':sys.argv[2],'config_path':sys.argv[3]}
PY
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
  [ ! -e "$HOME/.local/state/dot/chezmoi-state.boltdb" ]
  [ ! -e "$HOME/.config/eza/theme.yml" ]
}

@test "Finish default does not select files or invoke prebuilt browser or package install" {
  run "$HOME/.local/bin/dot" init
  [ "$status" -eq 0 ]
  [[ "$output" == *'Finish; adoption scope unchanged'* ]]
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
  [ ! -e "$HOME/.config/eza/theme.yml" ]
  ! grep -E '^brew (list|install)' "$TRACE"
  ! grep -F -- '--rounds' "$TRACE"
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

@test "explicit URL retrieval clones with plain git and tracker creation stays separate" {
  source="$HOME/.local/share/dot-config"
  config="$HOME/.config/chezmoi/chezmoi.toml"
  rm -rf "$source"
  run "$HOME/.local/bin/dot" init --url "$FIXTURE_REMOTE" --source "$source" --config "$config"
  [ "$status" -eq 0 ]
  [ -d "$source/.git" ]
  [[ "$(git -C "$source" remote get-url origin)" == *"$FIXTURE_REMOTE"* ]]
  [[ "$output" == *'Private checkout cloned'* ]]
  [ -f "$config" ]
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
  ! grep -E '^gh api|^gh repo clone' "$TRACE"
}

@test "init with no URL stops with guidance instead of assuming a repository" {
  source="$HOME/.local/share/dot-config"
  rm -rf "$source"
  run "$HOME/.local/bin/dot" init --source "$source" --config "$HOME/.config/chezmoi/chezmoi.toml"
  [ "$status" -eq 0 ]
  [[ "$output" == *'--url'* ]]
  [ ! -e "$source" ]
  ! grep -E '^gh api|^gh repo clone' "$TRACE"
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
}

@test "init rejects credential-bearing URLs without cloning" {
  source="$HOME/.local/share/dot-config"
  rm -rf "$source"
  run "$HOME/.local/bin/dot" init --url 'https://user:secret@example.invalid/owner/repo.git' --source "$source" --config "$HOME/.config/chezmoi/chezmoi.toml"
  [ "$status" -ne 0 ]
  [[ "$output" == *'credential'* ]]
  [ ! -e "$source" ]
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
}

@test "missing dependency stops with guidance and performs no install" {
  export CHEZMOI_MISSING=yes
  run "$HOME/.local/bin/dot" init
  [ "$status" -eq 0 ]
  [[ "$output" == *'Missing/incompatible dependencies: chezmoi'* ]]
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
  [ ! -e "$HOME/bottle-chezmoi" ]
  ! grep -E '^brew install --force-bottle ' "$TRACE"
}

@test "removed profile flag is rejected and no tracker data is written" {
  run "$HOME/.local/bin/dot" init --profile personal
  [ "$status" -ne 0 ]
  run "$HOME/.local/bin/dot" init --profile work
  [ "$status" -ne 0 ]
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
}

@test "optional rounds invoke installed prebuilt browser with explicit identity and no Go build" {
  mkdir -p "$HOME/.local/share/dot-cli/bin"
  printf '#!/bin/bash\nprintf "browser %%s\\n" "$*" >> "$TRACE"\n' > "$HOME/.local/share/dot-cli/bin/dot-browser"
  chmod +x "$HOME/.local/share/dot-cli/bin/dot-browser"
  tmux -L "$SOCKET" new-session -d -s ui -x 180 -y 60 "$HOME/.local/bin/dot init --rounds; echo UI_EXIT=\$?; sleep 30"
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

@test "alternate tracker formats and dangling config symlinks block creation without deleting data" {
  mkdir -p "$HOME/.config/chezmoi"
  printf '{"sourceDir":"/other"}\n' > "$HOME/.config/chezmoi/chezmoi.json"
  run "$HOME/.local/bin/dot" init --check
  [ "$status" -ne 0 ]
  [[ "$output" == *'alternate tracker'* ]]
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
  [ -f "$HOME/.config/chezmoi/chezmoi.json" ]
}

@test "render failure diagnoses actual tracker config and home source without deleting machine data" {
  run "$HOME/.local/bin/dot" init
  [ "$status" -eq 0 ]
  printf 'preserve database\n' > "$HOME/.local/state/dot/chezmoi-state.boltdb"
  run "$HOME/.local/bin/dot" status
  [ "$status" -ne 0 ]
  [[ "$output" == *"--source \"$HOME/.local/share/dot-config/home\""* ]]
  [[ "$output" == *"--config \"$HOME/.config/chezmoi/chezmoi.toml\""* ]]
  [[ "$output" == *'do not delete machine data'* ]]
  [ "$(cat "$HOME/.local/state/dot/chezmoi-state.boltdb")" = 'preserve database' ]
}

@test "init leaves installer-owned artifact state independent from public checkout remotes" {
  cp "$HOME/.local/state/dot/install.json" "$BATS_TEST_TMPDIR/install-before"
  git -C "$HOME/.local/share/dot-cli" remote set-url origin /unrelated-public
  run "$HOME/.local/bin/dot" init --check
  [ "$status" -eq 0 ]
  [[ "$output" == *'Tracker pending'* ]]
  cmp "$BATS_TEST_TMPDIR/install-before" "$HOME/.local/state/dot/install.json"
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

@test "missing machine identity stops with guidance without changing Git configuration" {
  git config --global --unset user.name
  git config --global --unset user.email
  run "$HOME/.local/bin/dot" init
  [ "$status" -eq 0 ]
  [[ "$output" == *'--name'* ]]
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
  run git config --global user.name
  [ "$status" -ne 0 ]
  run git config --global user.email
  [ "$status" -ne 0 ]
  run "$HOME/.local/bin/dot" init --name 'Local Fixture' --email 'local@example.invalid'
  [ "$status" -eq 0 ]
  grep -F 'name = "Local Fixture"' "$HOME/.config/chezmoi/chezmoi.toml"
  grep -F 'email = "local@example.invalid"' "$HOME/.config/chezmoi/chezmoi.toml"
}

@test "malformed installation metadata fails with actionable diagnostics instead of a traceback" {
  printf '[]\n' > "$HOME/.local/state/dot/connection.json"
  run "$HOME/.local/bin/dot" init --check
  [ "$status" -ne 0 ]
  [[ "$output" == *'connection.json'* ]]
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
