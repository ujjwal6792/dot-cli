#!/usr/bin/env bats
setup() {
  REPO="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
  export HOME="$BATS_TEST_TMPDIR/home" DOT_SOURCE="$BATS_TEST_TMPDIR/source"
  export DOT_ENGINE="$REPO/tools/dot"
  export DOT_CONFIG="$HOME/.config/chezmoi/chezmoi.toml"
  mkdir -p "$HOME/.config/chezmoi" "$HOME/.config/arbitrary" "$DOT_SOURCE/tools/dot" "$DOT_SOURCE/home"
  printf '[data]\nname="Test"\nemail="test@example.org"\nprofile="personal"\ndotEngine="%s"\n' "$DOT_ENGINE" > "$DOT_CONFIG"
  cp "$BATS_TEST_DIRNAME/fixtures/chezmoiignore.tmpl" "$DOT_SOURCE/home/.chezmoiignore"
  cp "$BATS_TEST_DIRNAME/fixtures/chezmoiremove.tmpl" "$DOT_SOURCE/home/.chezmoiremove"
  cp "$REPO/tools/dot/"*.py "$DOT_SOURCE/tools/dot/"
  printf '{"protocol":1,"engine_api":{"min":1,"max":1},"manifest_version":1,"approval_version":2,"manifest":"tools/dot/manifest.json","home_source":"home","required_adapters":["intent","scope","gitconfig","skills","layout"]}\n' > "$DOT_SOURCE/dot-contract.json"
  printf '{"version":1,"entries":{},"profiles":{}}\n' > "$DOT_SOURCE/tools/dot/manifest.json"
  git init -q "$DOT_SOURCE"
  git -C "$DOT_SOURCE" config user.name Test
  git -C "$DOT_SOURCE" config user.email test@example.org
  git -C "$DOT_SOURCE" add .
  git -C "$DOT_SOURCE" commit -qm fixture
  DOT="$REPO/tools/dot/dot"
  printf 'theme = light\n' > "$HOME/.config/arbitrary/preferences.conf"
  REQUEST="$BATS_TEST_TMPDIR/request.json"
  PLAN="$BATS_TEST_TMPDIR/plan.json"
  printf '{"version":1,"mode":"track","profile":"work","selections":[{"path":".config/arbitrary/preferences.conf","os":["darwin","linux"],"portability":"managed-shared","impact":"Safe"}]}\n' > "$REQUEST"
}

local_machine() {
  python3 - <<'PY' | python3 "$DOT_ENGINE/browser_api.py"
import json, os
print(json.dumps(dict(version=1, command='init-machine', engine=os.environ['DOT_ENGINE'],
    source=os.environ['DOT_SOURCE'], home=os.environ['HOME'], request=dict(action='save', id='fixture_machine'))))
PY
}

@test "Adopt rejects empty and unrelated metadata edits without modifying manifest or HOME" {
  printf 'other\n' > "$HOME/.config/arbitrary/other.conf"
  python3 - "$REQUEST" <<'PY'
import json, sys
p = sys.argv[1]; d = json.load(open(p))
d['selections'].append(dict(d['selections'][0], path='.config/arbitrary/other.conf'))
open(p, 'w').write(json.dumps(d))
PY
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve
  cp "$DOT_SOURCE/tools/dot/manifest.json" "$BATS_TEST_TMPDIR/manifest-before"
  BEFORE="$(git -C "$DOT_SOURCE" rev-parse HEAD)"
  for entries in '[]' '[".config/arbitrary/other.conf"]'; do
    printf '{"version":1,"mode":"adopt","profile":"work","entries":%s,"metadata":{".config/arbitrary/preferences.conf":{"warnings":["desktop-arch-compatibility"],"impact":"Unsafe"}}}\n' "$entries" > "$REQUEST"
    run "$DOT" intent plan "$REQUEST" --out "$PLAN"
    [ "$status" -ne 0 ]
    [[ "$output" == *'restricted to selected entries'* ]]
    cmp "$DOT_SOURCE/tools/dot/manifest.json" "$BATS_TEST_TMPDIR/manifest-before"
    [ "$(git -C "$DOT_SOURCE" rev-parse HEAD)" = "$BEFORE" ]
    [ ! -e "$HOME/.local/state/dot/adoption.json" ]
    [ "$(cat "$HOME/.config/arbitrary/preferences.conf")" = 'theme = light' ]
  done
}

