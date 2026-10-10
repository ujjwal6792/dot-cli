#!/usr/bin/env bats
setup() {
  command -v tmux >/dev/null || skip 'tmux required'
  REPO="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
  export GOPATH="${DOT_BROWSER_TEST_GOPATH:-$BATS_SUITE_TMPDIR/go}"
  export GOCACHE="$GOPATH/build-cache" GOFLAGS=-modcacherw
  export HOME="$BATS_TEST_TMPDIR/home" DOT_SOURCE="$BATS_TEST_TMPDIR/private" DOT_ENGINE="$BATS_TEST_TMPDIR/public/tools/dot" TERM=xterm-256color NO_COLOR=1
  mkdir -p "$HOME" "$DOT_SOURCE/home" "$DOT_SOURCE/tools/dot" "$DOT_ENGINE"
  cp "$REPO/tools/dot/"*.py "$DOT_ENGINE/"
  cp "$REPO/tools/dot/engine.json" "$REPO/tools/dot/dot" "$DOT_ENGINE/"
  printf '{"protocol":1,"engine_api":{"min":1,"max":1},"manifest_version":1,"approval_version":2,"manifest":"tools/dot/manifest.json","home_source":"home","required_adapters":["intent","scope","gitconfig","skills","layout"]}\n' > "$DOT_SOURCE/dot-contract.json"
  export DOT_CONFIG="$BATS_TEST_TMPDIR/tracker.toml"
  printf 'sourceDir="%s/home"\n[data]\ndotEngine="%s"\n' "$DOT_SOURCE" "$DOT_ENGINE" > "$DOT_CONFIG"
  printf '{"version":1,"entries":{},"profiles":{}}\n' > "$DOT_SOURCE/tools/dot/manifest.json"
  git init -q "$DOT_SOURCE"
  git -C "$DOT_SOURCE" config user.name Test
  git -C "$DOT_SOURCE" config user.email fixture@example.org
  git -C "$DOT_SOURCE" add .
  git -C "$DOT_SOURCE" commit -qm fixture
  export DOT_BROWSER="${DOT_BROWSER_TEST_BINARY:-$BATS_TEST_TMPDIR/dot-browser}"
  if [ -z "${DOT_BROWSER_TEST_BINARY:-}" ]; then
    mkdir -p "$(dirname "$DOT_BROWSER")"
    (cd "$REPO/tools/dot/browser" && HOME="$BATS_TEST_TMPDIR/build-home" go build -mod=readonly -o "$DOT_BROWSER" ./cmd/dot-browser)
  fi
  SOCKET="browser-approval-$$-$BATS_TEST_NUMBER"
  export TRACE="$BATS_TEST_TMPDIR/package-service-trace"
  mkdir -p "$BATS_TEST_TMPDIR/bin"
  for tool in brew pacman yay apt apt-get sudo systemctl launchctl; do
    printf '#!/bin/sh\nprintf "%%s\\n" "%s $*" >> "$TRACE"\nexit 97\n' "$tool" > "$BATS_TEST_TMPDIR/bin/$tool"
    chmod +x "$BATS_TEST_TMPDIR/bin/$tool"
  done
  export PATH="$BATS_TEST_TMPDIR/bin:$PATH"
}
teardown() {
  tmux -L "$SOCKET" kill-server 2>/dev/null || true
  [ ! -s "$TRACE" ]
}
pane() { tmux -L "$SOCKET" capture-pane -p -t approval; }
wait_for() {
  local i
  for i in {1..120}; do
    [[ "$(pane)" == *"$1"* ]] && return 0
    sleep 0.1
  done
  pane >&2
  return 1
}
start_browser() {
  local command
  printf -v command 'bash %q browse --mode %s %s; code=$?; echo UI_EXIT=$code; echo UI_DONE; sleep 30' "$DOT_ENGINE/dot" "${BROWSER_MODE:-adopt}" "$*"
  tmux -L "$SOCKET" new-session -d -s approval -x 120 -y 40 "bash -c $(printf %q "$command")"
}
@test "Track explicitly reviewed template commits source without adopting target" {
  printf 'live fixture\n' > "$HOME/preferences.conf"
  BROWSER_MODE=track
  start_browser
  wait_for preferences.conf
  wait_for 'live fixture'
  [[ "$(pane)" == *'REPOSITORY'* ]]
  tmux -L "$SOCKET" send-keys -t approval Space
  wait_for 'METADATA'
  tmux -L "$SOCKET" send-keys -t approval Tab C-u
  tmux -L "$SOCKET" send-keys -l -t approval 'darwin,linux'
  tmux -L "$SOCKET" send-keys -t approval Tab C-u
  tmux -L "$SOCKET" send-keys -l -t approval 'managed-templated'
  tmux -L "$SOCKET" send-keys -t approval Tab C-u
  tmux -L "$SOCKET" send-keys -l -t approval 'Safe'
  tmux -L "$SOCKET" send-keys -t approval Tab C-u
  tmux -L "$SOCKET" send-keys -l -t approval 'quoted reviewed template'
  tmux -L "$SOCKET" send-keys -t approval Enter
  wait_for '1 selected'
  tmux -L "$SOCKET" send-keys -t approval p
  wait_for 'REVIEW EXACT PLAN'
  tmux -L "$SOCKET" send-keys -t approval a
  wait_for 'FINAL CONFIRMATION'
  tmux -L "$SOCKET" send-keys -t approval j Enter
  wait_for Applied
  python3 - "$DOT_SOURCE" <<'PY'
import json, pathlib, sys
root=pathlib.Path(sys.argv[1]); entry=json.loads((root/'tools/dot/manifest.json').read_text())['entries']['preferences.conf']
assert entry['portability']=='managed-templated', entry
assert entry['os']==['darwin','linux'], entry
assert (root/entry['source']).read_text()=='quoted reviewed template\n'
PY
  [ "$(cat "$HOME/preferences.conf")" = 'live fixture' ]
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
  [ -z "$(git -C "$DOT_SOURCE" status --porcelain)" ]
}
adopt_fixture() {
  mkdir -p "$DOT_SOURCE/home/dot_config" "$HOME/.config"
  printf 'repository fixture\n' > "$DOT_SOURCE/home/dot_config/preferences.conf"
  printf 'target fixture\n' > "$HOME/.config/preferences.conf"
  cat > "$DOT_SOURCE/tools/dot/manifest.json" <<'JSON'
{"version":1,"entries":{"prefs":{"destination":".config/preferences.conf","source":"home/dot_config/preferences.conf","os":["darwin","linux"],"portability":"managed-shared","impact":"Safe","removals":[],"hooks":[],"tools":[]}},"profiles":{"work":["prefs"]}}
JSON
  git -C "$DOT_SOURCE" add .
  git -C "$DOT_SOURCE" commit -qm preferences
}
enter_review() {
  wait_for ADOPT
  tmux -L "$SOCKET" send-keys -t approval H
  wait_for '.config'
  tmux -L "$SOCKET" send-keys -t approval l
  wait_for preferences.conf
  tmux -L "$SOCKET" send-keys -t approval j Space
  wait_for '1 selected'
  tmux -L "$SOCKET" send-keys -t approval p
  wait_for 'REVIEW EXACT PLAN'
}
cancel_review() {
  tmux -L "$SOCKET" send-keys -t approval q
  wait_for canceled
  tmux -L "$SOCKET" send-keys -t approval q
}
@test "approved Adopt requires cancel-default confirmation then applies only exact scope" {
  adopt_fixture
  start_browser
  enter_review
  [[ "$(pane)" == *'REPOSITORY → TARGET'* ]]
  [[ "$(pane)" == *'.config/preferences.conf'* ]]
  [[ "$(pane)" == *'Plan ID:'* ]]
  tmux -L "$SOCKET" send-keys -t approval a
  wait_for 'FINAL CONFIRMATION'
  [[ "$(pane)" == *'> Cancel'* ]]
  [[ "$(pane)" == *'Apply exact reviewed plan'* ]]
  [ "$(cat "$HOME/.config/preferences.conf")" = 'target fixture' ]
  tmux -L "$SOCKET" send-keys -t approval Enter
  wait_for 'REVIEW EXACT PLAN'
  [ "$(cat "$HOME/.config/preferences.conf")" = 'target fixture' ]
  tmux -L "$SOCKET" send-keys -t approval a
  wait_for 'FINAL CONFIRMATION'
  tmux -L "$SOCKET" send-keys -t approval j Enter
  wait_for 'Applied'
  [ "$(cat "$HOME/.config/preferences.conf")" = 'repository fixture' ]
  python3 - "$HOME/.local/state/dot/adoption.json" <<'PY'
import json, sys
state=json.load(open(sys.argv[1]))
assert set(state['entries']) == {'prefs'}, state
PY
}
@test "canceling final confirmation and review leaves target, approval and source unchanged" {
  adopt_fixture
  before="$(git -C "$DOT_SOURCE" rev-parse HEAD)"
  start_browser
  enter_review
  tmux -L "$SOCKET" send-keys -t approval a
  wait_for 'FINAL CONFIRMATION'
  tmux -L "$SOCKET" send-keys -t approval Enter
  sleep 0.2
  wait_for 'REVIEW EXACT PLAN'
  cancel_review
  wait_for UI_DONE
  [ "$(cat "$HOME/.config/preferences.conf")" = 'target fixture' ]
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
  [ "$(git -C "$DOT_SOURCE" rev-parse HEAD)" = "$before" ]
  [ -z "$(git -C "$DOT_SOURCE" status --porcelain)" ]
}
@test "stale Adopt plan is rejected at apply without changing target or approval" {
  adopt_fixture
  start_browser
  enter_review
  # Change target after review but before the explicit final confirmation.
  printf 'new local target fixture\n' > "$HOME/.config/preferences.conf"
  tmux -L "$SOCKET" send-keys -t approval a
  wait_for 'FINAL CONFIRMATION'
  tmux -L "$SOCKET" send-keys -t approval j Enter
  wait_for 'stale/changed plan'
  [[ "$(pane)" != *'Applied reviewed plan.'* ]]
  [ "$(cat "$HOME/.config/preferences.conf")" = 'new local target fixture' ]
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
}
@test "unsafe and desktop adoption require separate acknowledgments before final confirmation" {
  adopt_fixture
  python3 - "$DOT_SOURCE/tools/dot/manifest.json" <<'PY'
import json, sys
path=sys.argv[1]
data=json.load(open(path)); data['entries']['prefs']['impact']='Unsafe'
data['entries']['prefs']['warnings']=['desktop-arch-compatibility']
open(path,'w').write(json.dumps(data)+'\n')
PY
  git -C "$DOT_SOURCE" add .
  git -C "$DOT_SOURCE" commit -qm unsafe-fixture
  start_browser
  enter_review
  tmux -L "$SOCKET" send-keys -t approval a
  wait_for 'Unsafe acknowledgment required separately'
  [[ "$(pane)" != *'FINAL CONFIRMATION'* ]]
  tmux -L "$SOCKET" send-keys -t approval u a
  wait_for 'Desktop acknowledgment required separately'
  [[ "$(pane)" != *'FINAL CONFIRMATION'* ]]
  tmux -L "$SOCKET" send-keys -t approval d a
  wait_for 'FINAL CONFIRMATION'
  tmux -L "$SOCKET" send-keys -t approval Enter
  sleep 0.2
  wait_for 'REVIEW EXACT PLAN'
  cancel_review
  wait_for UI_DONE
  [ "$(cat "$HOME/.config/preferences.conf")" = 'target fixture' ]
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
}
@test "named preset is opt-in basket selection not approval" {
  adopt_fixture
  start_browser
  wait_for ADOPT
  tmux -L "$SOCKET" send-keys -t approval s
  wait_for 'OPT-IN EXACT REPOSITORY PRESET'
  wait_for 'work'
  tmux -L "$SOCKET" send-keys -t approval Enter
  wait_for '1 selected'
  [ "$(cat "$HOME/.config/preferences.conf")" = 'target fixture' ]
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
  tmux -L "$SOCKET" send-keys -t approval q
  wait_for UI_DONE
  [ -z "$(git -C "$DOT_SOURCE" status --porcelain)" ]
}
@test "named preset requested during inventory loading waits for exact engine presets" {
  adopt_fixture
  before="$(git -C "$DOT_SOURCE" rev-parse HEAD)"
  printf '{"version":1,"mode":"adopt","profile":"work","entries":["prefs"]}\n' > "$BATS_TEST_TMPDIR/request"
  bash "$DOT_ENGINE/dot" intent plan "$BATS_TEST_TMPDIR/request" --out "$BATS_TEST_TMPDIR/plan" >/dev/null
  token="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["id"])' "$BATS_TEST_TMPDIR/plan")"
  # Delay only the installed adapter's inventory response; all policy is real.
  mv "$DOT_ENGINE/browser_api.py" "$DOT_ENGINE/browser_api_real.py"
  export INVENTORY_STARTED="$BATS_TEST_TMPDIR/inventory-started"
  export INVENTORY_RELEASE="$BATS_TEST_TMPDIR/inventory-release"
  cat > "$DOT_ENGINE/browser_api.py" <<'PY'
import io, json, os, pathlib, runpy, sys, time
body = sys.stdin.read()
if json.loads(body)['command'] == 'inventory':
    pathlib.Path(os.environ['INVENTORY_STARTED']).touch()
    deadline = time.monotonic() + 30
    while not pathlib.Path(os.environ['INVENTORY_RELEASE']).exists():
        if time.monotonic() > deadline:
            raise RuntimeError('test did not release inventory')
        time.sleep(0.01)
sys.stdin = io.StringIO(body)
runpy.run_path(str(pathlib.Path(__file__).with_name('browser_api_real.py')), run_name='__main__')
PY
  start_browser
  wait_for ADOPT
  for i in {1..100}; do
    [ ! -e "$INVENTORY_STARTED" ] || break
    sleep 0.05
  done
  [ -e "$INVENTORY_STARTED" ]
  tmux -L "$SOCKET" send-keys -t approval s
  wait_for 'Loading named presets'
  [[ "$(pane)" == *'0 selected'* ]]
  touch "$INVENTORY_RELEASE"
  wait_for 'OPT-IN EXACT REPOSITORY PRESET'
  wait_for work
  tmux -L "$SOCKET" send-keys -t approval Escape
  wait_for '0 selected'
  tmux -L "$SOCKET" send-keys -t approval s
  wait_for 'OPT-IN EXACT REPOSITORY PRESET'
  [ "$(cat "$HOME/.config/preferences.conf")" = 'target fixture' ]
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
  tmux -L "$SOCKET" send-keys -t approval Enter
  wait_for '1 selected'
  tmux -L "$SOCKET" send-keys -t approval p
  wait_for 'REVIEW EXACT PLAN'
  [[ "$(pane)" == *"$token"* ]]
  cancel_review
  wait_for UI_DONE
  [ "$(git -C "$DOT_SOURCE" rev-parse HEAD)" = "$before" ]
  [ -z "$(git -C "$DOT_SOURCE" status --porcelain)" ]
  [ "$(cat "$HOME/.config/preferences.conf")" = 'target fixture' ]
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
}
@test "Track bounded Git imports only explicit keys and preserves local credentials" {
  mkdir -p "$DOT_SOURCE/home/.chezmoitemplates"
  cp "$BATS_TEST_DIRNAME/fixtures/modify_dot_gitconfig.tmpl" "$DOT_SOURCE/home/"
  cp "$BATS_TEST_DIRNAME/fixtures/.chezmoitemplates/gitconfig.tmpl" "$DOT_SOURCE/home/.chezmoitemplates/"
  python3 - "$DOT_SOURCE/tools/dot/manifest.json" <<'PY'
import json,sys
entry={"destination":".gitconfig","git_keys":["user.name","user.email"],"git_source":"home/.chezmoitemplates/gitconfig.tmpl","hooks":[],"impact":"Unsafe","integrations":["git-config"],"notices":["config-only"],"os":["darwin","linux"],"portability":"managed-templated","removals":[],"source":"home/modify_dot_gitconfig.tmpl","tools":["git"]}
json.dump(dict(version=1,entries={'.gitconfig':entry},profiles={}),open(sys.argv[1],'w'))
PY
  cat > "$HOME/.gitconfig" <<'GIT'
[user]
name = Imported Name
email = do-not-import@example.org
signingkey = do-not-import-key
[credential]
password = do-not-import-secret
[include]
path = ~/do-not-import.conf
GIT
  cp "$HOME/.gitconfig" "$BATS_TEST_TMPDIR/git-before"
  git -C "$DOT_SOURCE" add .
  git -C "$DOT_SOURCE" commit -qm bounded-git
  BROWSER_MODE=track
  start_browser
  wait_for TRACK
  tmux -L "$SOCKET" send-keys -t approval H
  wait_for .gitconfig
  tmux -L "$SOCKET" send-keys -t approval e
  wait_for METADATA
  tmux -L "$SOCKET" send-keys -t approval Tab Tab Tab Tab Tab C-u
  tmux -L "$SOCKET" send-keys -l -t approval user.name
  tmux -L "$SOCKET" send-keys -t approval Enter
  wait_for '1 selected'
  tmux -L "$SOCKET" send-keys -t approval p
  wait_for 'REVIEW EXACT PLAN'
  [[ "$(pane)" != *'do-not-import-secret'* ]]
  tmux -L "$SOCKET" send-keys -t approval u a
  wait_for 'FINAL CONFIRMATION'
  tmux -L "$SOCKET" send-keys -t approval j Enter
  wait_for Applied
  cmp "$HOME/.gitconfig" "$BATS_TEST_TMPDIR/git-before"
  python3 - "$DOT_SOURCE" <<'PY'
import json,pathlib,sys
root=pathlib.Path(sys.argv[1]); entry=json.loads((root/'tools/dot/manifest.json').read_text())['entries']['.gitconfig']
assert entry['git_keys']==['user.name','user.email'], entry
text=(root/entry['git_source']).read_text()
assert 'Imported Name' in text, text
assert 'do-not-import' not in text
assert entry['integrations']==['git-config'] and entry['impact']=='Unsafe'
PY
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
  [ -z "$(git -C "$DOT_SOURCE" status --porcelain)" ]
}
@test "narrow browser keeps navigation preview and cancellation visible" {
  adopt_fixture
  start_browser
  wait_for ADOPT
  tmux -L "$SOCKET" resize-window -t approval -x 80 -y 24
  tmux -L "$SOCKET" send-keys -t approval H
  wait_for '.config'
  tmux -L "$SOCKET" send-keys -t approval l
  wait_for preferences.conf
  tmux -L "$SOCKET" send-keys -t approval j
  wait_for 'PREVIEW'
  wait_for 'repository fixture'
  [[ "$(pane)" == *'Scope: false'* ]]
  [[ "$(pane)" == *'OS: supported'* ]]
  [[ "$(pane)" == *'managed-shared'* && "$(pane)" == *'Safe'* ]]
  [[ "$(pane)" == *'q quit'* ]]
  pane >&3
  tmux -L "$SOCKET" send-keys -t approval q
  wait_for UI_DONE
  [ "$(cat "$HOME/.config/preferences.conf")" = 'target fixture' ]
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
}
@test "singleton skills backup replacement is an independent consent gate" {
  mkdir -p "$HOME/.agents/skills/example" "$HOME/.agents/skills.bak/example"
  printf 'old skill\n' > "$HOME/.agents/skills/example/SKILL.md"
  printf 'old backup\n' > "$HOME/.agents/skills.bak/example/SKILL.md"
  printf '{"version":1,"mode":"track","profile":"skills","selections":[{"path":".agents/skills/example/SKILL.md","os":["darwin","linux"],"portability":"managed-shared","impact":"Safe","integrations":["gemini-skills"]}]}\n' > "$BATS_TEST_TMPDIR/request"
  bash "$DOT_ENGINE/dot" intent plan "$BATS_TEST_TMPDIR/request" --out "$BATS_TEST_TMPDIR/plan" >/dev/null
  token="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["id"])' "$BATS_TEST_TMPDIR/plan")"
  bash "$DOT_ENGINE/dot" intent apply "$BATS_TEST_TMPDIR/plan" --approve "$token" >/dev/null
  printf 'updated skill\n' > "$DOT_SOURCE/home/dot_agents/skills/example/SKILL.md"
  git -C "$DOT_SOURCE" add .
  git -C "$DOT_SOURCE" commit -qm updated-skill
  start_browser
  wait_for ADOPT
  tmux -L "$SOCKET" send-keys -t approval s
  wait_for 'OPT-IN EXACT REPOSITORY PRESET'
  tmux -L "$SOCKET" send-keys -t approval Enter
  wait_for '1 selected'
  tmux -L "$SOCKET" send-keys -t approval p
  wait_for 'REVIEW EXACT PLAN'
  tmux -L "$SOCKET" send-keys -t approval a
  wait_for 'Backup replacement acknowledgment required separately'
  [ "$(cat "$HOME/.agents/skills.bak/example/SKILL.md")" = 'old backup' ]
  tmux -L "$SOCKET" send-keys -t approval b a
  wait_for 'FINAL CONFIRMATION'
  tmux -L "$SOCKET" send-keys -t approval Enter
  sleep 0.2
  cancel_review
  wait_for UI_DONE
  [ "$(cat "$HOME/.agents/skills/example/SKILL.md")" = 'old skill' ]
  [ "$(cat "$HOME/.agents/skills.bak/example/SKILL.md")" = 'old backup' ]
  [ ! -e "$HOME/.gemini/skills/example/SKILL.md" ]
}
@test "optional rounds return to Finish after apply with no inherited selection" {
  adopt_fixture
  start_browser --rounds
  wait_for 'Finish without further adoption'
  tmux -L "$SOCKET" send-keys -t approval j j Enter
  enter_review
  tmux -L "$SOCKET" send-keys -t approval a
  wait_for 'FINAL CONFIRMATION'
  tmux -L "$SOCKET" send-keys -t approval j Enter
  wait_for Applied
  tmux -L "$SOCKET" send-keys -t approval Enter
  wait_for '> Finish without further adoption'
  tmux -L "$SOCKET" send-keys -t approval Enter
  wait_for UI_DONE
  [ "$(cat "$HOME/.config/preferences.conf")" = 'repository fixture' ]
  [ -e "$HOME/.local/state/dot/adoption.json" ]
}
