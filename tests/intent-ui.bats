#!/usr/bin/env bats
setup() {
  command -v tmux >/dev/null || skip 'tmux required'
  REPO="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
  export HOME="$BATS_TEST_TMPDIR/home" DOT_SOURCE="$BATS_TEST_TMPDIR/source" TERM=xterm-256color
  export DOT_ENGINE="$REPO/tools/dot" DOT_CONFIG="$HOME/.config/chezmoi/chezmoi.toml"
  mkdir -p "$HOME/.config/arbitrary" "$HOME/.config/chezmoi"
  python3 "$BATS_TEST_DIRNAME/helpers/private-config.py" --source "$DOT_SOURCE" \
    --fixtures "$BATS_TEST_DIRNAME/fixtures" --empty
  cp "$BATS_TEST_DIRNAME/fixtures/chezmoiignore.tmpl" "$DOT_SOURCE/home/.chezmoiignore"
  cp "$BATS_TEST_DIRNAME/fixtures/chezmoiremove.tmpl" "$DOT_SOURCE/home/.chezmoiremove"
  cp "$BATS_TEST_DIRNAME/fixtures/run_after_sync-skills.sh.tmpl" "$DOT_SOURCE/home/run_after_sync-skills.sh.tmpl"
  printf '[data]\nname="Fixture"\nemail="fixture@example.org"\nprofile="test"\ndotEngine="%s"\n' "$DOT_ENGINE" > "$DOT_CONFIG"
  cp "$REPO/tools/dot/"*.py "$DOT_SOURCE/tools/dot/"
  git init -q "$DOT_SOURCE"
  git -C "$DOT_SOURCE" config user.name Test
  git -C "$DOT_SOURCE" config user.email test@example.org
  git -C "$DOT_SOURCE" add .
  git -C "$DOT_SOURCE" commit -qm fixture
  printf 'theme = light\n' > "$HOME/.config/arbitrary/preferences.conf"
  DOT="$REPO/tools/dot/dot"
  SOCKET="intent-ui-$$-$BATS_TEST_NUMBER"
}
teardown() { tmux -L "$SOCKET" kill-server 2>/dev/null || true; }
mixed_fixture() {
  mkdir -p "$DOT_SOURCE/home/dot_config"
  printf 'safe preference\n' > "$DOT_SOURCE/home/dot_config/preferences.conf"
  printf 'alias gt="lazygit"\n' > "$DOT_SOURCE/home/dot_bashrc"
  cat > "$DOT_SOURCE/tools/dot/manifest.json" <<'JSON'
{"version":1,"entries":{"safe":{"destination":".config/preferences.conf","source":"home/dot_config/preferences.conf","os":["darwin","linux"],"portability":"managed-shared","impact":"Safe","removals":[],"hooks":[],"tools":[]},"unsafe":{"destination":".bashrc","source":"home/dot_bashrc","os":["darwin","linux"],"portability":"managed-shared","impact":"Unsafe","removals":[],"hooks":[],"tools":["fixturemissingtool"],"warnings":["desktop-arch-compatibility"]}},"profiles":{"mixed":["safe","unsafe"]}}
JSON
  git -C "$DOT_SOURCE" add .
  git -C "$DOT_SOURCE" commit -qm mixed
}
pick_mixed() {
  mixed_fixture
  start_picker
  tmux -L "$SOCKET" send-keys -t ui j Enter
  wait_for 'Named repository profile'
  tmux -L "$SOCKET" send-keys -t ui mixed Enter
  wait_for 'Nothing selected'
  [[ "$(pane)" == *'[ ] safe'* ]]
  [[ "$(pane)" == *'[ ] unsafe'* ]]
  tmux -L "$SOCKET" send-keys -t ui Space j Space p
  wait_for 'Review exact plan'
  [[ "$(pane)" == *'Ubuntu compatibility is not promised'* ]]
  tmux -L "$SOCKET" send-keys -t ui a
  wait_for 'Apply the reviewed'
  tmux -L "$SOCKET" send-keys -t ui j Enter
  wait_for 'Type UNSAFE'
}
pane() { tmux -L "$SOCKET" capture-pane -p -t ui; }
wait_for() {
  local i
  for i in {1..100}; do
    [[ "$(pane)" == *"$1"* ]] && return 0
    sleep 0.1
  done
  pane >&2
  return 1
}
start_picker() {
  local command
  printf -v command 'bash %q intent pick; code=$?; echo UI_EXIT=$code; echo UI_DONE; sleep 30' "$DOT"
  tmux -L "$SOCKET" new-session -d -s ui -x 120 -y 40 "$command"
  wait_for 'Track / Adopt'
}
track_file() {
  start_picker
  tmux -L "$SOCKET" send-keys -t ui Enter
  wait_for 'Named repository profile'
  tmux -L "$SOCKET" send-keys -t ui work Enter
  wait_for 'Nothing selected'
  tmux -L "$SOCKET" send-keys -t ui l
  wait_for 'HOME/.config'
  tmux -L "$SOCKET" send-keys -t ui l
  wait_for 'HOME/.config/arbitrary'
  tmux -L "$SOCKET" send-keys -t ui Space
  wait_for 'Destination relative'
  tmux -L "$SOCKET" send-keys -t ui Enter
  wait_for 'Portability'
  tmux -L "$SOCKET" send-keys -t ui Enter
  wait_for 'Supported operating'
  tmux -L "$SOCKET" send-keys -t ui Enter
  wait_for 'Operational impact'
  tmux -L "$SOCKET" send-keys -t ui Enter
  wait_for '1 selections'
  tmux -L "$SOCKET" send-keys -t ui p
  wait_for 'Review exact plan'
}