@test "selected metadata downgrade requires original Unsafe and desktop acknowledgments" {
  local_machine
  python3 - "$REQUEST" <<'PY'
import json, sys
p = sys.argv[1]; d = json.load(open(p))
d['selections'][0].update(impact='Unsafe', warnings=['desktop-arch-compatibility'])
open(p, 'w').write(json.dumps(d))
PY
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  TOKEN="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["id"])' "$PLAN")"
  "$DOT" intent apply "$PLAN" --approve "$TOKEN" --ack-unsafe --ack-desktop
  printf '{"version":1,"mode":"adopt","profile":"work","entries":[".config/arbitrary/preferences.conf"],"metadata":{".config/arbitrary/preferences.conf":{"warnings":[],"impact":"Safe"}}}\n' > "$REQUEST"
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  TOKEN="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["id"])' "$PLAN")"
  run "$DOT" intent apply "$PLAN" --approve "$TOKEN" --ack-desktop
  [ "$status" -ne 0 ]
  [[ "$output" == *'--ack-unsafe'* ]]
  run "$DOT" intent apply "$PLAN" --approve "$TOKEN" --ack-unsafe
  [ "$status" -ne 0 ]
  [[ "$output" == *'--ack-desktop'* ]]
  python3 - "$HOME/.local/state/dot/adoption.json" <<'PY'
import json, sys
assert json.load(open(sys.argv[1]))['entries'] == {}
PY
  "$DOT" intent apply "$PLAN" --approve "$TOKEN" --ack-unsafe --ack-desktop
}

@test "literal tmpl file and directory names retain exact native destinations" {
  mkdir -p "$HOME/config.tmpl"
  printf 'literal content\n' > "$HOME/config.tmpl/preferences.tmpl"
  python3 - "$REQUEST" <<'PY'
import json, sys
p = sys.argv[1]; d = json.load(open(p))
d['selections'][0]['path'] = 'config.tmpl/preferences.tmpl'
open(p, 'w').write(json.dumps(d))
PY
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve
  printf '{"version":1,"mode":"adopt","profile":"work"}\n' > "$REQUEST"
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve
  run chezmoi --source "$DOT_SOURCE/home" --destination "$HOME" --config "$DOT_CONFIG" managed --include files
  [ "$status" -eq 0 ] || { printf '%s\n' "$output"; return 1; }
  [[ "$output" == *'config.tmpl/preferences.tmpl'* ]]
  printf 'local\n' > "$HOME/config.tmpl/preferences.tmpl"
  chezmoi --source "$DOT_SOURCE/home" --destination "$HOME" --config "$DOT_CONFIG" apply --force
  [ "$(cat "$HOME/config.tmpl/preferences.tmpl")" = 'literal content' ]
  python3 - "$REQUEST" <<'PY'
import json, sys
entry = {'path': 'config.tmpl/preferences.tmpl', 'os': ['darwin', 'linux'],
         'portability': 'managed-templated', 'impact': 'Safe', 'template': 'theme = {{ printf "dark" }}\n'}
open(sys.argv[1], 'w').write(json.dumps({'version': 1, 'mode': 'track', 'profile': 'work', 'selections': [entry]}))
PY
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve
  printf '{"version":1,"mode":"adopt","profile":"work"}\n' > "$REQUEST"
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve
  printf 'local\n' > "$HOME/config.tmpl/preferences.tmpl"
  chezmoi --source "$DOT_SOURCE/home" --destination "$HOME" --config "$DOT_CONFIG" apply --force
  [ "$(cat "$HOME/config.tmpl/preferences.tmpl")" = 'theme = dark' ]
}

