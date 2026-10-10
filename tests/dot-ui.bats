#!/usr/bin/env bats
# Real gum in an isolated tmux server; only external tools are fixtures.
setup() {
  command -v gum >/dev/null || skip 'gum required for UI tests'
  command -v tmux >/dev/null || skip 'tmux required for UI tests'
  REPO="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
  export HOME="$BATS_TEST_TMPDIR/home" DOT_SOURCE="$BATS_TEST_TMPDIR/source"
  export DOT_ENGINE="$REPO/tools/dot" DOT_CONFIG="$HOME/.config/chezmoi/chezmoi.toml"
  export TRACE="$BATS_TEST_TMPDIR/trace"
  mkdir -p "$HOME/.config/chezmoi" "$DOT_SOURCE/scripts" "$DOT_SOURCE/packages" "$BATS_TEST_TMPDIR/bin"
  printf '[data]\nname="Fixture"\nemail="fixture@example.org"\nprofile="test"\ndotEngine="%s"\n' "$DOT_ENGINE" > "$DOT_CONFIG"
  python3 "$BATS_TEST_DIRNAME/helpers/private-config.py" --source "$DOT_SOURCE" \
    --fixtures "$BATS_TEST_DIRNAME/fixtures" --empty
  touch "$TRACE"
  DOT="$REPO/tools/dot/dot"
  SOCKET="dot-ui-$$-$BATS_TEST_NUMBER"
  cat > "$BATS_TEST_TMPDIR/bin/chezmoi" <<'SH'
#!/usr/bin/env bash
printf 'chezmoi %s\n' "$*" >> "$TRACE"
if [ "${UI_FAIL:-}" = yes ]; then echo 'fixture diagnostic failure' >&2; exit 7; fi
case " $* " in
  *' diff '*)
    if [ "${UI_DIFF:-}" = yes ]; then
      printf 'diff --git a/first b/first\n--- a/first\n+++ b/first\n@@ -1 +1 @@\n-old-first\n+new-first\ndiff --git a/second b/second\n--- a/second\n+++ b/second\n@@ -1 +1 @@\n-old-second\n+new-second\n'
    else echo 'fixture diff'; fi ;;
  # Mutating verbs request machine-readable status; this fixture is clean.
  *' status --include '*|*' status --exclude '*) ;;
  *' status '*) echo 'fixture status result' ;;
esac
SH
  cat > "$BATS_TEST_TMPDIR/bin/git" <<'SH'
#!/usr/bin/env bash
printf 'git %s\n' "$*" >> "$TRACE"
case " $* " in *' rev-parse '*) echo origin/main ;; esac
SH
  chmod +x "$BATS_TEST_TMPDIR/bin/chezmoi" "$BATS_TEST_TMPDIR/bin/git"
  export PATH="$BATS_TEST_TMPDIR/bin:$PATH"
  printf '[pins.example]\nversion = "1.0.0"\ncheck = "example --version"\ninstall_darwin = "example@1.0.0"\ninstall_linux = "example@1.0.0"\n' > "$DOT_SOURCE/packages/pins.toml"
  printf 'echo script-called >> "$TRACE"\n' > "$DOT_SOURCE/scripts/example.sh"
  printf 'echo pins-called >> "$TRACE"\n' > "$DOT_SOURCE/scripts/install-pins.sh"
}

@test "status failure remains visible after the UI exits" {
  export UI_FAIL=yes
  select_verb 0
  wait_for UI_DONE
  [[ "$(pane)" == *'fixture diagnostic failure'* ]]
  [[ "$(pane)" == *'failed (exit 7)'* ]]
  [[ "$(pane)" == *'UI_EXIT=7'* ]]
}

