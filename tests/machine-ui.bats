#!/usr/bin/env bats
setup() {
  command -v tmux >/dev/null || skip 'tmux required'
  REPO="$(cd "$BATS_TEST_DIRNAME/.." && pwd -P)"
  BASE="$(cd "$BATS_TEST_TMPDIR" && pwd -P)"
  export GOPATH="${DOT_BROWSER_TEST_GOPATH:-$BATS_SUITE_TMPDIR/go}" GOCACHE="${DOT_BROWSER_TEST_GOCACHE:-$BATS_SUITE_TMPDIR/go-build}" GOFLAGS=-modcacherw
  export HOME="$BASE/home" DOT_SOURCE="$BASE/private" DOT_ENGINE="$REPO/tools/dot" TERM=xterm-256color NO_COLOR=1
  unset DOT_STATE_DIR XDG_STATE_HOME
  mkdir -p "$HOME" "$DOT_SOURCE/home"
  printf '{"protocol":1,"engine_api":{"min":1,"max":1},"manifest_version":1,"approval_version":2,"manifest":"manifest.json","home_source":"home","required_adapters":["intent","scope","gitconfig","skills","layout"]}\n' > "$DOT_SOURCE/dot-contract.json"
  printf '{"version":1,"entries":{},"profiles":{}}\n' > "$DOT_SOURCE/manifest.json"
  git init -q "$DOT_SOURCE"
  git -C "$DOT_SOURCE" remote add origin https://example.invalid/fixture/dots.git
  BROWSER="${DOT_BROWSER_TEST_BINARY:-$BASE/dot-browser}"
  if [ -z "${DOT_BROWSER_TEST_BINARY:-}" ]; then
    (cd "$REPO/tools/dot/browser" && HOME="$BASE/build-home" go build -mod=readonly -o "$BROWSER" ./cmd/dot-browser)
  fi
  SOCKET="machine-ui-$$-$BATS_TEST_NUMBER"
  local command
  printf -v command '%q --init --engine %q --source %q --home %q; echo UI_DONE; sleep 30' "$BROWSER" "$DOT_ENGINE" "$DOT_SOURCE" "$HOME"
  tmux -L "$SOCKET" new-session -d -s ui -x 100 -y 30 "$command"
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
open_machine() {
  wait_for 'Welcome to dot'
  tmux -L "$SOCKET" send-keys -t ui Enter
  wait_for 'existing checkout'
  tmux -L "$SOCKET" send-keys -t ui Enter
  wait_for 'Identity stays local'
  wait_for 'Machine identity'
}
@test "init terminal edits and validates machine identity before saving local v3" {
  open_machine
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
  tmux -L "$SOCKET" send-keys -t ui C-u
  tmux -L "$SOCKET" send-keys -l -t ui 'Invalid-ID'
  tmux -L "$SOCKET" send-keys -t ui Enter
  wait_for 'use lowercase'
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
  tmux -L "$SOCKET" send-keys -t ui C-u
  tmux -L "$SOCKET" send-keys -l -t ui 'terminal_machine'
  wait_for 'terminal_machine'
  tmux -L "$SOCKET" send-keys -t ui Enter
  wait_for 'Local machine terminal_machine saved'
  python3 - "$HOME/.local/state/dot/adoption.json" <<'PY'
import json, sys
data = json.load(open(sys.argv[1]))
assert data['version'] == 3 and data['machine']['id'] == 'terminal_machine'
assert data['entries'] == {}
PY
  [ -z "$(git -C "$DOT_SOURCE" status --porcelain --untracked-files=no)" ]
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
}
@test "quitting machine identity does not persist a suggestion" {
  open_machine
  tmux -L "$SOCKET" send-keys -t ui Escape
  wait_for 'UI_DONE'
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
}