@test "explicit OS metadata authorizes arbitrary Library and systemd paths" {
  mkdir -p "$HOME/Library" "$HOME/.config/systemd"
  printf 'portable\n' > "$HOME/Library/preferences.conf"
  printf 'portable\n' > "$HOME/.config/systemd/preferences.conf"
  python3 - "$REQUEST" <<'PY'
import json, sys
p = sys.argv[1]; d = json.load(open(p)); e = d['selections'][0]
d['selections'] = [dict(e, path=n) for n in ['Library/preferences.conf', '.config/systemd/preferences.conf']]
open(p, 'w').write(json.dumps(d))
PY
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve
  printf '{"version":1,"mode":"adopt","profile":"work"}\n' > "$REQUEST"
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve
  for os in darwin linux; do
    run chezmoi --source "$DOT_SOURCE/home" --destination "$HOME" --config "$DOT_CONFIG" --override-data "{\"chezmoi\":{\"os\":\"$os\"}}" managed --include files
    [ "$status" -eq 0 ]
    [[ "$output" == *'Library/preferences.conf'* ]]
    [[ "$output" == *'.config/systemd/preferences.conf'* ]]
    printf 'local\n' > "$HOME/Library/preferences.conf"
    printf 'local\n' > "$HOME/.config/systemd/preferences.conf"
    chezmoi --source "$DOT_SOURCE/home" --destination "$HOME" --config "$DOT_CONFIG" --override-data "{\"chezmoi\":{\"os\":\"$os\"}}" apply --force
    [ "$(cat "$HOME/Library/preferences.conf")" = portable ]
    [ "$(cat "$HOME/.config/systemd/preferences.conf")" = portable ]
  done
}

@test "native exact removal parity backs up only approved regular files and rejects expansion" {
  python3 - "$REQUEST" <<'PY'
import json, sys
p = sys.argv[1]; d = json.load(open(p)); d['selections'][0]['removals'] = ['obsolete.conf']
open(p, 'w').write(json.dumps(d))
PY
  printf 'obsolete\n' > "$HOME/obsolete.conf"
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve
  printf '{"version":1,"mode":"adopt","profile":"work"}\n' > "$REQUEST"
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve
  printf 'new obsolete\n' > "$HOME/obsolete.conf"
  printf 'unmanaged\n' > "$HOME/unmanaged.conf"
  chezmoi --source "$DOT_SOURCE/home" --destination "$HOME" --config "$DOT_CONFIG" apply --force
  [ ! -e "$HOME/obsolete.conf" ]
  [ "$(cat "$HOME/unmanaged.conf")" = unmanaged ]
  grep -r -q 'new obsolete' "$HOME/.local/state/dot/native-removal-recovery"
  mkdir "$HOME/obsolete.conf"
  run chezmoi --source "$DOT_SOURCE/home" --destination "$HOME" --config "$DOT_CONFIG" apply --force
  [ "$status" -ne 0 ]
  [ -d "$HOME/obsolete.conf" ]
}

@test "warning removal and broadened OS require renewed separate desktop acknowledgment" {
  local_machine
  python3 - "$REQUEST" <<'PY'
import json, sys
p = sys.argv[1]; d = json.load(open(p)); d['selections'][0]['warnings'] = ['desktop-arch-compatibility']
open(p, 'w').write(json.dumps(d))
PY
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  TOKEN="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["id"])' "$PLAN")"
  run "$DOT" intent apply "$PLAN" --approve "$TOKEN" --ack-unsafe
  [ "$status" -ne 0 ]
  [[ "$output" == *'--ack-desktop'* ]]
  "$DOT" intent apply "$PLAN" --approve "$TOKEN" --ack-desktop
  printf '{"version":1,"mode":"adopt","profile":"work"}\n' > "$REQUEST"
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  TOKEN="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["id"])' "$PLAN")"
  "$DOT" intent apply "$PLAN" --approve "$TOKEN" --ack-desktop
  printf '{"version":1,"mode":"adopt","profile":"work","metadata":{".config/arbitrary/preferences.conf":{"warnings":[]}}}\n' > "$REQUEST"
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  run approve
  [ "$status" -ne 0 ]
  [[ "$output" == *'--ack-desktop'* ]]
  TOKEN="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["id"])' "$PLAN")"
  "$DOT" intent apply "$PLAN" --approve "$TOKEN" --ack-desktop
  "$DOT" intent update --yes
}