@test "doctor check failures and results remain visible after the UI exits" {
  export UI_FAIL=yes
  select_verb 8
  wait_for UI_DONE
  [[ "$(pane)" == *'fixture diagnostic failure'* ]]
  [[ "$(pane)" == *'failed (exit 1)'* ]]
  [[ "$(pane)" == *'UI_EXIT=1'* ]]
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
start_menu() {
  local command
  printf -v command 'bash %q; code=$?; echo UI_EXIT=$code; echo UI_DONE; sleep 30' "$DOT"
  tmux -L "$SOCKET" new-session -d -s ui -x 120 -y 40 "$command"
  wait_for 'configuration management'
}
select_verb() {
  local i
  start_menu
  for ((i=0; i<$1; i++)); do tmux -L "$SOCKET" send-keys -t ui Down; done
  tmux -L "$SOCKET" send-keys -t ui Enter
}
input() {
  wait_for "$1"
  tmux -L "$SOCKET" send-keys -t ui -l "$2"
  tmux -L "$SOCKET" send-keys -t ui Enter
}
yes() { tmux -L "$SOCKET" send-keys -t ui Left Enter; }
close_preview() {
  wait_for 'Inspect changes'
  tmux -L "$SOCKET" send-keys -t ui Enter
}

@test "successful status output remains in the terminal after execution" {
  select_verb 0
  wait_for UI_DONE
  [[ "$(pane)" == *'fixture status result'* ]]
  [[ "$(pane)" == *'drift: 1'* ]]
}

@test "diff starts with a summary and shows only the requested file on demand" {
  export UI_DIFF=yes
  select_verb 1
  wait_for 'Diff direction'
  tmux -L "$SOCKET" send-keys -t ui Enter
  wait_for 'Inspect changes'
  [[ "$(pane)" == *'first  +1 -1'* ]]
  [[ "$(pane)" == *'second  +1 -1'* ]]
  [[ "$(pane)" != *new-first* ]]
  tmux -L "$SOCKET" send-keys -t ui Down Down Enter
  wait_for new-first
  [[ "$(pane)" != *new-second* ]]
  tmux -L "$SOCKET" send-keys -t ui q
  wait_for 'Inspect changes'
  tmux -L "$SOCKET" send-keys -t ui Enter
  wait_for UI_DONE
  [[ "$(pane)" == *'Changed-file summary'* ]]
}

@test "menu renders all nine verbs and cancellation invokes nothing" {
  start_menu
  for verb in status diff push pull add remove update run doctor; do [[ "$(pane)" == *"$verb"* ]]; done
  tmux -L "$SOCKET" send-keys -t ui Escape
  wait_for UI_DONE
  [ ! -s "$TRACE" ]
}

@test "navigation reaches every verb and invokes its non-interactive core" {
  local index
  for index in 0 1 2 4 5 6 7 8; do
    select_verb "$index"
    case "$index" in
      1) wait_for 'Diff direction'; tmux -L "$SOCKET" send-keys -t ui Enter; close_preview ;;
      2) close_preview; input 'Commit message' fixture; wait_for 'Commit and push'; yes ;;
      3) close_preview; wait_for 'Apply remote'; yes ;;
      4) printf 'fixture\n' > "$HOME/.new"; input 'Path to adopt' "$HOME/.new"; wait_for Classification; tmux -L "$SOCKET" send-keys -t ui Enter ;;
      5) input 'Path to unmanage' "$HOME/.new"; wait_for Unmanage; yes ;;
      6) input 'Pin name' example; input 'New version' 2.0.0; wait_for 'run installers'; yes ;;
      7) wait_for 'Run script'; tmux -L "$SOCKET" send-keys -t ui Enter; wait_for 'Scripts may change'; yes ;;
    esac
    wait_for UI_DONE
    if [ "$index" = 4 ] || [ "$index" = 5 ]; then
      [[ "$(pane)" == *'UI_EXIT=1'* ]] || { pane >&2; cat "$TRACE" >&2; return 1; }
      [[ "$(pane)" == *'exact dot intent'* ]]
    else
      [[ "$(pane)" == *'UI_EXIT=0'* ]] || { pane >&2; cat "$TRACE" >&2; return 1; }
    fi
    case "$index" in
      0) [[ "$(cat "$TRACE")" == *' status'* ]] ;;
      1) [[ "$(cat "$TRACE")" == *' diff'* ]] ;;
      2) [[ "$(cat "$TRACE")" == *' push'* ]] ;;
      3) [[ "$(cat "$TRACE")" == *' apply --force'* ]] ;;
      4) ! grep -q ' add --force' "$TRACE" ;;
      5) ! grep -q ' forget --force' "$TRACE" ;;
      6) [[ "$(cat "$DOT_SOURCE/packages/pins.toml")" == *'2.0.0'* ]] ;;
      7) [[ "$(cat "$TRACE")" == *script-called* ]] ;;
      8) [[ "$(cat "$TRACE")" == *' doctor'* ]] ;;
    esac
    tmux -L "$SOCKET" kill-server
    : > "$TRACE"
  done
}

@test "pull without exact approval fails closed before fetch or application" {
  select_verb 3
  wait_for UI_DONE
  [[ "$(pane)" == *'pull requires an approved exact scope'* ]]
  [[ "$(pane)" == *'UI_EXIT=1'* ]]
  ! grep -Eq 'fetch|pull|apply' "$TRACE"
}