@test "vim picker previews the identical CLI plan and commits only after confirmation" {
  printf '{"version":1,"mode":"track","profile":"work","selections":[{"path":".config/arbitrary/preferences.conf","os":["darwin","linux"],"portability":"managed-shared","impact":"Safe"}]}\n' > "$BATS_TEST_TMPDIR/request"
  "$DOT" intent plan "$BATS_TEST_TMPDIR/request" --out "$BATS_TEST_TMPDIR/plan"
  TOKEN="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["id"])' "$BATS_TEST_TMPDIR/plan")"
  track_file
  [[ "$(pane)" == *"$TOKEN"* ]]
  [ ! -e "$DOT_SOURCE/home/dot_config/arbitrary/preferences.conf" ]
  tmux -L "$SOCKET" send-keys -t ui a
  wait_for 'Apply the reviewed'
  tmux -L "$SOCKET" send-keys -t ui j Enter
  wait_for UI_DONE
  [[ "$(pane)" == *'UI_EXIT=0'* ]]
  [ "$(git -C "$DOT_SOURCE" show HEAD:home/dot_config/arbitrary/preferences.conf)" = 'theme = light' ]
}

@test "mixed Safe Unsafe picker cancellation at additional acknowledgment leaves scope and files untouched" {
  pick_mixed
  tmux -L "$SOCKET" send-keys -t ui Escape
  wait_for 'Unsafe acknowledgment canceled'
  tmux -L "$SOCKET" send-keys -t ui q
  wait_for UI_DONE
  [ ! -e "$HOME/.bashrc" ]
  [ ! -e "$HOME/.config/preferences.conf" ]
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
}

@test "mixed Safe Unsafe picker requires distinct Unsafe and desktop confirmations before applying" {
  pick_mixed
  tmux -L "$SOCKET" send-keys -t ui UNSAFE Enter
  wait_for 'type DESKTOP'
  [ ! -e "$HOME/.bashrc" ]
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
  tmux -L "$SOCKET" send-keys -t ui DESKTOP Enter
  wait_for UI_DONE
  [[ "$(pane)" == *'UI_EXIT=0'* ]]
  [ "$(cat "$HOME/.bashrc")" = 'alias gt="lazygit"' ]
  [ "$(cat "$HOME/.config/preferences.conf")" = 'safe preference' ]
  grep -q fixturemissingtool "$HOME/.local/state/dot/missing-tools.json"
}