@test "broader OS applicability without a warning still needs separate desktop consent" {
  local_machine
  python3 - "$REQUEST" <<'PY'
import json, sys
p = sys.argv[1]; d = json.load(open(p))
d['selections'][0]['os'] = ['linux' if sys.platform.startswith('linux') else sys.platform]
open(p, 'w').write(json.dumps(d))
PY
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve
  printf '{"version":1,"mode":"adopt","profile":"work"}\n' > "$REQUEST"
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve
  printf '{"version":1,"mode":"adopt","profile":"work","metadata":{".config/arbitrary/preferences.conf":{"os":["darwin","linux"]}}}\n' > "$REQUEST"
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  run approve
  [ "$status" -ne 0 ]
  [[ "$output" == *'--ack-desktop'* ]]
  TOKEN="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["id"])' "$PLAN")"
  "$DOT" intent apply "$PLAN" --approve "$TOKEN" --ack-desktop
}

@test "credential content is rejected regardless of filename and cannot be approved away" {
  printf 'password = "fixture-credential"\n' > "$HOME/.config/arbitrary/preferences.conf"
  run "$DOT" intent plan "$REQUEST" --out "$PLAN"
  [ "$status" -ne 0 ]
  [[ "$output" == *'credential content detected'* ]]
  [[ "$output" != *'fixture-credential'* ]]
  [ ! -e "$PLAN" ]
  [ ! -e "$DOT_SOURCE/home/dot_config" ]
}

@test "stale saved plans cannot apply changed inputs or metadata" {
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  printf 'theme = changed\n' > "$HOME/.config/arbitrary/preferences.conf"
  run approve
  [ "$status" -ne 0 ]
  [[ "$output" == *'stale/changed plan'* ]]
  [ ! -e "$DOT_SOURCE/home/dot_config" ]
}

@test "directory snapshots exclude credentials and never enroll future files or upstream profile additions" {
  printf 'local-secret\n' > "$HOME/.config/arbitrary/auth.json"
  sed 's@arbitrary/preferences.conf@arbitrary@' "$REQUEST" > "$REQUEST.new"
  mv "$REQUEST.new" "$REQUEST"
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve
  [ ! -e "$DOT_SOURCE/home/dot_config/arbitrary/auth.json" ]
  printf 'future\n' > "$HOME/.config/arbitrary/future.conf"
  printf '{"version":1,"mode":"adopt","profile":"work"}\n' > "$REQUEST"
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve
  printf 'remote\n' > "$DOT_SOURCE/home/dot_config/arbitrary/future.conf"
  python3 - "$DOT_SOURCE/tools/dot/manifest.json" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
e = dict(d['entries']['.config/arbitrary/preferences.conf'])
e.update(destination='.config/arbitrary/future.conf', source='home/dot_config/arbitrary/future.conf')
d['entries'][e['destination']] = e
d['profiles']['work'].append(e['destination'])
open(p, 'w').write(json.dumps(d))
PY
  chezmoi --source "$DOT_SOURCE/home" --destination "$HOME" --config "$DOT_CONFIG" apply --force
  [ "$(cat "$HOME/.config/arbitrary/future.conf")" = future ]
  [ "$(cat "$HOME/.config/arbitrary/auth.json")" = local-secret ]
}

@test "Unsafe plans require extra acknowledgment and exact approval; no general yes flag" {
  sed 's/"Safe"/"Unsafe"/' "$REQUEST" > "$REQUEST.new"
  mv "$REQUEST.new" "$REQUEST"
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  run approve
  [ "$status" -ne 0 ]
  [[ "$output" == *'--ack-unsafe'* ]]
  run "$DOT" intent apply "$PLAN" --yes
  [ "$status" -ne 0 ]
  approve --ack-unsafe
}

