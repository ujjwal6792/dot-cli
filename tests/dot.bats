#!/usr/bin/env bats
setup() {
  REPO="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
  export HOME="$BATS_TEST_TMPDIR/home" DOT_SOURCE="$BATS_TEST_TMPDIR/source"
  export DOT_ENGINE="$REPO/tools/dot"
  mkdir -p "$HOME" "$DOT_SOURCE/home" "$DOT_SOURCE/tools/dot"
  mkdir -p "$HOME/.config/chezmoi"
  export DOT_CONFIG="$HOME/.config/chezmoi/chezmoi.toml"
  printf '[data]\nname="Test"\nemail="test@example.org"\nprofile="personal"\ndotEngine="%s"\n' "$DOT_ENGINE" > "$DOT_CONFIG"
  printf '{"protocol":1,"engine_api":{"min":1,"max":1},"manifest_version":1,"approval_version":2,"manifest":"tools/dot/manifest.json","home_source":"home","required_adapters":["intent","scope","gitconfig","skills","layout"]}\n' > "$DOT_SOURCE/dot-contract.json"
  printf '{"version":1,"entries":{},"profiles":{}}\n' > "$DOT_SOURCE/tools/dot/manifest.json"
  DOT="$BATS_TEST_DIRNAME/../tools/dot/dot"
  git init -q --bare "$BATS_TEST_TMPDIR/remote"
  git init -q "$DOT_SOURCE"
  git -C "$DOT_SOURCE" config user.email test@example.org
  git -C "$DOT_SOURCE" config user.name Test
  printf 'base\n' > "$DOT_SOURCE/home/dot_example"
  printf 'base\n' > "$HOME/.example"
  cp "$BATS_TEST_DIRNAME/fixtures/chezmoiignore.tmpl" "$DOT_SOURCE/home/.chezmoiignore"
  mkdir -p "$DOT_SOURCE/packages" "$DOT_SOURCE/scripts"
  printf '[pins.fixture]\nversion = "1.0.0"\ncheck = "fixture --version"\ninstall_darwin = "fixture install"\ninstall_linux = "fixture install"\n' > "$DOT_SOURCE/packages/pins.toml"
  printf '#!/bin/sh\necho pins-ok\nexit 0\n' > "$DOT_SOURCE/scripts/install-pins.sh"
  chmod +x "$DOT_SOURCE/scripts/install-pins.sh"
  git -C "$DOT_SOURCE" add .
  git -C "$DOT_SOURCE" commit -qm initial
  git -C "$DOT_SOURCE" remote add origin "$BATS_TEST_TMPDIR/remote"
  git -C "$DOT_SOURCE" push -qu origin HEAD
  chezmoi --source "$DOT_SOURCE/home" --destination "$HOME" --config "$DOT_CONFIG" apply --force
}
select_example() {
  printf '{"version":1,"mode":"track","profile":"example","selections":[{"path":".example","os":["darwin","linux"],"portability":"managed-shared","impact":"Safe"}]}\n' > "$BATS_TEST_TMPDIR/example-request.json"
  "$DOT" intent plan "$BATS_TEST_TMPDIR/example-request.json" --out "$BATS_TEST_TMPDIR/example-plan.json"
  token="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["id"])' "$BATS_TEST_TMPDIR/example-plan.json")"
  "$DOT" intent apply "$BATS_TEST_TMPDIR/example-plan.json" --approve "$token"
  printf '{"version":1,"mode":"adopt","profile":"example","entries":[".example"]}\n' > "$BATS_TEST_TMPDIR/example-adopt.json"
  "$DOT" intent plan "$BATS_TEST_TMPDIR/example-adopt.json" --out "$BATS_TEST_TMPDIR/example-adopt-plan.json"
  token="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["id"])' "$BATS_TEST_TMPDIR/example-adopt-plan.json")"
  "$DOT" intent apply "$BATS_TEST_TMPDIR/example-adopt-plan.json" --approve "$token"
}
track_file() {
  local path="$1" portability="$2" systems="$3"
  if [ "$portability" = managed-templated ]; then
    printf '{"version":1,"mode":"track","profile":"fixture","selections":[{"path":"%s","os":%s,"portability":"%s","impact":"Safe","template":"{{ .name }}"}]}\n' \
      "$path" "$systems" "$portability" > "$BATS_TEST_TMPDIR/track-request.json"
  else
    printf '{"version":1,"mode":"track","profile":"fixture","selections":[{"path":"%s","os":%s,"portability":"%s","impact":"Safe"}]}\n' \
      "$path" "$systems" "$portability" > "$BATS_TEST_TMPDIR/track-request.json"
  fi
  "$DOT" intent plan "$BATS_TEST_TMPDIR/track-request.json" --out "$BATS_TEST_TMPDIR/track-plan.json"
  local token
  token="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["id"])' "$BATS_TEST_TMPDIR/track-plan.json")"
  "$DOT" intent apply "$BATS_TEST_TMPDIR/track-plan.json" --approve "$token"
}
untrack_file() {
  local path="$1" token
  printf '{"version":1,"mode":"untrack","profile":"fixture","entries":["%s"]}\n' "$path" > "$BATS_TEST_TMPDIR/untrack-request.json"
  "$DOT" intent plan "$BATS_TEST_TMPDIR/untrack-request.json" --out "$BATS_TEST_TMPDIR/untrack-plan.json"
  token="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["id"])' "$BATS_TEST_TMPDIR/untrack-plan.json")"
  "$DOT" intent apply "$BATS_TEST_TMPDIR/untrack-plan.json" --approve "$token"
}

