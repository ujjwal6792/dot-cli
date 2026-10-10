#!/usr/bin/env bats
setup() {
  command -v tmux >/dev/null || skip 'tmux required'
  REPO="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
  export GOPATH="${DOT_BROWSER_TEST_GOPATH:-$BATS_SUITE_TMPDIR/go}" GOCACHE="${DOT_BROWSER_TEST_GOCACHE:-$BATS_SUITE_TMPDIR/go-build}" GOFLAGS=-modcacherw
  export HOME="$BATS_TEST_TMPDIR/home" DOT_SOURCE="$BATS_TEST_TMPDIR/private" DOT_ENGINE="$BATS_TEST_TMPDIR/public/tools/dot" TERM=xterm-256color NO_COLOR=1
  mkdir -p "$HOME/.local/state/dot" "$DOT_SOURCE/home" "$DOT_SOURCE/tools/dot" "$DOT_ENGINE"
  cp "$REPO/tools/dot/"*.py "$DOT_ENGINE/"
  cp "$REPO/tools/dot/engine.json" "$REPO/tools/dot/dot" "$DOT_ENGINE/"
  printf '{"protocol":1,"engine_api":{"min":1,"max":1},"manifest_version":1,"approval_version":2,"manifest":"tools/dot/manifest.json","home_source":"home","required_adapters":["intent","scope","gitconfig","skills","layout"]}\n' > "$DOT_SOURCE/dot-contract.json"
  export DOT_CONFIG="$BATS_TEST_TMPDIR/tracker.toml"
  printf 'sourceDir="%s/home"\n[data]\ndotEngine="%s"\n' "$DOT_SOURCE" "$DOT_ENGINE" > "$DOT_CONFIG"
  printf 'private preference\n' > "$DOT_SOURCE/home/prefs.conf"
  printf 'live preference\n' > "$HOME/prefs.conf"
  chmod 640 "$HOME/prefs.conf"
  cat > "$DOT_SOURCE/tools/dot/manifest.json" <<'JSON'
{"version":1,"entries":{"prefs":{"destination":"prefs.conf","source":"home/prefs.conf","os":["darwin","linux"],"portability":"managed-shared","impact":"Safe","removals":[],"hooks":[],"tools":[]}},"profiles":{"default":["prefs"]}}
JSON
  python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); json.dump(dict(version=2,profile="default",entries=d["entries"]),open(sys.argv[2],"w"))' "$DOT_SOURCE/tools/dot/manifest.json" "$HOME/.local/state/dot/adoption.json"
  git init -q "$DOT_SOURCE"
  git -C "$DOT_SOURCE" config user.name Fixture
  git -C "$DOT_SOURCE" config user.email fixture@example.invalid
  git -C "$DOT_SOURCE" add .
  git -C "$DOT_SOURCE" commit -qm fixture
  export DOT_BROWSER="${DOT_BROWSER_TEST_BINARY:-$BATS_TEST_TMPDIR/dot-browser}"
  if [ -z "${DOT_BROWSER_TEST_BINARY:-}" ]; then
    (cd "$REPO/tools/dot/browser" && HOME="$BATS_TEST_TMPDIR/build-home" go build -mod=readonly -o "$DOT_BROWSER" ./cmd/dot-browser)
  fi
  SOCKET="tracking-ui-$$-$BATS_TEST_NUMBER"
}
teardown() { tmux -L "$SOCKET" kill-server 2>/dev/null || true; }
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
start_browser() {
  local command
  printf -v command 'bash %q browse; code=$?; echo UI_EXIT=$code; echo UI_DONE; sleep 30' "$DOT_ENGINE/dot"
  tmux -L "$SOCKET" new-session -d -s ui -x 140 -y 45 "$command"
}
@test "Dotfiles stop-tracking final confirmation defaults Cancel then exact apply retains live bytes/mode" {
  start_browser
  wait_for 'Dotfiles'
  tmux -L "$SOCKET" send-keys -t ui u
  wait_for 'STOP TRACKING'
  wait_for prefs.conf
  wait_for 'Approved scope: true'
  wait_for 'private preference'
  tmux -L "$SOCKET" send-keys -t ui Space
  wait_for '1 selected'
  tmux -L "$SOCKET" send-keys -t ui p
  wait_for 'REVIEW EXACT PLAN'
  wait_for 'live files unchanged'
  [ -f "$DOT_SOURCE/home/prefs.conf" ]
  tmux -L "$SOCKET" send-keys -t ui a
  wait_for 'FINAL CONFIRMATION'
  [[ "$(pane)" == *'> Cancel'* ]]
  tmux -L "$SOCKET" send-keys -t ui Enter
  wait_for 'canceled'
  [ -f "$DOT_SOURCE/home/prefs.conf" ]
  tmux -L "$SOCKET" send-keys -t ui a j Enter
  wait_for 'Applied reviewed plan'
  [ ! -e "$DOT_SOURCE/home/prefs.conf" ]
  [ "$(cat "$HOME/prefs.conf")" = 'live preference' ]
  [ "$(stat -f '%Lp' "$HOME/prefs.conf")" = 640 ]
  python3 -c 'import json,sys; assert json.load(open(sys.argv[1]))["entries"] == {}' "$HOME/.local/state/dot/adoption.json"
  [ -z "$(git -C "$DOT_SOURCE" status --porcelain)" ]
  pane >&3
}
@test "Dotfiles logical metadata editor reviews a source-only metadata change before applying" {
  os_name=darwin
  [ "$(uname -s)" != Linux ] || os_name=linux
  python3 - "$DOT_SOURCE" "$HOME" "$os_name" <<'PY'
import json, pathlib, sys
source, home, system = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2]), sys.argv[3]
path = source / 'tools/dot/manifest.json'
data = json.loads(path.read_text())
data.update(version=2, machines={'fixture-machine': {'label': 'Fixture machine', 'os': system}}, tracking={})
path.write_text(json.dumps(data))
path = source / 'dot-contract.json'
contract = json.loads(path.read_text()); contract.update(manifest_version=2, approval_version=3)
path.write_text(json.dumps(contract))
path = home / '.local/state/dot/machine.json'
path.write_text(json.dumps({'version':1, 'id':'fixture-machine'})); path.chmod(0o600)
PY
  git -C "$DOT_SOURCE" add .
  git -C "$DOT_SOURCE" commit -qm 'private fixture metadata'
  printf '{"version":1,"mode":"track","profile":"default","selections":[{"path":"prefs.conf","logical":"mapped","destinations":{"%s":"mapped.conf"},"common":{"impact":"Safe","portability":"managed-shared"}}]}\n' "$os_name" > "$BATS_TEST_TMPDIR/request"
  "$DOT_ENGINE/dot" intent plan "$BATS_TEST_TMPDIR/request" --out "$BATS_TEST_TMPDIR/plan" >/dev/null
  token="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["id"])' "$BATS_TEST_TMPDIR/plan")"
  "$DOT_ENGINE/dot" intent apply "$BATS_TEST_TMPDIR/plan" --approve "$token" >/dev/null
  printf '{"version":1,"mode":"adopt","profile":"default","entries":["mapped@%s"]}\n' "$os_name" > "$BATS_TEST_TMPDIR/request"
  "$DOT_ENGINE/dot" intent plan "$BATS_TEST_TMPDIR/request" --out "$BATS_TEST_TMPDIR/plan" >/dev/null
  token="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["id"])' "$BATS_TEST_TMPDIR/plan")"
  "$DOT_ENGINE/dot" intent apply "$BATS_TEST_TMPDIR/plan" --approve "$token" >/dev/null
  start_browser
  wait_for 'mapped.conf'
  tmux -L "$SOCKET" send-keys -t ui / mapped.conf Enter
  wait_for 'Approved scope: true'
  tmux -L "$SOCKET" send-keys -t ui M
  wait_for 'LOGICAL FILE METADATA'
  tmux -L "$SOCKET" send-keys -t ui C-a C-k
  tmux -L "$SOCKET" send-keys -l -t ui '{"version":1,"mode":"metadata","profile":"default","metadata":{"mapped":{"common":{"impact":"Unsafe","impact":"Safe","portability":"managed-shared"}}}}'
  tmux -L "$SOCKET" send-keys -t ui C-s
  wait_for 'duplicate JSON field'
  tmux -L "$SOCKET" send-keys -t ui C-a C-k
  tmux -L "$SOCKET" send-keys -l -t ui '{"version":1,"mode":"metadata","profile":"default","metadata":{"mapped":{"common":{"impact":"Safe","portability":"managed-shared","tools":["fixture-editor-tool"]}}}}'
  tmux -L "$SOCKET" send-keys -t ui C-s
  wait_for 'Logical metadata request prepared'
  tmux -L "$SOCKET" send-keys -t ui p
  wait_for 'REVIEW EXACT PLAN'
  wait_for 'live files unchanged'
  tmux -L "$SOCKET" send-keys -t ui a
  wait_for 'FINAL CONFIRMATION'
  tmux -L "$SOCKET" send-keys -t ui j Enter
  wait_for 'Applied reviewed plan'
  [ "$(cat "$HOME/mapped.conf")" = 'live preference' ]
  python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); assert next(iter(d["entries"].values()))["tools"] == ["fixture-editor-tool"]' "$HOME/.local/state/dot/adoption.json"
  [ -z "$(git -C "$DOT_SOURCE" status --porcelain)" ]
}