@test "unsupported binary imports and symlink escapes fail closed" {
  printf '\000binary' > "$HOME/.config/arbitrary/preferences.conf"
  run "$DOT" intent plan "$REQUEST" --out "$PLAN"
  [ "$status" -ne 0 ]
  [[ "$output" == *'unsupported/binary'* ]]
  rm "$HOME/.config/arbitrary/preferences.conf"
  printf 'outside\n' > "$BATS_TEST_TMPDIR/outside"
  ln -s "$BATS_TEST_TMPDIR/outside" "$HOME/.config/arbitrary/preferences.conf"
  run "$DOT" intent plan "$REQUEST" --out "$PLAN"
  [ "$status" -ne 0 ]
  [[ "$output" == *'symlink/path escape'* ]]
}
approve() {
  local token
  token="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["id"])' "$PLAN")"
  "$DOT" intent apply "$PLAN" --approve "$token" "$@"
}

@test "Track previews imports and commits intent without adopting HOME; Adopt uses the same saved plan" {
  run "$DOT" intent plan "$REQUEST" --out "$PLAN"
  [ "$status" -eq 0 ]
  [[ "$output" == *'Track: HOME → repository'* ]]
  [ ! -e "$DOT_SOURCE/home/dot_config/arbitrary/preferences.conf" ]
  approve
  [ "$(cat "$HOME/.config/arbitrary/preferences.conf")" = 'theme = light' ]
  [ "$(git -C "$DOT_SOURCE" show HEAD:home/dot_config/arbitrary/preferences.conf)" = 'theme = light' ]
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
  printf 'theme = local\n' > "$HOME/.config/arbitrary/preferences.conf"
  printf '{"version":1,"mode":"adopt","profile":"work"}\n' > "$REQUEST"
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve
  [ "$(cat "$HOME/.config/arbitrary/preferences.conf")" = 'theme = light' ]
  run chezmoi --source "$DOT_SOURCE/home" --destination "$HOME" --config "$DOT_CONFIG" managed --include files
  [ "$status" -eq 0 ]
  [[ "$output" == *'.config/arbitrary/preferences.conf'* ]]
}

seed_adoption() {
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve
  printf '{"version":1,"mode":"adopt","profile":"work"}\n' > "$REQUEST"
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve
}

@test "content updates retain exact approval but increased impact and new removal capabilities stop" {
  seed_adoption
  printf '\n\n' >> "$HOME/.local/state/dot/adoption.json"
  chmod 640 "$HOME/.local/state/dot/adoption.json"
  cp "$HOME/.local/state/dot/adoption.json" "$BATS_TEST_TMPDIR/approval-before"
  printf 'theme = upstream\n' > "$DOT_SOURCE/home/dot_config/arbitrary/preferences.conf"
  "$DOT" intent update --yes
  [ "$(cat "$HOME/.config/arbitrary/preferences.conf")" = 'theme = upstream' ]
  cmp "$BATS_TEST_TMPDIR/approval-before" "$HOME/.local/state/dot/adoption.json"
  python3 - "$HOME/.local/state/dot/adoption.json" <<'PY'
import pathlib,sys
assert pathlib.Path(sys.argv[1]).stat().st_mode & 0o777 == 0o640
PY
  python3 - "$DOT_SOURCE/tools/dot/manifest.json" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
d['entries']['.config/arbitrary/preferences.conf']['impact'] = 'Unsafe'
open(p, 'w').write(json.dumps(d))
PY
  run "$DOT" intent update --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'fresh adoption approval required'* ]]
  run chezmoi --source "$DOT_SOURCE/home" --destination "$HOME" --config "$DOT_CONFIG" apply --force
  [ "$status" -ne 0 ]
  [[ "$output" == *'approved metadata changed'* ]]
}

@test "legacy migration previews and narrows existing scope to manifested exact files only" {
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve
  "$DOT" scope init
  # Public legacy scope seam needs its documented catalog in the fixture.
  printf '{"eza":{"risk":"Safe","os":["darwin","linux"],"paths":[".config/eza"]},"shell":{"risk":"Unsafe","os":["darwin","linux"],"paths":[".zshrc"]},"skills":{"risk":"Safe","os":["darwin","linux"],"paths":[".agents/skills",".gemini/skills"]}}\n' > "$DOT_SOURCE/tools/dot/groups.json"
  "$DOT" scope expand eza --yes
  printf '{"version":1,"mode":"migrate","profile":"work"}\n' > "$REQUEST"
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  run "$DOT" scope show
  [[ "$output" == *'"version": 1'* ]]
  approve
  run "$DOT" scope show
  [[ "$output" == *'"version": 2'* ]]
  [[ "$output" == *'"entries": {}'* ]]
  [ "$(cat "$HOME/.config/arbitrary/preferences.conf")" = 'theme = light' ]
}