@test "push captures local changes and publishes only managed changes" {
  select_example
  printf 'local\n' > "$HOME/.example"
  printf 'unrelated\n' > "$DOT_SOURCE/home/notes"
  run "$DOT" push --message 'capture local'
  [ "$status" -eq 0 ]
  [ "$(cat "$DOT_SOURCE/home/dot_example")" = local ]
  [ "$(git --git-dir="$BATS_TEST_TMPDIR/remote" show HEAD:home/dot_example)" = local ]
  [ -f "$DOT_SOURCE/home/notes" ]
  run git --git-dir="$BATS_TEST_TMPDIR/remote" show HEAD:notes
  [ "$status" -ne 0 ]
}

@test "approved installed public launcher reaches the independent engine" {
  mkdir -p "$DOT_SOURCE/tools/dot" "$DOT_SOURCE/home/dot_local/bin"
  printf '#!/bin/sh\nexec "$DOT_ENGINE/dot" "$@"\n' > "$DOT_SOURCE/home/dot_local/bin/executable_dot"
  python3 - "$DOT_SOURCE/tools/dot/manifest.json" <<'PY'
import json, sys
p = sys.argv[1]
entry = {'destination': '.local/bin/dot', 'source': 'home/dot_local/bin/executable_dot',
         'os': ['darwin', 'linux'], 'portability': 'managed-shared', 'impact': 'Safe',
         'removals': [], 'hooks': [], 'tools': []}
json.dump({'version': 1, 'entries': {'.local/bin/dot': entry},
           'profiles': {'launcher': ['.local/bin/dot']}}, open(p, 'w'))
PY
  git -C "$DOT_SOURCE" add .
  git -C "$DOT_SOURCE" commit -qm 'synthetic public launcher entry'
  printf '{"version":1,"mode":"adopt","profile":"launcher","entries":[".local/bin/dot"]}\n' > "$BATS_TEST_TMPDIR/launcher-request.json"
  "$DOT" intent plan "$BATS_TEST_TMPDIR/launcher-request.json" --out "$BATS_TEST_TMPDIR/launcher-plan.json"
  token="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["id"])' "$BATS_TEST_TMPDIR/launcher-plan.json")"
  "$DOT" intent apply "$BATS_TEST_TMPDIR/launcher-plan.json" --approve "$token"
  chezmoi --source "$DOT_SOURCE/home" --destination "$HOME" --config "$DOT_CONFIG" apply --force
  [ -x "$HOME/.local/bin/dot" ]
  run "$HOME/.local/bin/dot" status
  [ "$status" -eq 0 ]
  [[ "$output" == *'drift: 0'* ]]
}