@test "picker Unsafe-to-Safe metadata downgrade asks for original Unsafe and removed desktop warning consent" {
  mixed_fixture
  start_picker
  tmux -L "$SOCKET" send-keys -t ui j Enter
  wait_for 'Named repository profile'
  tmux -L "$SOCKET" send-keys -t ui mixed Enter
  wait_for 'Nothing selected'
  tmux -L "$SOCKET" send-keys -t ui j e
  wait_for 'Supported operating systems'
  tmux -L "$SOCKET" send-keys -t ui Enter
  wait_for 'Operational impact'
  tmux -L "$SOCKET" send-keys -t ui Enter
  wait_for 'Exact file removals'
  tmux -L "$SOCKET" send-keys -t ui Enter
  wait_for 'Advisory tools'
  tmux -L "$SOCKET" send-keys -t ui Enter
  wait_for 'Compatibility warning'
  tmux -L "$SOCKET" send-keys -t ui Enter p
  wait_for 'Review exact plan'
  tmux -L "$SOCKET" send-keys -t ui a
  wait_for 'Apply the reviewed'
  tmux -L "$SOCKET" send-keys -t ui j Enter
  wait_for 'Type UNSAFE'
  [ ! -e "$HOME/.bashrc" ]
  tmux -L "$SOCKET" send-keys -t ui UNSAFE Enter
  wait_for 'type DESKTOP'
  tmux -L "$SOCKET" send-keys -t ui DESKTOP Enter
  wait_for UI_DONE
  [[ "$(pane)" == *'UI_EXIT=0'* ]]
  [ ! -e "$HOME/.config/preferences.conf" ]
  python3 - "$HOME/.local/state/dot/adoption.json" <<'PY'
import json, sys
e = json.load(open(sys.argv[1]))['entries']['unsafe']
assert e['impact'] == 'Safe' and e['warnings'] == []
PY
}

@test "Track picker offers bounded explicit Git keys without importing credentials or authorizing adoption" {
  export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
  mkdir -p "$DOT_SOURCE/home/.chezmoitemplates"
  cp "$BATS_TEST_DIRNAME/fixtures/modify_dot_gitconfig.tmpl" "$DOT_SOURCE/home/"
  cp "$BATS_TEST_DIRNAME/fixtures/.chezmoitemplates/gitconfig.tmpl" "$DOT_SOURCE/home/.chezmoitemplates/"
  python3 - "$DOT_SOURCE/tools/dot/manifest.json" <<'PY'
import json, sys
e={"destination":".gitconfig","git_keys":["user.name","user.email"],"git_source":"home/.chezmoitemplates/gitconfig.tmpl","hooks":[],"impact":"Unsafe","integrations":["git-config"],"notices":["config-only"],"os":["darwin","linux"],"portability":"managed-templated","removals":[],"source":"home/modify_dot_gitconfig.tmpl","tools":["git"]}
json.dump({'version': 1, 'entries': {'.gitconfig': e}, 'profiles': {}}, open(sys.argv[1], 'w'))
PY
  git -C "$DOT_SOURCE" add .
  git -C "$DOT_SOURCE" commit -qm git-preset
  printf '[user]\nname = Picker Import\nemail = keep@example.org\n[credential]\npassword = private-fixture-value\n' > "$HOME/.gitconfig"
  cp "$HOME/.gitconfig" "$BATS_TEST_TMPDIR/git-before"
  start_picker
  tmux -L "$SOCKET" send-keys -t ui Enter
  wait_for 'Named repository profile'
  tmux -L "$SOCKET" send-keys -t ui key-track Enter
  wait_for 'Nothing selected'
  tmux -L "$SOCKET" send-keys -t ui j Space
  wait_for 'Track only explicitly selected Git keys'
  tmux -L "$SOCKET" send-keys -t ui user.name Enter p
  wait_for 'Review exact plan'
  [[ "$(pane)" != *'private-fixture-value'* ]]
  tmux -L "$SOCKET" send-keys -t ui a
  wait_for 'Apply the reviewed'
  tmux -L "$SOCKET" send-keys -t ui j Enter
  wait_for 'Type UNSAFE'
  tmux -L "$SOCKET" send-keys -t ui UNSAFE Enter
  wait_for UI_DONE
  [[ "$(pane)" == *'UI_EXIT=0'* ]]
  cmp "$HOME/.gitconfig" "$BATS_TEST_TMPDIR/git-before"
  grep -q 'Picker Import' "$DOT_SOURCE/home/.chezmoitemplates/gitconfig.tmpl"
  ! grep -q private-fixture-value "$DOT_SOURCE/home/.chezmoitemplates/gitconfig.tmpl"
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
}