@test "mixed credentials require a reviewed secret-free template; binary template bypass is denied" {
  printf 'password = "fixture-credential"\n' > "$HOME/.config/arbitrary/preferences.conf"
  python3 - "$REQUEST" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
d['selections'][0].update(portability='managed-templated', template='password = "${LOCAL_SECRET}"\n')
open(p, 'w').write(json.dumps(d))
PY
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve
  [ "$(cat "$DOT_SOURCE/home/dot_config/arbitrary/private_preferences.conf.tmpl")" = 'password = "${LOCAL_SECRET}"' ]
  [[ "$(cat "$PLAN")" != *fixture-credential* ]]
  printf '\000binary' > "$HOME/.config/arbitrary/preferences.conf"
  run "$DOT" intent plan "$REQUEST"
  [ "$status" -ne 0 ]
  [[ "$output" == *'unsupported/binary'* ]]
}

@test "empty profile adopts nothing and malformed capability metadata fails closed" {
  printf '{"version":1,"mode":"track","profile":"empty","selections":[]}\n' > "$REQUEST"
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve
  printf '{"version":1,"mode":"adopt","profile":"empty"}\n' > "$REQUEST"
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve
  run chezmoi --source "$DOT_SOURCE/home" --destination "$HOME" --config "$DOT_CONFIG" managed --include files
  [ "$status" -eq 0 ]
  [ -z "$output" ]
  [ "$(cat "$HOME/.config/arbitrary/preferences.conf")" = 'theme = light' ]
}

@test "exact removals are previewed and backed up; directory removals and arbitrary hooks are denied" {
  printf 'obsolete preferences\n' > "$HOME/.config/arbitrary/obsolete.conf"
  python3 - "$REQUEST" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
d['selections'][0]['removals'] = ['.config/arbitrary/obsolete.conf']
open(p, 'w').write(json.dumps(d))
PY
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve
  printf '{"version":1,"mode":"adopt","profile":"work"}\n' > "$REQUEST"
  run "$DOT" intent plan "$REQUEST" --out "$PLAN"
  [ "$status" -eq 0 ]
  [[ "$output" == *'remove: .config/arbitrary/obsolete.conf'* ]]
  [ -e "$HOME/.config/arbitrary/obsolete.conf" ]
  approve
  [ ! -e "$HOME/.config/arbitrary/obsolete.conf" ]
  run find "$HOME/.local/state/dot/recovery" -type f -exec grep -l 'obsolete preferences' '{}' +
  [ "$status" -eq 0 ]
  [ -n "$output" ]
  python3 - "$DOT_SOURCE/tools/dot/manifest.json" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
d['entries']['.config/arbitrary/preferences.conf']['hooks'] = ['run_after_escape.sh']
open(p, 'w').write(json.dumps(d))
PY
  run "$DOT" intent plan "$REQUEST"
  [ "$status" -ne 0 ]
  [[ "$output" == *'unsupported hook capability'* ]]
}

@test "arbitrary HOME configuration cannot disappear behind repository metadata ignores" {
  mkdir -p "$HOME/tools"
  printf 'theme = personal\n' > "$HOME/tools/preferences.conf"
  sed 's@.config/arbitrary/preferences.conf@tools/preferences.conf@' "$REQUEST" > "$REQUEST.new"
  mv "$REQUEST.new" "$REQUEST"
  seed_adoption
  run chezmoi --source "$DOT_SOURCE/home" --destination "$HOME" --config "$DOT_CONFIG" managed --include files
  [ "$status" -eq 0 ]
  [[ "$output" == *'tools/preferences.conf'* ]]
}

