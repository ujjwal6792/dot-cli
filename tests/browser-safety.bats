#!/usr/bin/env bats
setup() {
  REPO="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
  export HOME="$BATS_TEST_TMPDIR/home" DOT_SOURCE="$BATS_TEST_TMPDIR/private" DOT_ENGINE="$BATS_TEST_TMPDIR/public/tools/dot"
  mkdir -p "$HOME" "$DOT_SOURCE/home" "$DOT_SOURCE/tools/dot" "$DOT_ENGINE"
  cp "$REPO/tools/dot/"*.py "$DOT_ENGINE/"
  cp "$REPO/tools/dot/engine.json" "$REPO/tools/dot/dot" "$DOT_ENGINE/"
  printf '{"protocol":1,"engine_api":{"min":1,"max":1},"manifest_version":1,"approval_version":2,"manifest":"tools/dot/manifest.json","home_source":"home","required_adapters":["intent","scope","gitconfig","skills","layout"]}\n' > "$DOT_SOURCE/dot-contract.json"
  printf '{"version":1,"entries":{},"profiles":{}}\n' > "$DOT_SOURCE/tools/dot/manifest.json"
}
inventory() {
  python3 -c 'import json,os; print(json.dumps(dict(version=1,command="inventory",engine=os.environ["DOT_ENGINE"],source=os.environ["DOT_SOURCE"])))' | python3 -B "$DOT_ENGINE/browser_api.py"
}
@test "inventory does not enumerate excluded runtime data or unrelated repository history" {
  mkdir -p "$HOME/.ssh" "$HOME/project/.git" "$HOME/.cache"
  printf 'fixture only\n' > "$HOME/.ssh/private-label"
  printf 'fixture only\n' > "$HOME/project/owner-local-file"
  printf 'fixture only\n' > "$HOME/.cache/private-label"
  run inventory
  [ "$status" -eq 0 ]
  [[ "$output" != *'private-label'* ]]
  [[ "$output" != *'owner-local-file'* ]]
  [[ "$output" == *'excluded'* ]]
}
@test "Adopt inventory distinguishes target symlinks without following them" {
  printf 'repository fixture\n' > "$DOT_SOURCE/home/preferences.conf"
  printf 'private outside fixture\n' > "$BATS_TEST_TMPDIR/outside"
  ln -s "$BATS_TEST_TMPDIR/outside" "$HOME/preferences.conf"
  cat > "$DOT_SOURCE/tools/dot/manifest.json" <<'JSON'
{"version":1,"entries":{"prefs":{"destination":"preferences.conf","source":"home/preferences.conf","os":["darwin","linux"],"portability":"managed-shared","impact":"Safe","removals":[],"hooks":[],"tools":[]}},"profiles":{}}
JSON
  run python3 - "$DOT_ENGINE/browser_api.py" <<'PY'
import json, os, subprocess, sys
request = dict(version=1, command='inventory', mode='adopt', engine=os.environ['DOT_ENGINE'], source=os.environ['DOT_SOURCE'])
proc = subprocess.run([sys.executable, '-B', sys.argv[1]], input=json.dumps(request)+'\n', text=True, capture_output=True)
assert proc.returncode == 0, (proc.stdout, proc.stderr)
row = json.loads(proc.stdout)['result']['entries'][0]
assert row['target_status'] == 'symlink', row
assert 'private outside fixture' not in proc.stdout
PY
  [ "$status" -eq 0 ]
}
@test "Adopt named presets expose exact manifest IDs without selecting them" {
  cat > "$DOT_SOURCE/tools/dot/manifest.json" <<'JSON'
{"version":1,"entries":{"prefs":{"destination":"preferences.conf","source":"home/preferences.conf","os":["darwin","linux"],"portability":"managed-shared","impact":"Safe","removals":[],"hooks":[],"tools":[]}},"profiles":{"work":["prefs"]}}
JSON
  run python3 - "$DOT_ENGINE/browser_api.py" <<'PY'
import json, os, subprocess, sys
request=dict(version=1, command='inventory', mode='adopt', engine=os.environ['DOT_ENGINE'], source=os.environ['DOT_SOURCE'])
proc=subprocess.run([sys.executable,'-B',sys.argv[1]],input=json.dumps(request)+'\n',text=True,capture_output=True)
assert proc.returncode==0, (proc.stdout,proc.stderr)
result=json.loads(proc.stdout)['result']
assert result['profiles']==[{'name':'work','entries':['prefs']}], result
assert not os.path.exists(os.environ['HOME']+'/.local/state/dot/adoption.json')
PY
  [ "$status" -eq 0 ]
}
@test "Track live highlight identifies source diff rather than unrelated manifest rewrite" {
  printf 'live highlight fixture\n' > "$HOME/preferences.conf"
  git init -q "$DOT_SOURCE"
  git -C "$DOT_SOURCE" config user.name Test
  git -C "$DOT_SOURCE" config user.email fixture@example.org
  git -C "$DOT_SOURCE" add .
  git -C "$DOT_SOURCE" commit -qm fixture
  run python3 - "$DOT_ENGINE/browser_api.py" <<'PY'
import json,os,subprocess,sys
request=dict(version=1,command='highlight',mode='track',path='preferences.conf',engine=os.environ['DOT_ENGINE'],source=os.environ['DOT_SOURCE'])
proc=subprocess.run([sys.executable,'-B',sys.argv[1]],input=json.dumps(request)+'\n',text=True,capture_output=True)
assert proc.returncode==0,(proc.stdout,proc.stderr)
result=json.loads(proc.stdout)['result']
assert 'live highlight fixture' in result['text'],result
assert 'REPOSITORY/home/preferences.conf' in result['text'],result
assert 'manifest.json' not in result['text'],result
assert not os.path.exists(os.environ['DOT_SOURCE']+'/home/preferences.conf')
PY
  [ "$status" -eq 0 ]
}
@test "tampered exact plan cannot preview an unselected target path" {
  printf 'unselected fixture context\n' > "$HOME/unselected.conf"
  printf 'reviewed fixture\n' > "$DOT_SOURCE/home/preferences.conf"
  cat > "$DOT_SOURCE/tools/dot/manifest.json" <<'JSON'
{"version":1,"entries":{"prefs":{"destination":"preferences.conf","source":"home/preferences.conf","os":["darwin","linux"],"portability":"managed-shared","impact":"Safe","removals":[],"hooks":[],"tools":[]}},"profiles":{}}
JSON
  git init -q "$DOT_SOURCE"
  git -C "$DOT_SOURCE" config user.name Test
  git -C "$DOT_SOURCE" config user.email fixture@example.org
  git -C "$DOT_SOURCE" add .
  git -C "$DOT_SOURCE" commit -qm fixture
  run python3 - "$DOT_ENGINE/browser_api.py" <<'PY'
import json, os, subprocess, sys
base = dict(version=1, engine=os.environ['DOT_ENGINE'], source=os.environ['DOT_SOURCE'])
def api(**fields):
    response = subprocess.run([sys.executable, '-B', sys.argv[1]], input=json.dumps(dict(base, **fields))+'\n', text=True, capture_output=True)
    assert response.stdout, response.stderr
    return response, json.loads(response.stdout)
_, result = api(command='plan', request=dict(version=1, mode='adopt', profile='default', entries=['prefs']))
plan = result['result']['plan']
plan['actions'][0]['path'] = 'unselected.conf'
response, result = api(command='preview', plan=plan)
assert response.returncode != 0, response.stdout
assert result['ok'] is False
assert 'unselected fixture context' not in response.stdout
assert 'stale' in result['error'] or 'exact' in result['error'], result
PY
  printf '%s\n' "$output" >&3
  [ "$status" -eq 0 ]
}
@test "directory snapshot contains only current eligible regular files" {
  mkdir -p "$HOME/configs/project/.git"
  printf 'safe fixture\n' > "$HOME/configs/preferences.conf"
  printf 'fake credential\n' > "$HOME/configs/.env"
  printf 'private fixture\n' > "$HOME/configs/project/unrelated.conf"
  printf '\000binary\n' > "$HOME/configs/image.dat"
  ln -s preferences.conf "$HOME/configs/link.conf"
  run python3 - "$DOT_ENGINE/browser_api.py" <<'PY'
import json, os, subprocess, sys
request = dict(version=1, command='snapshot', path='configs', engine=os.environ['DOT_ENGINE'], source=os.environ['DOT_SOURCE'])
proc = subprocess.run([sys.executable, '-B', sys.argv[1]], input=json.dumps(request)+'\n', text=True, capture_output=True)
assert proc.returncode == 0, (proc.stdout, proc.stderr)
response = json.loads(proc.stdout)
assert response['result']['paths'] == ['configs/preferences.conf'], response
PY
  printf '%s\n' "$output" >&3
  [ "$status" -eq 0 ]
}
@test "Track applies through the negotiated private manifest path" {
  mkdir -p "$DOT_SOURCE/config"
  mv "$DOT_SOURCE/tools/dot/manifest.json" "$DOT_SOURCE/config/catalog.json"
  python3 - "$DOT_SOURCE/dot-contract.json" <<'PY'
import json, sys
path = sys.argv[1]
data = json.load(open(path)); data['manifest'] = 'config/catalog.json'
with open(path, 'w') as file: json.dump(data, file)
PY
  printf 'target fixture\n' > "$HOME/preferences.conf"
  git init -q "$DOT_SOURCE"
  git -C "$DOT_SOURCE" config user.name Test
  git -C "$DOT_SOURCE" config user.email fixture@example.org
  git -C "$DOT_SOURCE" add .
  git -C "$DOT_SOURCE" commit -qm fixture
  run python3 - "$DOT_ENGINE/browser_api.py" <<'PY'
import json, os, subprocess, sys
base = dict(version=1, engine=os.environ['DOT_ENGINE'], source=os.environ['DOT_SOURCE'])
def api(**fields):
    response = subprocess.run([sys.executable, '-B', sys.argv[1]], input=json.dumps(dict(base, **fields))+'\n', text=True, capture_output=True)
    assert response.returncode == 0, (response.stdout, response.stderr)
    return json.loads(response.stdout)['result']
request = dict(version=1, mode='track', profile='work', selections=[dict(path='preferences.conf', os=['darwin','linux'], portability='managed-shared', impact='Safe')])
plan = api(command='plan', request=request)['plan']
review = api(command='preview', plan=plan)
assert 'none → 0600' in review['diffs'][0]['summary'], review
api(command='apply', plan=plan, approve=plan['id'])
source = os.environ['DOT_SOURCE']
assert 'preferences.conf' in json.load(open(source+'/config/catalog.json'))['entries']
assert not os.path.exists(source+'/tools/dot/manifest.json')
assert open(source+'/home/preferences.conf').read() == 'target fixture\n'
assert os.stat(source+'/home/preferences.conf').st_mode & 0o777 == 0o600
assert open(os.environ['HOME']+'/preferences.conf').read() == 'target fixture\n'
PY
  printf '%s\n' "$output" >&3
  [ "$status" -eq 0 ]
}
@test "review reports exact executable permission changes before apply" {
  printf '#!/bin/sh\nprintf fixture\n' > "$DOT_SOURCE/home/executable_script.sh"
  printf '#!/bin/sh\nprintf local\n' > "$HOME/script.sh"
  chmod 644 "$HOME/script.sh"
  cat > "$DOT_SOURCE/tools/dot/manifest.json" <<'JSON'
{"version":1,"entries":{"script":{"destination":"script.sh","source":"home/executable_script.sh","os":["darwin","linux"],"portability":"managed-shared","impact":"Safe","removals":[],"hooks":[],"tools":[]}},"profiles":{}}
JSON
  git init -q "$DOT_SOURCE"
  git -C "$DOT_SOURCE" config user.name Test
  git -C "$DOT_SOURCE" config user.email fixture@example.org
  git -C "$DOT_SOURCE" add .
  git -C "$DOT_SOURCE" commit -qm fixture
  run python3 - "$DOT_ENGINE/browser_api.py" <<'PY'
import json, os, subprocess, sys
request = dict(version=1, command='plan', engine=os.environ['DOT_ENGINE'], source=os.environ['DOT_SOURCE'], request=dict(version=1, mode='adopt', profile='default', entries=['script']))
proc = subprocess.run([sys.executable, '-B', sys.argv[1]], input=json.dumps(request)+'\n', text=True, capture_output=True)
assert proc.returncode == 0, (proc.stdout, proc.stderr)
result = json.loads(proc.stdout)['result']
review = result['review'] + '\n' + '\n'.join(d['summary'] for d in result['diffs'])
assert '0644' in review and '0755' in review, review
assert os.stat(os.environ['HOME']+'/script.sh').st_mode & 0o777 == 0o644
PY
  printf '%s\n' "$output" >&3
  [ "$status" -eq 0 ]
}
@test "live Adopt highlight previews missing destinations without Track fallback" {
  printf 'repository fixture\n' > "$DOT_SOURCE/home/preferences.conf"
  cat > "$DOT_SOURCE/tools/dot/manifest.json" <<'JSON'
{"version":1,"entries":{"prefs":{"destination":"preferences.conf","source":"home/preferences.conf","os":["darwin","linux"],"portability":"managed-shared","impact":"Safe","removals":[],"hooks":[],"tools":[]}},"profiles":{}}
JSON
  git init -q "$DOT_SOURCE"
  git -C "$DOT_SOURCE" config user.name Test
  git -C "$DOT_SOURCE" config user.email fixture@example.org
  git -C "$DOT_SOURCE" add .
  git -C "$DOT_SOURCE" commit -qm fixture
  run python3 - "$DOT_ENGINE/browser_api.py" <<'PY'
import json, os, subprocess, sys
request = dict(version=1, command='highlight', mode='adopt', path='preferences.conf', engine=os.environ['DOT_ENGINE'], source=os.environ['DOT_SOURCE'])
proc = subprocess.run([sys.executable, '-B', sys.argv[1]], input=json.dumps(request)+'\n', text=True, capture_output=True)
assert proc.returncode == 0, (proc.stdout, proc.stderr)
result = json.loads(proc.stdout)['result']
assert result['direction'] == 'repository → TARGET', result
assert 'repository fixture' in result['diffs'][0]['text'], result
assert not os.path.exists(os.environ['HOME']+'/preferences.conf')
target = os.environ['HOME']+'/preferences.conf'
with open(target, 'w') as file: file.write('password=private-fixture-value\n')
os.chmod(target, 0o640)
proc = subprocess.run([sys.executable, '-B', sys.argv[1]], input=json.dumps(request)+'\n', text=True, capture_output=True)
assert proc.returncode == 0, (proc.stdout, proc.stderr)
result = json.loads(proc.stdout)['result']
assert 'private-fixture-value' not in proc.stdout
assert 'withheld' in result['diffs'][0]['text'].lower(), result
assert 'Replace' in result['diffs'][0]['summary'] and '0640' in result['diffs'][0]['summary'], result
assert open(target).read() == 'password=private-fixture-value\n'
PY
  printf '%s\n' "$output" >&3
  [ "$status" -eq 0 ]
}