@test "legacy pull refuses uncontracted unscoped apply without changing source or target" {
  git clone -q "$BATS_TEST_TMPDIR/remote" "$BATS_TEST_TMPDIR/other"
  git -C "$BATS_TEST_TMPDIR/other" config user.email test@example.org
  git -C "$BATS_TEST_TMPDIR/other" config user.name Test
  printf 'remote\n' > "$BATS_TEST_TMPDIR/other/home/dot_example"
  git -C "$BATS_TEST_TMPDIR/other" commit -qam remote
  git -C "$BATS_TEST_TMPDIR/other" push -q
  before="$(git -C "$DOT_SOURCE" rev-parse HEAD)"
  run "$DOT" pull
  [ "$status" -ne 0 ]
  [ "$(cat "$HOME/.example")" = base ]
  printf 'local\n' > "$HOME/.example"
  run "$DOT" pull --yes
  [ "$status" -ne 0 ]
  [ "$(cat "$HOME/.example")" = local ]
  printf 'base\n' > "$HOME/.example"
  run "$DOT" pull --yes
  [ "$status" -ne 0 ]
  [ "$(cat "$HOME/.example")" = base ]
  [ "$before" = "$(git -C "$DOT_SOURCE" rev-parse HEAD)" ]
}

@test "status reports drift and diff exposes both explicit directions" {
  select_example
  printf 'local\n' > "$HOME/.example"
  run "$DOT" status
  [ "$status" -eq 0 ]
  [[ "$output" == *"drift: 1"* ]]
  run "$DOT" diff --direction machine-to-repo
  [ "$status" -eq 0 ]
  [[ "$output" == *"you are ahead"* ]]
  [[ "$output" == *"+local"* ]]
  run "$DOT" diff --direction repo-to-machine
  [ "$status" -eq 0 ]
  [[ "$output" == *"+base"* ]]
}

@test "status distinguishes an empty exact adoption scope from zero drift" {
  mkdir -p "$HOME/.local/state/dot"
  printf '{"version":2,"profile":"fixture","entries":{}}\n' > "$HOME/.local/state/dot/adoption.json"
  run "$DOT" status
  [ "$status" -eq 0 ]
  [[ "$output" == *'0 selected entries'* ]]
  [[ "$output" == *'empty adoption scope'* ]]
  [[ "$output" == *'not whole-machine parity'* ]]
}

@test "status rendering failure preserves Git status and gives recovery guidance" {
  printf '{{ .name }}\n' > "$DOT_SOURCE/home/dot_broken.tmpl"
  python3 - "$DOT_SOURCE/tools/dot/manifest.json" <<'PY'
import json,sys
p=sys.argv[1]; d=json.load(open(p))
entry={'destination':'.broken','source':'home/dot_broken.tmpl','os':['darwin','linux'],
       'portability':'managed-templated','impact':'Safe','removals':[],'hooks':[],'tools':[]}
d['entries']['broken']=entry; d['profiles']['broken']=['broken']
json.dump(d,open(p,'w'))
PY
  git -C "$DOT_SOURCE" add home/dot_broken.tmpl tools/dot/manifest.json
  git -C "$DOT_SOURCE" -c user.name=Test -c user.email=test@example.org commit -qm 'synthetic broken template'
  printf '{"version":1,"mode":"adopt","profile":"broken","entries":["broken"]}\n' > "$BATS_TEST_TMPDIR/broken-request.json"
  "$DOT" intent plan "$BATS_TEST_TMPDIR/broken-request.json" --out "$BATS_TEST_TMPDIR/broken-plan.json"
  token="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["id"])' "$BATS_TEST_TMPDIR/broken-plan.json")"
  "$DOT" intent apply "$BATS_TEST_TMPDIR/broken-plan.json" --approve "$token"
  printf '{{ .missing }}\n' > "$DOT_SOURCE/home/dot_broken.tmpl"
  run "$DOT" status
  [ "$status" -ne 0 ]
  [[ "$output" == *'map has no entry for key "missing"'* ]]
  [[ "$output" == *'## '* ]]
  [[ "$output" == *'drift: unavailable'* ]]
  [[ "$output" == *'dot init --check'* ]]
  [[ "$output" == *'do not delete machine data or apply unscoped configuration'* ]]
}