@test "contracted private pull reviews exact scope defaults No then applies only after consent" {
  mv "$BATS_TEST_TMPDIR/bin/git" "$BATS_TEST_TMPDIR/bin/mock-git"
  mv "$BATS_TEST_TMPDIR/bin/chezmoi" "$BATS_TEST_TMPDIR/bin/mock-chezmoi"
  REPO="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
  export DOT_ENGINE="$REPO/tools/dot" DOT_CONFIG="$HOME/tracker.toml"
  mkdir -p "$DOT_SOURCE/home" "$DOT_SOURCE/tools/dot" "$HOME/.local/state/dot"
  printf '{"protocol":1,"engine_api":{"min":1,"max":1},"manifest_version":1,"approval_version":2,"manifest":"tools/dot/manifest.json","home_source":"home","required_adapters":["intent","scope","gitconfig","skills","layout"]}\n' > "$DOT_SOURCE/dot-contract.json"
  cp "$BATS_TEST_DIRNAME/fixtures/chezmoiignore.tmpl" "$DOT_SOURCE/home/.chezmoiignore"; cp "$BATS_TEST_DIRNAME/fixtures/chezmoiremove.tmpl" "$DOT_SOURCE/home/.chezmoiremove"
  printf 'old fixture\n' > "$DOT_SOURCE/home/dot_fixture"
  printf 'old fixture\n' > "$HOME/.fixture"
  printf 'sourceDir="%s/home"\n[data]\ndotEngine="%s"\n' "$DOT_SOURCE" "$DOT_ENGINE" > "$DOT_CONFIG"
  printf '{"version":1,"entries":{"fixture":{"destination":".fixture","source":"home/dot_fixture","os":["darwin","linux"],"portability":"managed-shared","impact":"Safe","removals":[],"hooks":[],"tools":[]}},"profiles":{"fixture":["fixture"]}}\n' > "$DOT_SOURCE/tools/dot/manifest.json"
  python3 - "$DOT_SOURCE/tools/dot/manifest.json" "$HOME/.local/state/dot/adoption.json" <<'PY'
import json,sys
manifest=json.load(open(sys.argv[1]))
json.dump(dict(version=2,profile='fixture',entries=manifest['entries']),open(sys.argv[2],'w'))
PY
  git init -q --bare "$BATS_TEST_TMPDIR/remote"
  git init -q "$DOT_SOURCE"
  git -C "$DOT_SOURCE" config user.name Fixture
  git -C "$DOT_SOURCE" config user.email fixture@example.invalid
  git -C "$DOT_SOURCE" add .
  git -C "$DOT_SOURCE" commit -qm fixture
  git -C "$DOT_SOURCE" remote add origin "$BATS_TEST_TMPDIR/remote"
  git -C "$DOT_SOURCE" push -qu origin HEAD
  git clone -q "$BATS_TEST_TMPDIR/remote" "$BATS_TEST_TMPDIR/author"
  printf 'new fixture\n' > "$BATS_TEST_TMPDIR/author/home/dot_fixture"
  git -C "$BATS_TEST_TMPDIR/author" -c user.name=Fixture -c user.email=fixture@example.invalid commit -qam update
  git -C "$BATS_TEST_TMPDIR/author" push -q
  before="$(git -C "$DOT_SOURCE" rev-parse HEAD)"
  cp "$HOME/.local/state/dot/adoption.json" "$BATS_TEST_TMPDIR/approval-before"
  select_verb 3
  close_preview
  wait_for 'exact approved scope'
  tmux -L "$SOCKET" send-keys -t ui Enter
  wait_for UI_DONE
  [ "$(git -C "$DOT_SOURCE" rev-parse HEAD)" = "$before" ]
  [ "$(cat "$HOME/.fixture")" = 'old fixture' ]
  cmp "$BATS_TEST_TMPDIR/approval-before" "$HOME/.local/state/dot/adoption.json"
  tmux -L "$SOCKET" kill-server
  select_verb 3
  close_preview
  wait_for 'exact approved scope'
  yes
  wait_for UI_DONE
  [[ "$(pane)" == *'UI_EXIT=0'* ]]
  [ "$(cat "$HOME/.fixture")" = 'new fixture' ]
  cmp "$BATS_TEST_TMPDIR/approval-before" "$HOME/.local/state/dot/adoption.json"
}

@test "remove and pin update default to No and do not mutate state" {
  local index
  for index in 5 6; do
    select_verb "$index"
    case "$index" in
      3) close_preview; wait_for 'Apply remote' ;;
      5) input 'Path to unmanage' "$HOME/.new"; wait_for Unmanage ;;
      6) input 'Pin name' example; input 'New version' 2.0.0; wait_for 'run installers' ;;
    esac
    tmux -L "$SOCKET" send-keys -t ui Enter
    wait_for UI_DONE
    [[ "$(cat "$TRACE")" != *' apply '* ]]
    [[ "$(cat "$TRACE")" != *' forget '* ]]
    [[ "$(cat "$TRACE")" != *pins-called* ]]
    [[ "$(cat "$DOT_SOURCE/packages/pins.toml")" == *'1.0.0'* ]]
    tmux -L "$SOCKET" kill-server
  done
}