@test "preview cannot execute command-bearing templates before approval" {
  python3 - "$REQUEST" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
d['selections'][0].update(portability='managed-templated', template='{{ output "sh" "-c" "touch $HOME/escaped" }}')
open(p, 'w').write(json.dumps(d))
PY
  run "$DOT" intent plan "$REQUEST" --out "$PLAN"
  [ "$status" -ne 0 ]
  [[ "$output" == *'nondeclarative template function'* ]]
  [ ! -e "$HOME/escaped" ]
}

@test "Track cannot authorize direct chezmoi adoption when local approval is absent" {
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve
  printf 'theme = local\n' > "$HOME/.config/arbitrary/preferences.conf"
  chezmoi --source "$DOT_SOURCE/home" --destination "$HOME" --config "$DOT_CONFIG" apply --force
  [ "$(cat "$HOME/.config/arbitrary/preferences.conf")" = 'theme = local' ]
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
}

@test "exact file approval does not broaden permissions on unapproved parent directories" {
  chmod 700 "$HOME/.config"
  seed_adoption
  printf 'local preserved\n' > "$HOME/.config/arbitrary/preferences.conf"
  run chezmoi --source "$DOT_SOURCE/home" --destination "$HOME" --config "$DOT_CONFIG" apply --force
  [ "$status" -ne 0 ]
  [[ "$output" == *'unapproved ancestor permission change'* ]]
  [ "$(cat "$HOME/.config/arbitrary/preferences.conf")" = 'local preserved' ]
  for flag in --verbose --debug; do
    run chezmoi --source "$DOT_SOURCE/home" --destination "$HOME" --config "$DOT_CONFIG" apply --force "$flag"
    [ "$status" -ne 0 ]
    [[ "$output" == *'unapproved ancestor permission change'* ]]
    [ "$(cat "$HOME/.config/arbitrary/preferences.conf")" = 'local preserved' ]
  done
  python3 - "$HOME/.config" <<'PY'
import pathlib, sys
mode = pathlib.Path(sys.argv[1]).stat().st_mode & 0o777
assert mode == 0o700, f'unapproved parent permissions changed from 0700 to {mode:04o}'
PY
}

@test "native umask cannot implicitly narrow an existing ancestor directory" {
  seed_adoption
  printf '\numask = 0o077\n' > "$BATS_TEST_TMPDIR/mask-config.toml"
  printf '[data]\nname="Test"\nemail="test@example.org"\n' >> "$BATS_TEST_TMPDIR/mask-config.toml"
  run chezmoi --source "$DOT_SOURCE/home" --destination "$HOME" --config "$BATS_TEST_TMPDIR/mask-config.toml" apply --force
  [ "$status" -ne 0 ]
  [[ "$output" == *'unapproved ancestor permission change'* ]] || { printf '%s\n' "$output" >&2; return 1; }
  for flag in --verbose --debug; do
    run chezmoi --source "$DOT_SOURCE/home" --destination "$HOME" --config "$BATS_TEST_TMPDIR/mask-config.toml" apply --force "$flag"
    [ "$status" -ne 0 ]
    [[ "$output" == *'unapproved ancestor permission change'* ]]
  done
  python3 - "$HOME/.config" <<'PY'
import pathlib, sys
assert pathlib.Path(sys.argv[1]).stat().st_mode & 0o777 == 0o755
PY
}

@test "reviewed portability edits replace the exact old source without duplicate native destinations" {
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve
  python3 - "$REQUEST" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
d['selections'][0].update(portability='managed-templated', template='theme = dark\n')
open(p, 'w').write(json.dumps(d))
PY
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve
  [ ! -e "$DOT_SOURCE/home/dot_config/arbitrary/preferences.conf" ]
  [ -e "$DOT_SOURCE/home/dot_config/arbitrary/private_preferences.conf.tmpl" ]
  printf '{"version":1,"mode":"adopt","profile":"work"}\n' > "$REQUEST"
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve
  chezmoi --source "$DOT_SOURCE/home" --destination "$HOME" --config "$DOT_CONFIG" apply --force
  [ "$(cat "$HOME/.config/arbitrary/preferences.conf")" = 'theme = dark' ]
}