@test "exact Track and untrack approvals classify source while retaining the live file" {
  printf 'new\n' > "$HOME/.new"
  run "$DOT" add --class shared "$HOME/.new"
  [ "$status" -ne 0 ]
  [[ "$output" == *'exact dot intent Track plan/approval'* ]]
  [ ! -e "$DOT_SOURCE/home/dot_new" ]
  track_file .new managed-shared '["darwin","linux"]'
  [ -f "$DOT_SOURCE/home/dot_new" ]
  python3 - "$DOT_SOURCE/tools/dot/manifest.json" <<'PY'
import json,sys
assert json.load(open(sys.argv[1]))['entries']['.new']['portability']=='managed-shared'
PY
  run "$DOT" remove "$HOME/.new"
  [ "$status" -ne 0 ]
  [[ "$output" == *'exact dot intent untrack plan/approval'* ]]
  [ -f "$HOME/.new" ]
  untrack_file .new
  [ -f "$HOME/.new" ]
  [ ! -e "$DOT_SOURCE/home/dot_new" ]
  track_file .new managed-templated '["darwin","linux"]'
  [ -f "$DOT_SOURCE/home/private_dot_new.tmpl" ]
}

@test "add rejects local-unmanaged files even nested inside an adopted directory" {
  mkdir -p "$HOME/.config/test"
  printf 'token\n' > "$HOME/.config/test/auth.json"
  run "$DOT" add --class shared "$HOME/.config/test"
  [ "$status" -ne 0 ]
  [ ! -e "$DOT_SOURCE/dot_config" ]
}

@test "os-only adoption is gated and classified" {
  printf 'new\n' > "$HOME/.new"
  track_file .new managed-os-only '["darwin"]'
  python3 - "$DOT_SOURCE/tools/dot/manifest.json" <<'PY'
import json,sys
entry=json.load(open(sys.argv[1]))['entries']['.new']
assert entry['portability']=='managed-os-only' and entry['os']==['darwin']
PY
}

@test "run forwards arguments but rejects traversal" {
  mkdir -p "$DOT_SOURCE/scripts"
  printf '#!/usr/bin/env bash\nprintf "argument: %%s\\n" "$1"\n' > "$DOT_SOURCE/scripts/example.sh"
  run "$DOT" run example 'two words'
  [ "$status" -eq 0 ]
  [ "$output" = 'argument: two words' ]
  run "$DOT" run ../example
  [ "$status" -ne 0 ]
}

@test "update bumps a pin and its installer before invoking installers" {
  mkdir -p "$DOT_SOURCE/packages" "$DOT_SOURCE/scripts"
  printf '[pins.example]\nversion = "1.2.3"\ncheck = "example --version"\ninstall_darwin = "install example@1.2.3"\ninstall_linux = "install example@1.2.3"\n' > "$DOT_SOURCE/packages/pins.toml"
  printf '#!/usr/bin/env bash\n[ "$1" = --apply ] && touch "$HOME/installed"\n' > "$DOT_SOURCE/scripts/install-pins.sh"
  run "$DOT" update example 2.0.0
  [ "$status" -eq 0 ]
  [[ "$(cat "$DOT_SOURCE/packages/pins.toml")" == *'example@2.0.0'* ]]
  [ -e "$HOME/installed" ]
}