@test "picker cancellation after metadata and preview leaves source and target untouched" {
  track_file
  tmux -L "$SOCKET" send-keys -t ui q
  wait_for 'Preview canceled'
  tmux -L "$SOCKET" send-keys -t ui q
  wait_for UI_DONE
  [[ "$(pane)" == *'Canceled; nothing applied'* ]]
  [ ! -e "$DOT_SOURCE/home/dot_config" ]
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
  [ "$(cat "$HOME/.config/arbitrary/preferences.conf")" = 'theme = light' ]
}

@test "Adopt picker direction and exact selection produce the equivalent CLI plan" {
  printf '{"version":1,"mode":"track","profile":"work","selections":[{"path":".config/arbitrary/preferences.conf","os":["darwin","linux"],"portability":"managed-shared","impact":"Safe"}]}\n' > "$BATS_TEST_TMPDIR/request"
  "$DOT" intent plan "$BATS_TEST_TMPDIR/request" --out "$BATS_TEST_TMPDIR/track-plan" >/dev/null
  TOKEN="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["id"])' "$BATS_TEST_TMPDIR/track-plan")"
  "$DOT" intent apply "$BATS_TEST_TMPDIR/track-plan" --approve "$TOKEN" >/dev/null
  printf 'local\n' > "$HOME/.config/arbitrary/preferences.conf"
  printf '{"version":1,"mode":"adopt","profile":"work","entries":[".config/arbitrary/preferences.conf"]}\n' > "$BATS_TEST_TMPDIR/request"
  "$DOT" intent plan "$BATS_TEST_TMPDIR/request" --out "$BATS_TEST_TMPDIR/adopt-plan" >/dev/null
  TOKEN="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["id"])' "$BATS_TEST_TMPDIR/adopt-plan")"
  start_picker
  tmux -L "$SOCKET" send-keys -t ui j Enter
  wait_for 'Named repository profile'
  tmux -L "$SOCKET" send-keys -t ui work Enter
  wait_for 'Repository manifest entries'
  tmux -L "$SOCKET" send-keys -t ui Space p
  wait_for 'Review exact plan'
  [[ "$(pane)" == *"$TOKEN"* ]]
  [[ "$(pane)" == *'Adopt: repository'* ]]
  tmux -L "$SOCKET" send-keys -t ui a
  wait_for 'Apply the reviewed'
  tmux -L "$SOCKET" send-keys -t ui j Enter
  wait_for UI_DONE
  [ "$(cat "$HOME/.config/arbitrary/preferences.conf")" = 'theme = light' ]
}

