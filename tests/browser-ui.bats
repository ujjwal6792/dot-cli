#!/usr/bin/env bats
setup() {
  command -v tmux >/dev/null || skip 'tmux required'
  REPO="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
  export GOPATH="${DOT_BROWSER_TEST_GOPATH:-$BATS_SUITE_TMPDIR/go}"
  export GOCACHE="$GOPATH/build-cache"
  export GOFLAGS=-modcacherw
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
    (cd "$REPO/tools/dot/browser" && HOME="$BATS_TEST_TMPDIR/build-home" go build -mod=readonly -o "$DOT_BROWSER" ./cmd/dot-browser)
  fi
  SOCKET="browser-ui-$$-$BATS_TEST_NUMBER"
}
teardown() { tmux -L "$SOCKET" kill-server 2>/dev/null || true; }
package_fixture() {
  mkdir -p "$BATS_TEST_TMPDIR/managers" "$DOT_SOURCE/packages" "$DOT_SOURCE/supplied"
  export PACKAGE_FIXTURE_STATE="$BATS_TEST_TMPDIR/package-state" PACKAGE_FIXTURE_LOG="$BATS_TEST_TMPDIR/package-log"
  printf 'fixture-cli\n' > "$PACKAGE_FIXTURE_STATE"
  : > "$PACKAGE_FIXTURE_LOG"
  printf 'brew "fixture-cli"\nbrew "fixture-tmux"\n' > "$DOT_SOURCE/packages/Brewfile"
  printf 'fixture-cli\nfixture-tmux\n' > "$DOT_SOURCE/packages/pacman.txt"
  printf 'fixture-cli\nfixture-tmux\n' > "$DOT_SOURCE/packages/apt.txt"
  printf 'brew "fixture-tmux"\n' > "$DOT_SOURCE/supplied/brew-list"
  printf 'fixture-tmux\n' > "$DOT_SOURCE/supplied/native-list"
  cat > "$BATS_TEST_TMPDIR/managers/manager" <<'SH'
#!/bin/sh
name=${0##*/}
if [ "$name" = sudo ] && [ "${1:-}" != --version ]; then exec "$@"; fi
case " $* " in
  *' --version '*) sleep 0.1; printf 'fixture-manager 1.0\n' ;;
  *' --print-architecture '*) printf 'amd64\n' ;;
  *' --dry-run '*|*' -Sp '*|*' -Si '*) printf 'fixture-tmux 1.0\n' ;;
  *' -s '*) printf 'Inst fixture-tmux (1.0 fixture)\nConf fixture-tmux (1.0 fixture)\n' ;;
  *' -W '*) while IFS= read -r item; do printf '%s\tinstalled\n' "$item"; done < "$PACKAGE_FIXTURE_STATE" ;;
  *' --cask '*) : ;;
  *' list '*|*' -Qq '*) cat "$PACKAGE_FIXTURE_STATE" ;;
  *' install '*|*' -S '*) printf '%s %s\n' "$name" "$*" >> "$PACKAGE_FIXTURE_LOG"; printf 'fixture-tmux\n' >> "$PACKAGE_FIXTURE_STATE" ;;
  *) exit 1 ;;