@test "doctor aggregates pin and secret checks without executing secrets.env" {
  mkdir -p "$DOT_SOURCE/scripts" "$HOME/.config"
  printf '#!/usr/bin/env bash\necho pins-ok\n' > "$DOT_SOURCE/scripts/install-pins.sh"
  mkdir -p "$DOT_SOURCE/home/dot_config/example"
  printf '{"key": "{env:FIRECRAWL_API_KEY}", "credential": "{env:ACCESS_CREDENTIAL}"}\n' > "$DOT_SOURCE/home/dot_config/example/secret-config.json"
  python3 - "$DOT_SOURCE/tools/dot/manifest.json" <<'PY'
import json,sys
p=sys.argv[1]; d=json.load(open(p))
d['entries']['secret-config']={'destination':'.config/example/secret-config.json',
 'source':'home/dot_config/example/secret-config.json','os':['darwin','linux'],
 'portability':'managed-shared','impact':'Safe','removals':[],'hooks':[],'tools':[]}
d['profiles']['secret-check']=['secret-config']
json.dump(d,open(p,'w'))
PY
  git -C "$DOT_SOURCE" add home/dot_config/example/secret-config.json tools/dot/manifest.json
  git -C "$DOT_SOURCE" -c user.name=Test -c user.email=test@example.org commit -qm 'synthetic secret reference fixture'
  printf '{"version":1,"mode":"adopt","profile":"secret-check","entries":["secret-config"]}\n' > "$BATS_TEST_TMPDIR/secret-request.json"
  "$DOT" intent plan "$BATS_TEST_TMPDIR/secret-request.json" --out "$BATS_TEST_TMPDIR/secret-plan.json"
  token="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["id"])' "$BATS_TEST_TMPDIR/secret-plan.json")"
  "$DOT" intent apply "$BATS_TEST_TMPDIR/secret-plan.json" --approve "$token"
  run "$DOT" doctor
  [ "$status" -ne 0 ]
  [[ "$output" == *'MISSING FIRECRAWL_API_KEY'* ]]
  [[ "$output" == *pins-ok* ]]
  printf 'FIRECRAWL_API_KEY=fixture\n' > "$HOME/.config/secrets.env"
  run "$DOT" doctor
  [ "$status" -ne 0 ]
  [[ "$output" == *'MISSING ACCESS_CREDENTIAL'* ]]
  printf 'FIRECRAWL_API_KEY=fixture\nACCESS_CREDENTIAL=fixture\ntouch "$HOME/executed"\n' > "$HOME/.config/secrets.env"
  run "$DOT" doctor
  [ "$status" -eq 0 ]
  [ ! -e "$HOME/executed" ]
}

@test "doctor ignores secret examples in skill documentation and scripts" {
  mkdir -p "$DOT_SOURCE/dot_agents/skills/example" "$DOT_SOURCE/scripts" "$HOME/.config"
  printf '# Example\nUse $EXAMPLE_API_KEY and {env:DOC_TOKEN}\n' > "$DOT_SOURCE/dot_agents/skills/example/SKILL.md"
  printf '#!/usr/bin/env bash\necho pins-ok\n# $SCRIPT_API_KEY\n' > "$DOT_SOURCE/scripts/install-pins.sh"
  run "$DOT" doctor
  [ "$status" -eq 0 ]
  [[ "$output" != *'MISSING'* ]]
}

@test "adoption rejects credential stores and reclassification removes old OS gates" {
  mkdir -p "$HOME/.aws"
  printf 'aws_secret_access_key=fixture\n' > "$HOME/.aws/credentials"
  run "$DOT" add --class shared "$HOME/.aws"
  [ "$status" -ne 0 ]
  printf 'new\n' > "$HOME/.new"
  track_file .new managed-os-only '["darwin"]'
  python3 - "$DOT_SOURCE/tools/dot/manifest.json" <<'PY'
import json,sys
assert json.load(open(sys.argv[1]))['entries']['.new']['os']==['darwin']
PY
  untrack_file .new
  track_file .new managed-shared '["darwin","linux"]'
  python3 - "$DOT_SOURCE/tools/dot/manifest.json" <<'PY'
import json,sys
entry=json.load(open(sys.argv[1]))['entries']['.new']
assert entry['portability']=='managed-shared' and entry['os']==['darwin','linux']
PY
}