@test "Adopt metadata editor previews CLI-equivalent removals tools and compatibility warning" {
  printf '{"version":1,"mode":"track","profile":"work","selections":[{"path":".config/arbitrary/preferences.conf","os":["darwin","linux"],"portability":"managed-shared","impact":"Safe"}]}\n' > "$BATS_TEST_TMPDIR/request"
  "$DOT" intent plan "$BATS_TEST_TMPDIR/request" --out "$BATS_TEST_TMPDIR/track-plan" >/dev/null
  TOKEN="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["id"])' "$BATS_TEST_TMPDIR/track-plan")"
  "$DOT" intent apply "$BATS_TEST_TMPDIR/track-plan" --approve "$TOKEN" >/dev/null
  printf 'obsolete\n' > "$HOME/obsolete.conf"
  printf '{"version":1,"mode":"adopt","profile":"work","entries":[".config/arbitrary/preferences.conf"],"metadata":{".config/arbitrary/preferences.conf":{"os":["darwin","linux"],"impact":"Safe","removals":["obsolete.conf"],"tools":["fixturemissingtool"],"warnings":["desktop-arch-compatibility"]}}}\n' > "$BATS_TEST_TMPDIR/request"
  "$DOT" intent plan "$BATS_TEST_TMPDIR/request" --out "$BATS_TEST_TMPDIR/expected" >/dev/null
  TOKEN="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["id"])' "$BATS_TEST_TMPDIR/expected")"
  start_picker
  tmux -L "$SOCKET" send-keys -t ui j Enter
  wait_for 'Named repository profile'
  tmux -L "$SOCKET" send-keys -t ui work Enter
  wait_for 'Repository manifest entries'
  tmux -L "$SOCKET" send-keys -t ui e
  wait_for 'Supported operating systems'
  tmux -L "$SOCKET" send-keys -t ui Enter
  wait_for 'Operational impact'
  tmux -L "$SOCKET" send-keys -t ui Enter
  wait_for 'Exact file removals'
  tmux -L "$SOCKET" send-keys -t ui obsolete.conf Enter
  wait_for 'Advisory tools'
  tmux -L "$SOCKET" send-keys -t ui fixturemissingtool Enter
  wait_for 'Compatibility warning'
  tmux -L "$SOCKET" send-keys -t ui j Enter
  wait_for '1 selections'
  tmux -L "$SOCKET" send-keys -t ui p
  wait_for 'Review exact plan'
  [[ "$(pane)" == *"$TOKEN"* ]]
  tmux -L "$SOCKET" send-keys -t ui a
  wait_for 'Apply the reviewed'
  tmux -L "$SOCKET" send-keys -t ui j Enter
  wait_for 'type DESKTOP'
  [ -e "$HOME/obsolete.conf" ]
  tmux -L "$SOCKET" send-keys -t ui DESKTOP Enter
  wait_for UI_DONE
  [[ "$(pane)" == *'UI_EXIT=0'* ]]
  [ ! -e "$HOME/obsolete.conf" ]
  grep -q fixturemissingtool "$HOME/.local/state/dot/missing-tools.json"
}

@test "Track capabilities editor has CLI-equivalent exact removals and advisory tools" {
  printf '{"version":1,"mode":"track","profile":"work","selections":[{"path":".config/arbitrary/preferences.conf","os":["darwin","linux"],"portability":"managed-shared","impact":"Safe","removals":["obsolete.conf"],"tools":["fixturemissingtool"],"warnings":[]}]}\n' > "$BATS_TEST_TMPDIR/request"
  "$DOT" intent plan "$BATS_TEST_TMPDIR/request" --out "$BATS_TEST_TMPDIR/expected" >/dev/null
  TOKEN="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["id"])' "$BATS_TEST_TMPDIR/expected")"
  track_file
  tmux -L "$SOCKET" send-keys -t ui q
  wait_for 'Preview canceled'
  tmux -L "$SOCKET" send-keys -t ui c
  wait_for 'Exact file removals'
  tmux -L "$SOCKET" send-keys -t ui obsolete.conf Enter
  wait_for 'Advisory tools'
  tmux -L "$SOCKET" send-keys -t ui fixturemissingtool Enter
  wait_for 'Compatibility warning'
  tmux -L "$SOCKET" send-keys -t ui Enter
  wait_for '1 selections'
  tmux -L "$SOCKET" send-keys -t ui p
  wait_for 'Review exact plan'
  [[ "$(pane)" == *"$TOKEN"* ]]
  tmux -L "$SOCKET" send-keys -t ui a
  wait_for 'Apply the reviewed'
  tmux -L "$SOCKET" send-keys -t ui j Enter
  wait_for UI_DONE
  [[ "$(pane)" == *'UI_EXIT=0'* ]]
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
}