esac
SH
  chmod +x "$BATS_TEST_TMPDIR/managers/manager"
  for manager in brew pacman yay apt-get dpkg-query dpkg sudo; do
    cp "$BATS_TEST_TMPDIR/managers/manager" "$BATS_TEST_TMPDIR/managers/$manager"
  done
  export PATH="$BATS_TEST_TMPDIR/managers:$PATH"
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
start_browser() {
  local command
  printf -v command 'bash %q browse %s; code=$?; echo UI_EXIT=$code; echo UI_DONE; sleep 30' "$DOT_ENGINE/dot" "$*"
  tmux -L "$SOCKET" new-session -d -s ui -x 120 -y 40 "$command"
}
@test "Packages private import save and navigation are inert on wide and narrow terminals" {
  package_fixture
  start_browser
  wait_for 'Packages (P)'
  tmux -L "$SOCKET" send-keys -t ui P
  wait_for 'fixture-tmux'
  [[ "$(pane)" == *'desired · missing'* ]]
  tmux -L "$SOCKET" resize-window -t ui -x 70 -y 24
  wait_for 'Install missing'
  pane >&3
  tmux -L "$SOCKET" send-keys -t ui e
  wait_for 'PRIVATE PACKAGE SOURCE'
  tmux -L "$SOCKET" send-keys -t ui C-s
  wait_for 'saved with backup'
  [ ! -s "$PACKAGE_FIXTURE_LOG" ]
  tmux -L "$SOCKET" send-keys -t ui L
  wait_for 'IMPORT PRIVATE PACKAGE SOURCE'
  if [[ "$(pane)" == *'brew'* ]]; then supplied=supplied/brew-list; else supplied=supplied/native-list; fi
  tmux -L "$SOCKET" send-keys -t ui -l "$supplied"
  tmux -L "$SOCKET" send-keys -t ui C-s
  wait_for 'saved with backup'
  tmux -L "$SOCKET" send-keys -t ui r
  wait_for 'installed · not desired'
  [ ! -s "$PACKAGE_FIXTURE_LOG" ]
  tmux -L "$SOCKET" send-keys -t ui P
  wait_for 'Dotfiles'
  tmux -L "$SOCKET" send-keys -t ui q
  wait_for UI_DONE
  [ ! -s "$PACKAGE_FIXTURE_LOG" ]
}
@test "Packages Install missing defaults to cancel then installs only the freshly approved fixture target" {
  package_fixture
  start_browser
  wait_for 'Packages (P)'
  tmux -L "$SOCKET" send-keys -t ui P
  wait_for 'fixture-tmux'
  tmux -L "$SOCKET" send-keys -t ui i
  wait_for 'Preparing an exact'
  wait_for 'INSTALL MISSING'
  wait_for 'Plan ID:'
  [ ! -s "$PACKAGE_FIXTURE_LOG" ]
  pane >&3
  first_id="$(pane | sed -n 's/^Plan ID: //p')"
  tmux -L "$SOCKET" send-keys -t ui a
  wait_for 'FINAL CONSENT'
  [[ "$(pane)" == *'> Cancel'* ]]
  tmux -L "$SOCKET" send-keys -t ui Enter
  wait_for 'Installation canceled'
  [ ! -s "$PACKAGE_FIXTURE_LOG" ]
  for source in Brewfile pacman.txt apt.txt; do printf '# independently reviewed edit\n' >> "$DOT_SOURCE/packages/$source"; done
  tmux -L "$SOCKET" send-keys -t ui i
  wait_for 'Preparing an exact'
  wait_for 'Plan ID:'
  for attempt in {1..100}; do
    [[ "$(pane)" != *"$first_id"* && "$(pane)" == *'Plan ID:'* ]] && break
    sleep 0.1
  done
  [[ "$(pane)" != *"$first_id"* ]]
  tmux -L "$SOCKET" send-keys -t ui a
  wait_for 'FINAL CONSENT'
  tmux -L "$SOCKET" send-keys -t ui j
  wait_for '> Install exact reviewed'
  tmux -L "$SOCKET" send-keys -t ui Enter
  wait_for 'Reviewed installation completed'
  [[ "$(cat "$PACKAGE_FIXTURE_LOG")" == *'fixture-tmux'* ]]
  [[ "$(cat "$PACKAGE_FIXTURE_LOG")" != *'fixture-cli'* ]]
  tmux -L "$SOCKET" send-keys -t ui q
  wait_for UI_DONE
}
@test "Packages changed private source invalidates final installation approval" {
  package_fixture
  start_browser
  wait_for 'Packages (P)'
  tmux -L "$SOCKET" send-keys -t ui P
  wait_for 'fixture-tmux'
  tmux -L "$SOCKET" send-keys -t ui i
  wait_for 'Preparing an exact'
  wait_for 'Plan ID:'
  tmux -L "$SOCKET" send-keys -t ui a
  wait_for 'FINAL CONSENT'
  for source in Brewfile pacman.txt apt.txt; do printf '# changed after review\n' >> "$DOT_SOURCE/packages/$source"; done
  tmux -L "$SOCKET" send-keys -t ui j
  wait_for '> Install exact reviewed'
  tmux -L "$SOCKET" send-keys -t ui Enter
  wait_for 'stale plan'
  [ ! -s "$PACKAGE_FIXTURE_LOG" ]
  tmux -L "$SOCKET" send-keys -t ui q
  wait_for UI_DONE
}
adopt_fixture() {
  mkdir -p "$DOT_SOURCE/home/dot_config" "$HOME/.config"
  printf 'repository theme\n' > "$DOT_SOURCE/home/dot_config/preferences.conf"
  printf 'target theme\n' > "$HOME/.config/preferences.conf"
  cat > "$DOT_SOURCE/tools/dot/manifest.json" <<'JSON'
{"version":1,"entries":{"prefs":{"destination":".config/preferences.conf","source":"home/dot_config/preferences.conf","os":["darwin","linux"],"portability":"managed-shared","impact":"Safe","removals":[],"hooks":[],"tools":[]}},"profiles":{"work":["prefs"]}}
JSON
  git -C "$DOT_SOURCE" add .
  git -C "$DOT_SOURCE" commit -qm preferences
}
mixed_adopt_fixture() {
  local system="$1"
  python3 - "$DOT_ENGINE" "$DOT_SOURCE" "$HOME" "$system" <<'PY'
import json,sys
from pathlib import Path
engine,source,home=map(Path,sys.argv[1:4]); system=sys.argv[4]
sys.path.insert(0,str(engine))
from file_metadata import compile_entries
def entry(destination,source_file,systems,impact='Safe',portability='managed-shared'):
    return dict(destination=destination,source=source_file,os=systems,impact=impact,portability=portability,removals=[],hooks=[],tools=[])
foreign='linux' if system=='darwin' else 'darwin'
entries={'common':entry('.config/common.conf','home/dot_config/common.conf',['darwin','linux']),
         'eligible':entry('.config/zeligible.conf','home/dot_config/zeligible.conf.tmpl',[system],'Unsafe','managed-templated'),
         'foreign':entry('.config/foreign.conf','home/dot_config/foreign.conf',[foreign])}
definition=dict(destinations={system:'.config/other-machine.conf'},sources={system:'home/dot_config/other-machine.conf'},common={},machine={},os_metadata={},machines=['other'])
entries.update(compile_entries('othermachine',definition))
catalog=dict(version=2,entries=entries,profiles={},machines={'fixture':{'label':'Fixture','os':system},'other':{'label':'Other','os':system}},tracking={'othermachine':definition})
(source/'tools/dot/manifest.json').write_text(json.dumps(catalog))
contract=json.loads((source/'dot-contract.json').read_text());contract.update(manifest_version=2,approval_version=3)
(source/'dot-contract.json').write_text(json.dumps(contract))
for value in entries.values():
    path=source/value['source'];path.parent.mkdir(parents=True,exist_ok=True);path.write_text('repository fixture\n')
state=home/'.local/state/dot';state.mkdir(parents=True)
identity=state/'machine.json';identity.write_text(json.dumps({'version':1,'id':'fixture'}));identity.chmod(0o600)
PY
  git -C "$DOT_SOURCE" add .
  git -C "$DOT_SOURCE" commit -qm 'mixed OS and machine applicability fixture'
}
mixed_adopt_folder_round() {
  local system="$1"
  start_browser --mode adopt
  wait_for ADOPT
  tmux -L "$SOCKET" send-keys -t ui H
  wait_for '.config'
  tmux -L "$SOCKET" send-keys -t ui Space
  wait_for '2 selected'
  [[ "$(pane)" == *'[x] .config'* ]]
  tmux -L "$SOCKET" send-keys -t ui l j
  wait_for '>[x] .config/common.conf'
  tmux -L "$SOCKET" send-keys -t ui Space
  wait_for '1 selected'
  [[ "$(pane)" == *'[-] .config'* ]]
  [[ "$(pane)" == *'[ ] .config/foreign.conf'* ]]
  [[ "$(pane)" == *'[ ] .config/other-machine.conf'* ]]
  printf 'future opt-in\n' > "$DOT_SOURCE/home/dot_config/future.conf"
  python3 - "$DOT_SOURCE/tools/dot/manifest.json" <<'PY'
import json,sys
path=sys.argv[1];catalog=json.load(open(path));value=dict(catalog['entries']['common'])
value.update(destination='.config/future.conf',source='home/dot_config/future.conf')
catalog['entries']['future']=value;json.dump(catalog,open(path,'w'))
PY
  git -C "$DOT_SOURCE" add .; git -C "$DOT_SOURCE" commit -qm 'future file remains opt-in'
  printf '{"version":1,"mode":"adopt","profile":"default","entries":["eligible"]}\n' > "$BATS_TEST_TMPDIR/request"
  "$DOT_ENGINE/dot" intent plan "$BATS_TEST_TMPDIR/request" --out "$BATS_TEST_TMPDIR/plan"
  token="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["id"])' "$BATS_TEST_TMPDIR/plan")"
  tmux -L "$SOCKET" send-keys -t ui / zeligible Enter p
  wait_for 'REVIEW EXACT PLAN'
  [[ "$(pane)" == *"$token"* ]]
  [[ "$(pane)" == *'Unsafe'* ]]
  tmux -L "$SOCKET" send-keys -t ui a
  wait_for 'Unsafe acknowledgment required'
  tmux -L "$SOCKET" send-keys -t ui q q
  wait_for UI_DONE
  [ ! -e "$HOME/.config/common.conf" ]
  [ ! -e "$HOME/.config/zeligible.conf" ]
  [ ! -e "$HOME/.config/foreign.conf" ]
  [ ! -e "$HOME/.config/other-machine.conf" ]
  [ ! -e "$HOME/.config/future.conf" ]
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
}
@test "native Adopt folder snapshots exclude foreign OS and other machine without widening future consent" {
  system="$(python3 -c 'import sys; print("linux" if sys.platform.startswith("linux") else sys.platform)')"
  mixed_adopt_fixture "$system"
  mixed_adopt_folder_round "$system"
}
@test "Linux-platform engine fixture excludes macOS and other machine in actual prebuilt Adopt folder" {
  mixed_adopt_fixture linux
  real_python="$(command -v python3)"
  mkdir -p "$BATS_TEST_TMPDIR/linux-python"
  printf '#!%s\n' "$real_python" > "$BATS_TEST_TMPDIR/linux-python/python3"
  cat >> "$BATS_TEST_TMPDIR/linux-python/python3" <<PY
import os,runpy,sys
args=sys.argv[1:]
if args and args[0]=='-B': args.pop(0)
if args and args[0].endswith(('/browser_api.py','/intent.py')):
    sys.platform='linux';sys.argv=args;sys.path.insert(0,os.path.dirname(args[0]));runpy.run_path(args[0],run_name='__main__')
else:
    os.execv('$real_python',['$real_python']+args)
PY
  chmod +x "$BATS_TEST_TMPDIR/linux-python/python3"
  export PATH="$BATS_TEST_TMPDIR/linux-python:$PATH"
  mixed_adopt_folder_round linux
}
@test "Adopt folder selects exact current IDs and shows partial selection without future opt-in" {
  adopt_fixture
  printf 'template {{ .chezmoi.os }}\n' > "$DOT_SOURCE/home/dot_config/zextra.conf.tmpl"
  python3 - "$DOT_SOURCE/tools/dot/manifest.json" <<'PY'
import json,sys
path=sys.argv[1]; catalog=json.load(open(path)); entry=dict(catalog['entries']['prefs'])
entry.update(destination='.config/zextra.conf',source='home/dot_config/zextra.conf.tmpl',portability='managed-templated',impact='Unsafe')
catalog['entries']['extra']=entry
json.dump(catalog,open(path,'w'))
PY
  git -C "$DOT_SOURCE" add .; git -C "$DOT_SOURCE" commit -qm 'second eligible template'
  start_browser --mode adopt
  wait_for ADOPT
  tmux -L "$SOCKET" send-keys -t ui H
  wait_for '.config'
  tmux -L "$SOCKET" send-keys -t ui Space
  wait_for '2 selected'
  [[ "$(pane)" == *'[x] .config'* ]]
  tmux -L "$SOCKET" send-keys -t ui l j Space
  wait_for '1 selected'
  [[ "$(pane)" == *'[-] .config'* ]]
  printf 'future opt-in\n' > "$DOT_SOURCE/home/dot_config/future.conf"
  python3 - "$DOT_SOURCE/tools/dot/manifest.json" <<'PY'
import json,sys
path=sys.argv[1]; catalog=json.load(open(path)); entry=dict(catalog['entries']['prefs'])
entry.update(destination='.config/future.conf',source='home/dot_config/future.conf')
catalog['entries']['future']=entry
json.dump(catalog,open(path,'w'))
PY
  git -C "$DOT_SOURCE" add .; git -C "$DOT_SOURCE" commit -qm 'future remains opt-in'
  printf '{"version":1,"mode":"adopt","profile":"default","entries":["extra"]}\n' > "$BATS_TEST_TMPDIR/request"
  "$DOT_ENGINE/dot" intent plan "$BATS_TEST_TMPDIR/request" --out "$BATS_TEST_TMPDIR/plan"
  token="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["id"])' "$BATS_TEST_TMPDIR/plan")"
  tmux -L "$SOCKET" send-keys -t ui / zextra Enter
  wait_for '1 selected'
  tmux -L "$SOCKET" send-keys -t ui p
  wait_for 'REVIEW EXACT PLAN'
  [[ "$(pane)" == *"$token"* ]]
  [[ "$(pane)" == *'Unsafe'* ]]
  [ "$(cat "$HOME/.config/preferences.conf")" = 'target theme' ]
  tmux -L "$SOCKET" send-keys -t ui q q
  wait_for UI_DONE
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
  [ ! -e "$HOME/.config/zextra.conf" ]
  [ ! -e "$HOME/.config/future.conf" ]
}