@test "optional rounds default Finish and opt-in preset still needs exact preview approval with persistent results" {
  printf '{"version":1,"mode":"track","profile":"work","selections":[{"path":".config/arbitrary/preferences.conf","os":["darwin","linux"],"portability":"managed-shared","impact":"Safe","tools":["fixturemissingtool"]}]}\n' > "$BATS_TEST_TMPDIR/request"
  "$DOT" intent plan "$BATS_TEST_TMPDIR/request" --out "$BATS_TEST_TMPDIR/plan" >/dev/null
  TOKEN="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["id"])' "$BATS_TEST_TMPDIR/plan")"
  "$DOT" intent apply "$BATS_TEST_TMPDIR/plan" --approve "$TOKEN" >/dev/null
  printf 'local\n' > "$HOME/.config/arbitrary/preferences.conf"
  printf -v command 'bash %q intent rounds; code=$?; echo UI_EXIT=$code; echo UI_DONE; sleep 30' "$DOT"
  tmux -L "$SOCKET" new-session -d -s ui -x 120 -y 40 "$command"
  wait_for 'Optional configuration round'
  [[ "$(pane)" == *'> Finish without further adoption'* ]]
  tmux -L "$SOCKET" send-keys -t ui j j Enter
  wait_for 'Named repository profile'
  tmux -L "$SOCKET" send-keys -t ui work Enter
  wait_for 'Nothing selected'
  [ "$(cat "$HOME/.config/arbitrary/preferences.conf")" = local ]
  tmux -L "$SOCKET" send-keys -t ui s
  wait_for 'Opt-in exact repository preset'
  tmux -L "$SOCKET" send-keys -t ui j Enter
  wait_for 'Explicit preset snapshot: 1'
  tmux -L "$SOCKET" send-keys -t ui p
  wait_for 'Review exact plan'
  [ "$(cat "$HOME/.config/arbitrary/preferences.conf")" = local ]
  tmux -L "$SOCKET" send-keys -t ui a
  wait_for 'Apply the reviewed'
  tmux -L "$SOCKET" send-keys -t ui j Enter
  wait_for 'Optional configuration round'
  tmux -L "$SOCKET" send-keys -t ui Enter
  wait_for UI_DONE
  [[ "$(pane)" == *'UI_EXIT=0'* ]]
  [[ "$(pane)" == *'Missing tools (advisory): fixturemissingtool'* ]]
  [[ "$(pane)" == *'Applied approved plan'* ]]
  [ "$(cat "$HOME/.config/arbitrary/preferences.conf")" = 'theme = light' ]
}

@test "skills picker singleton backup conflict requires separate typed consent and declining applies nothing" {
  mkdir -p "$HOME/.agents/skills/example" "$HOME/.agents/skills.bak/example"
  printf 'old skill\n' > "$HOME/.agents/skills/example/SKILL.md"
  printf 'old backup\n' > "$HOME/.agents/skills.bak/example/SKILL.md"
  printf '{"version":1,"mode":"track","profile":"skills","selections":[{"path":".agents/skills/example/SKILL.md","os":["darwin","linux"],"portability":"managed-shared","impact":"Safe","integrations":["gemini-skills"]}]}\n' > "$BATS_TEST_TMPDIR/request"
  "$DOT" intent plan "$BATS_TEST_TMPDIR/request" --out "$BATS_TEST_TMPDIR/plan" >/dev/null
  TOKEN="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["id"])' "$BATS_TEST_TMPDIR/plan")"
  "$DOT" intent apply "$BATS_TEST_TMPDIR/plan" --approve "$TOKEN" >/dev/null
  printf 'updated skill\n' > "$DOT_SOURCE/home/dot_agents/skills/example/SKILL.md"
  git -C "$DOT_SOURCE" add .
  git -C "$DOT_SOURCE" commit -qm update
  start_picker
  tmux -L "$SOCKET" send-keys -t ui j Enter
  wait_for 'Named repository profile'
  tmux -L "$SOCKET" send-keys -t ui skills Enter
  wait_for 'Nothing selected'
  tmux -L "$SOCKET" send-keys -t ui Space p
  wait_for 'Review exact plan'
  tmux -L "$SOCKET" send-keys -t ui a
  wait_for 'Apply the reviewed'
  tmux -L "$SOCKET" send-keys -t ui j Enter
  wait_for 'Singleton backup conflict: type REPLACE'
  tmux -L "$SOCKET" send-keys -t ui Enter
  wait_for UI_DONE
  [[ "$(pane)" == *'Backup conflict consent canceled'* ]]
  [ "$(cat "$HOME/.agents/skills/example/SKILL.md")" = 'old skill' ]
  [ "$(cat "$HOME/.agents/skills.bak/example/SKILL.md")" = 'old backup' ]
  [ ! -e "$HOME/.gemini/skills/example/SKILL.md" ]
}