@test "80x24 tree scroll keeps the active fifth file visible and retains basket" {
  mkdir -p "$HOME/configs"
  for name in f00 f01 f02 f03 f04 f05 f06 f07 f08 f09; do printf 'fixture\n' > "$HOME/configs/$name"; done
  start_browser --mode track
  wait_for configs
  tmux -L "$SOCKET" send-keys -t ui Space
  wait_for '10 selected'
  tmux -L "$SOCKET" send-keys -t ui l
  wait_for f00
  tmux -L "$SOCKET" resize-window -t ui -x 80 -y 24
  tmux -L "$SOCKET" send-keys -t ui j j j j j
  sleep 0.4
  [[ "$(pane)" == *'>[x] configs/f04'* ]]
  [[ "$(pane)" == *'10 selected'* ]]
  tmux -L "$SOCKET" send-keys -t ui / f08 Enter
  wait_for '>[x] configs/f08'
  [[ "$(pane)" == *'10 selected'* ]]
  tmux -L "$SOCKET" send-keys -t ui J K
  [[ "$(pane)" == *'10 selected'* ]]
  tmux -L "$SOCKET" resize-window -t ui -x 120 -y 40
  wait_for '10 selected'
  tmux -L "$SOCKET" send-keys -t ui q
  wait_for UI_DONE
}
@test "optional browser rounds default Finish and leave fixture source target scope unchanged" {
  before="$(git -C "$DOT_SOURCE" rev-parse HEAD)"
  start_browser --rounds
  wait_for Finish
  tmux -L "$SOCKET" send-keys -t ui Enter
  wait_for UI_DONE
  pane >&3
  [[ "$(pane)" == *'UI_EXIT=0'* ]]
  [ "$(git -C "$DOT_SOURCE" rev-parse HEAD)" = "$before" ]
  [ -z "$(git -C "$DOT_SOURCE" status --porcelain)" ]
  [ ! -e "$HOME/.local/state/dot" ]
}
@test "Adopt live preview names repository and target without mutating either side" {
  adopt_fixture
  start_browser --mode adopt
  wait_for 'ADOPT'
  tmux -L "$SOCKET" send-keys -t ui H
  wait_for '.config'
  tmux -L "$SOCKET" send-keys -t ui l
  wait_for 'preferences.conf'
  tmux -L "$SOCKET" send-keys -t ui j
  wait_for 'repository theme'
  [[ "$(pane)" == *'TARGET'* ]]
  [[ "$(pane)" == *'REPOSITORY'* ]]
  pane >&3
  [[ "$(pane)" == *'mode 0644 → 0644'* ]]
  tmux -L "$SOCKET" resize-window -t ui -x 80 -y 24
  wait_for 'repository theme'
  [[ "$(pane)" == *'REPOSITORY'* ]]
  [[ "$(pane)" == *'TARGET'* ]]
  tmux -L "$SOCKET" resize-window -t ui -x 120 -y 40
  wait_for 'repository theme'
  [[ "$(pane)" == *'REPOSITORY'* ]]
  [[ "$(pane)" == *'TARGET'* ]]
  [ "$(cat "$HOME/.config/preferences.conf")" = 'target theme' ]
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
  tmux -L "$SOCKET" send-keys -t ui q
  wait_for UI_DONE
  [ -z "$(git -C "$DOT_SOURCE" status --porcelain)" ]
}
@test "browser exact review matches CLI plan and cancellation preserves target and approval" {
  adopt_fixture
  printf '{"version":1,"mode":"adopt","profile":"default","entries":["prefs"]}\n' > "$BATS_TEST_TMPDIR/request"
  "$DOT_ENGINE/dot" intent plan "$BATS_TEST_TMPDIR/request" --out "$BATS_TEST_TMPDIR/plan"
  token="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["id"])' "$BATS_TEST_TMPDIR/plan")"
  start_browser --mode adopt
  wait_for ADOPT
  tmux -L "$SOCKET" send-keys -t ui H
  wait_for '.config'
  tmux -L "$SOCKET" send-keys -t ui l
  wait_for preferences.conf
  tmux -L "$SOCKET" send-keys -t ui j Space
  wait_for '1 selected'
  tmux -L "$SOCKET" send-keys -t ui p
  wait_for 'REVIEW EXACT PLAN'
  [[ "$(pane)" == *"$token"* ]]
  [ "$(cat "$HOME/.config/preferences.conf")" = 'target theme' ]
  tmux -L "$SOCKET" send-keys -t ui q
  wait_for 'canceled'
  tmux -L "$SOCKET" send-keys -t ui q
  wait_for UI_DONE
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
  [ -z "$(git -C "$DOT_SOURCE" status --porcelain)" ]
}
@test "Track folder snapshot basket survives expansion search and hidden toggles" {
  mkdir -p "$HOME/configs"
  printf 'first fixture\n' > "$HOME/configs/first.conf"
  printf 'second fixture\n' > "$HOME/configs/second.conf"
  printf 'fake local credential\n' > "$HOME/configs/.env"
  start_browser --mode track
  wait_for TRACK
  wait_for configs
  tmux -L "$SOCKET" send-keys -t ui Space
  wait_for '2 selected'
  printf 'future opt-in fixture\n' > "$HOME/configs/later.conf"
  tmux -L "$SOCKET" send-keys -t ui l j
  wait_for first.conf
  [[ "$(pane)" == *'2 selected'* ]]
  tmux -L "$SOCKET" send-keys -t ui Space
  wait_for '1 selected'
  [[ "$(pane)" == *'[-]'* ]]
  tmux -L "$SOCKET" send-keys -t ui / second Enter
  wait_for second.conf
  [[ "$(pane)" == *'1 selected'* ]]
  tmux -L "$SOCKET" send-keys -t ui Escape H
  wait_for '1 selected'
  tmux -L "$SOCKET" send-keys -t ui q
  wait_for UI_DONE
  [ ! -e "$DOT_SOURCE/home/configs" ]
  [ ! -e "$HOME/.local/state/dot" ]
}
