#!/usr/bin/env bats
setup() {
  export HOME="$BATS_TEST_TMPDIR/home"
  ENGINE="$BATS_TEST_TMPDIR/public/tools/dot"
  SOURCE="$BATS_TEST_TMPDIR/private"
  mkdir -p "$HOME" "$ENGINE" "$SOURCE/home" "$SOURCE/tools/dot"
  input="${DOT_UPDATE_ENGINE_FIXTURE:-$BATS_TEST_DIRNAME/../tools/dot}"
  cp "$input/"*.py "$input/engine.json" "$input/dot" "$ENGINE/"
  cp "$input/ui.sh" "$ENGINE/"
  printf '{"protocol":1,"engine_api":{"min":1,"max":1},"manifest_version":1,"approval_version":2,"manifest":"tools/dot/manifest.json","home_source":"home","required_adapters":["intent","scope","gitconfig","skills","layout"]}\n' > "$SOURCE/dot-contract.json"
  printf '{"version":1,"entries":{},"profiles":{}}\n' > "$SOURCE/tools/dot/manifest.json"
  printf '[data]\ndotEngine = "%s"\n' "$ENGINE" > "$HOME/config.toml"
  export DOT_ENGINE="$ENGINE" DOT_SOURCE="$SOURCE" DOT_CONFIG="$HOME/config.toml"
  git init -q "$ENGINE/../.."
  git -C "$ENGINE/../.." add .
  git -C "$ENGINE/../.." -c user.name=Fixture -c user.email=fixture@example.invalid commit -qm initial
  git clone -q --bare "$ENGINE/../.." "$BATS_TEST_TMPDIR/remote"
  git -C "$ENGINE/../.." remote add origin "$BATS_TEST_TMPDIR/remote"
  git -C "$ENGINE/../.." fetch -q origin
  git -C "$ENGINE/../.." branch --set-upstream-to=origin/$(git -C "$ENGINE/../.." branch --show-current) >/dev/null
  git clone -q "$BATS_TEST_TMPDIR/remote" "$BATS_TEST_TMPDIR/author"
  printf 'reviewed tooling\n' > "$BATS_TEST_TMPDIR/author/CHANGELOG"
  git -C "$BATS_TEST_TMPDIR/author" add .
  git -C "$BATS_TEST_TMPDIR/author" -c user.name=Fixture -c user.email=fixture@example.invalid commit -qm tooling
  git -C "$BATS_TEST_TMPDIR/author" push -q
}
@test "self-update previews then explicitly fast-forwards public tooling only" {
  before=$(git -C "$ENGINE/../.." rev-parse HEAD)
  run "$ENGINE/dot" self-update
  [ "$status" -eq 0 ]
  [ "$before" = "$(git -C "$ENGINE/../.." rev-parse HEAD)" ]
  run "$ENGINE/dot" self-update --yes
  [ "$status" -eq 0 ]
  [ -f "$ENGINE/../../CHANGELOG" ]
  [ ! -e "$HOME/.local" ]
  [ ! -e "$SOURCE/CHANGELOG" ]
}
@test "self-update refuses dirty public work without discarding it" {
  printf 'local work\n' > "$ENGINE/../../local"
  run "$ENGINE/dot" self-update --yes
  [ "$status" -ne 0 ]
  [ "$(cat "$ENGINE/../../local")" = 'local work' ]
  [ ! -e "$ENGINE/../../CHANGELOG" ]
}
@test "self-update rejects incompatible candidate before advancing or executing it" {
  printf '{"protocol":999}\n' > "$BATS_TEST_TMPDIR/author/tools/dot/engine.json"
  printf '#!/bin/sh\ntouch "%s/executed"\n' "$HOME" > "$BATS_TEST_TMPDIR/author/tools/dot/dot"
  git -C "$BATS_TEST_TMPDIR/author" add .
  git -C "$BATS_TEST_TMPDIR/author" -c user.name=Fixture -c user.email=fixture@example.invalid commit -qm incompatible
  git -C "$BATS_TEST_TMPDIR/author" push -q
  run "$ENGINE/dot" self-update --yes
  [ "$status" -ne 0 ]
  [ ! -e "$HOME/executed" ]
  [ ! -e "$ENGINE/../../CHANGELOG" ]
}
@test "pull refuses missing exact approval without fetching or applying" {
  run "$ENGINE/dot" pull --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'approved exact scope'* ]]
  [ ! -e "$SOURCE/.git/FETCH_HEAD" ]
  [ ! -e "$ENGINE/../../CHANGELOG" ]
}
private_remote() {
  python3 - "$SOURCE/tools/dot/manifest.json" <<'PY'
import json,sys
entry = dict(destination='.config/example.conf', source='home/dot_config/example.conf', hooks=[],
             impact='Safe', integrations=[], os=['darwin','linux'], portability='managed-shared', removals=[], tools=[])
json.dump(dict(version=1, entries={'.config/example.conf':entry}, profiles={'fixture':['.config/example.conf']}),open(sys.argv[1],'w'))
PY
  mkdir -p "$SOURCE/home/dot_config"
  printf 'old\n' > "$SOURCE/home/dot_config/example.conf"
  if [ "${1:-}" = v3 ]; then
    python3 -B - "$SOURCE" "$HOME" "$ENGINE" <<'PY'
import json,sys
from pathlib import Path
source,home,engine=map(Path,sys.argv[1:])
sys.path.insert(0,str(engine))
from file_metadata import compile_entries, current_os
system=current_os()
definition=dict(destinations={system:'.config/example.conf'},
                sources={system:'home/dot_config/example.conf'},
                common={'tools':['fixture-common']},
                machine={'fixture-machine':{'tools':['fixture-machine-tool']}},
                os_metadata={system:{'tools':['fixture-os-tool']}}, machines=['fixture-machine'])
catalog=dict(version=2, entries=compile_entries('example',definition),
             profiles={'fixture':['example@'+system]},
             machines={'fixture-machine':{'label':'Fixture','os':system}},
             tracking={'example':definition})
(source/'tools/dot/manifest.json').write_text(json.dumps(catalog))
path=source/'dot-contract.json'
contract=json.loads(path.read_text()); contract.update(manifest_version=2,approval_version=3)
path.write_text(json.dumps(contract))
identity=home/'.local/state/dot/machine.json'
identity.parent.mkdir(parents=True)
identity.write_text(json.dumps({'version':1,'id':'fixture-machine'})); identity.chmod(0o600)
PY
  fi
  git init -q "$SOURCE"
  git -C "$SOURCE" config user.name Fixture
  git -C "$SOURCE" config user.email fixture@example.invalid
  git -C "$SOURCE" add .
  git -C "$SOURCE" commit -qm initial
  printf '{"version":1,"mode":"adopt","profile":"fixture"}\n' > "$BATS_TEST_TMPDIR/request.json"
  "$ENGINE/dot" intent plan "$BATS_TEST_TMPDIR/request.json" --out "$BATS_TEST_TMPDIR/plan.json"
  token=$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["id"])' "$BATS_TEST_TMPDIR/plan.json")
  "$ENGINE/dot" intent apply "$BATS_TEST_TMPDIR/plan.json" --approve "$token"
  cp "$HOME/.local/state/dot/adoption.json" "$BATS_TEST_TMPDIR/approval-before"
  git clone -q --bare "$SOURCE" "$BATS_TEST_TMPDIR/private-remote"
  git -C "$SOURCE" remote add origin "$BATS_TEST_TMPDIR/private-remote"
  git -C "$SOURCE" fetch -q origin
  git -C "$SOURCE" branch --set-upstream-to=origin/$(git -C "$SOURCE" branch --show-current) >/dev/null
  git clone -q "$BATS_TEST_TMPDIR/private-remote" "$BATS_TEST_TMPDIR/private-author"
  printf 'new\n' > "$BATS_TEST_TMPDIR/private-author/home/dot_config/example.conf"
  git -C "$BATS_TEST_TMPDIR/private-author" add .
  git -C "$BATS_TEST_TMPDIR/private-author" -c user.name=Fixture -c user.email=fixture@example.invalid commit -qm configuration
  git -C "$BATS_TEST_TMPDIR/private-author" push -q
}
@test "pull previews candidate then applies unchanged scope without changing approval bytes or public checkout" {
  private_remote
  before=$(git -C "$SOURCE" rev-parse HEAD)
  public=$(git -C "$ENGINE/../.." rev-parse HEAD)
  run "$ENGINE/dot" pull
  [ "$status" -eq 0 ]
  [ "$before" = "$(git -C "$SOURCE" rev-parse HEAD)" ]
  [ "$(cat "$HOME/.config/example.conf")" = old ]
  run "$ENGINE/dot" pull --yes
  [ "$status" -eq 0 ]
  [ "$(cat "$HOME/.config/example.conf")" = new ]
  [ "$public" = "$(git -C "$ENGINE/../.." rev-parse HEAD)" ]
  cmp "$BATS_TEST_TMPDIR/approval-before" "$HOME/.local/state/dot/adoption.json"
}
@test "self-update is tooling maintenance even when tracker is not initialized" {
  run env -u DOT_CONFIG "$ENGINE/dot" self-update --yes
  [ "$status" -eq 0 ]
  [ -f "$ENGINE/../../CHANGELOG" ]
  [ ! -e "$HOME/.local" ]
}
@test "self-update rejects a broken candidate entrypoint despite compatible descriptor" {
  printf '#!/bin/bash\nif broken\n' > "$BATS_TEST_TMPDIR/author/tools/dot/dot"
  git -C "$BATS_TEST_TMPDIR/author" add .
  git -C "$BATS_TEST_TMPDIR/author" -c user.name=Fixture -c user.email=fixture@example.invalid commit -qm broken
  git -C "$BATS_TEST_TMPDIR/author" push -q
  run "$ENGINE/dot" self-update --yes
  [ "$status" -ne 0 ]
  [ ! -e "$ENGINE/../../CHANGELOG" ]
}
@test "pull refuses target drift before advancing private source" {
  private_remote
  before=$(git -C "$SOURCE" rev-parse HEAD)
  printf 'local work\n' > "$HOME/.config/example.conf"
  run "$ENGINE/dot" pull --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'machine drift'* ]]
  [ "$before" = "$(git -C "$SOURCE" rev-parse HEAD)" ]
  [ "$(cat "$HOME/.config/example.conf")" = 'local work' ]
  cmp "$BATS_TEST_TMPDIR/approval-before" "$HOME/.local/state/dot/adoption.json"
}
@test "self-update refuses private configuration smuggled into public payload" {
  printf '{}\n' > "$BATS_TEST_TMPDIR/author/tools/dot/manifest.json"
  git -C "$BATS_TEST_TMPDIR/author" add .
  git -C "$BATS_TEST_TMPDIR/author" -c user.name=Fixture -c user.email=fixture@example.invalid commit -qm private
  git -C "$BATS_TEST_TMPDIR/author" push -q
  run "$ENGINE/dot" self-update --yes
  [ "$status" -ne 0 ]
  [ ! -e "$ENGINE/manifest.json" ]
  [ ! -e "$ENGINE/../../CHANGELOG" ]
}
@test "pull rejects changed approved capabilities before advancing source" {
  private_remote
  before=$(git -C "$SOURCE" rev-parse HEAD)
  python3 - "$BATS_TEST_TMPDIR/private-author/tools/dot/manifest.json" <<'PY'
import json,sys
p=sys.argv[1]; data=json.load(open(p)); data['entries']['.config/example.conf']['impact']='Unsafe'
json.dump(data,open(p,'w'))
PY
  git -C "$BATS_TEST_TMPDIR/private-author" -c user.name=Fixture -c user.email=fixture@example.invalid commit -qam capability
  git -C "$BATS_TEST_TMPDIR/private-author" push -q
  run "$ENGINE/dot" pull --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'fresh adoption approval required'* ]]
  [ "$before" = "$(git -C "$SOURCE" rev-parse HEAD)" ]
  [ "$(cat "$HOME/.config/example.conf")" = old ]
  cmp "$BATS_TEST_TMPDIR/approval-before" "$HOME/.local/state/dot/adoption.json"
}
@test "pull leaves newly published entries and hooks opt-in while updating approved files" {
  private_remote
  author="$BATS_TEST_TMPDIR/private-author"
  printf 'opt-in\n' > "$author/home/dot_config/new.conf"
  printf '#!/bin/sh\ntouch "$HOME/private-hook-ran"\n' > "$author/home/run_before_surprise.sh"
  python3 - "$author/tools/dot/manifest.json" <<'PY'
import json,sys
p=sys.argv[1]; data=json.load(open(p)); e=dict(data['entries']['.config/example.conf'])
e.update(destination='.config/new.conf', source='home/dot_config/new.conf')
data['entries']['.config/new.conf']=e; data['profiles']['fixture'].append('.config/new.conf')
json.dump(data,open(p,'w'))
PY
  git -C "$author" add .
  git -C "$author" -c user.name=Fixture -c user.email=fixture@example.invalid commit -qm opt-in
  git -C "$author" push -q
  run "$ENGINE/dot" pull --yes
  [ "$status" -eq 0 ]
  [ "$(cat "$HOME/.config/example.conf")" = new ]
  [ ! -e "$HOME/.config/new.conf" ]
  [ ! -e "$HOME/private-hook-ran" ]
  cmp "$BATS_TEST_TMPDIR/approval-before" "$HOME/.local/state/dot/adoption.json"
}
@test "pull never runs repository checkout filters as implicit capabilities" {
  private_remote
  printf '*.conf filter=surprise\n' > "$BATS_TEST_TMPDIR/private-author/.gitattributes"
  git -C "$BATS_TEST_TMPDIR/private-author" add .
  git -C "$BATS_TEST_TMPDIR/private-author" -c user.name=Fixture -c user.email=fixture@example.invalid commit -qm filters
  git -C "$BATS_TEST_TMPDIR/private-author" push -q
  git -C "$SOURCE" config filter.surprise.smudge "sh -c 'touch \"$HOME/filter-ran\"; cat'"
  run "$ENGINE/dot" pull --yes
  [ "$status" -eq 0 ]
  [ ! -e "$HOME/filter-ran" ]
  [ "$(cat "$HOME/.config/example.conf")" = new ]
}
@test "public and private remote-access failures preserve HEAD target and approval bytes" {
  private_remote
  public=$(git -C "$ENGINE/../.." rev-parse HEAD)
  private=$(git -C "$SOURCE" rev-parse HEAD)
  git -C "$ENGINE/../.." remote set-url origin "$BATS_TEST_TMPDIR/denied-public"
  git -C "$SOURCE" remote set-url origin "$BATS_TEST_TMPDIR/denied-private"
  run "$ENGINE/dot" self-update --yes
  [ "$status" -ne 0 ]
  run "$ENGINE/dot" pull --yes
  [ "$status" -ne 0 ]
  [ "$public" = "$(git -C "$ENGINE/../.." rev-parse HEAD)" ]
  [ "$private" = "$(git -C "$SOURCE" rev-parse HEAD)" ]
  [ "$(cat "$HOME/.config/example.conf")" = old ]
  cmp "$BATS_TEST_TMPDIR/approval-before" "$HOME/.local/state/dot/adoption.json"
}
@test "dirty private source is preserved without fetching or applying" {
  private_remote
  before=$(git -C "$SOURCE" rev-parse HEAD)
  printf 'uncommitted\n' > "$SOURCE/local-work"
  run "$ENGINE/dot" pull --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'dirty'* ]]
  [ "$(cat "$SOURCE/local-work")" = uncommitted ]
  [ "$before" = "$(git -C "$SOURCE" rev-parse HEAD)" ]
  [ "$(cat "$HOME/.config/example.conf")" = old ]
}
@test "diverged public and private local commits are never reset" {
  private_remote
  for root in "$ENGINE/../.." "$SOURCE"; do
    printf 'local commit\n' > "$root/local-work"
    git -C "$root" add local-work
    git -C "$root" -c user.name=Fixture -c user.email=fixture@example.invalid commit -qm local
  done
  public=$(git -C "$ENGINE/../.." rev-parse HEAD)
  private=$(git -C "$SOURCE" rev-parse HEAD)
  run "$ENGINE/dot" self-update --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'ahead or diverged'* ]]
  run "$ENGINE/dot" pull --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'ahead or diverged'* ]]
  [ "$public" = "$(git -C "$ENGINE/../.." rev-parse HEAD)" ]
  [ "$private" = "$(git -C "$SOURCE" rev-parse HEAD)" ]
  [ "$(cat "$HOME/.config/example.conf")" = old ]
  cmp "$BATS_TEST_TMPDIR/approval-before" "$HOME/.local/state/dot/adoption.json"
}
@test "self-update preserves existing private source target approvals and never invokes hooks or managers" {
  private_remote
  private=$(git -C "$SOURCE" rev-parse HEAD)
  cp "$SOURCE/tools/dot/manifest.json" "$BATS_TEST_TMPDIR/manifest-before"
  mkdir -p "$BATS_TEST_TMPDIR/managers"
  for name in brew apt apt-get pacman yay chezmoi; do
    printf '#!/bin/sh\ntouch "$HOME/side-effect"; exit 99\n' > "$BATS_TEST_TMPDIR/managers/$name"
    chmod +x "$BATS_TEST_TMPDIR/managers/$name"
  done
  printf '#!/bin/sh\ntouch "$HOME/side-effect"\n' > "$ENGINE/../../.git/hooks/post-merge"
  chmod +x "$ENGINE/../../.git/hooks/post-merge"
  printf '#!/bin/sh\ntouch "$HOME/side-effect"\n' > "$SOURCE/home/run_before_surprise.sh"
  run env PATH="$BATS_TEST_TMPDIR/managers:$PATH" "$ENGINE/dot" self-update --yes
  [ "$status" -eq 0 ]
  [ ! -e "$HOME/side-effect" ]
  [ "$private" = "$(git -C "$SOURCE" rev-parse HEAD)" ]
  [ "$(cat "$HOME/.config/example.conf")" = old ]
  [ "$(cat "$SOURCE/home/dot_config/example.conf")" = old ]
  cmp "$BATS_TEST_TMPDIR/manifest-before" "$SOURCE/tools/dot/manifest.json"
  cmp "$BATS_TEST_TMPDIR/approval-before" "$HOME/.local/state/dot/adoption.json"
}
@test "self-update cannot overwrite ignored local files newly published upstream" {
  printf 'CHANGELOG\n' > "$ENGINE/../../.git/info/exclude"
  printf 'ignored local work\n' > "$ENGINE/../../CHANGELOG"
  run "$ENGINE/dot" self-update --yes
  [ "$status" -ne 0 ]
  [ "$(cat "$ENGINE/../../CHANGELOG")" = 'ignored local work' ]
}
@test "ahead public and private commits are preserved without reset" {
  private_remote
  for root in "$ENGINE/../.." "$SOURCE"; do
    git -C "$root" fetch -q origin
    git -C "$root" merge --ff-only -q '@{upstream}'
    printf 'ahead\n' > "$root/local-work"
    git -C "$root" add local-work
    git -C "$root" -c user.name=Fixture -c user.email=fixture@example.invalid commit -qm ahead
  done
  printf 'new\n' > "$HOME/.config/example.conf"
  public=$(git -C "$ENGINE/../.." rev-parse HEAD)
  private=$(git -C "$SOURCE" rev-parse HEAD)
  run "$ENGINE/dot" self-update --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'ahead or diverged'* ]]
  run "$ENGINE/dot" pull --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'ahead or diverged'* ]]
  [ "$public" = "$(git -C "$ENGINE/../.." rev-parse HEAD)" ]
  [ "$private" = "$(git -C "$SOURCE" rev-parse HEAD)" ]
  [ "$(cat "$HOME/.config/example.conf")" = new ]
  cmp "$BATS_TEST_TMPDIR/approval-before" "$HOME/.local/state/dot/adoption.json"
}
@test "pull rejects redirected recovery state before fast-forwarding source" {
  private_remote
  before=$(git -C "$SOURCE" rev-parse HEAD)
  mv "$HOME/.local/state/dot/recovery" "$BATS_TEST_TMPDIR/recovery"
  ln -s "$BATS_TEST_TMPDIR/recovery" "$HOME/.local/state/dot/recovery"
  run "$ENGINE/dot" pull --yes
  [ "$status" -ne 0 ]
  [ "$before" = "$(git -C "$SOURCE" rev-parse HEAD)" ]
  [ "$(cat "$HOME/.config/example.conf")" = old ]
  cmp "$BATS_TEST_TMPDIR/approval-before" "$HOME/.local/state/dot/adoption.json"
}

assert_v3_approval() {
  python3 - "$HOME/.local/state/dot/adoption.json" <<'PY'
import json,sys
state=json.load(open(sys.argv[1]))
system='linux' if sys.platform.startswith('linux') else sys.platform
assert state['version']==3
assert state['machine']=={'id':'fixture-machine','os':system}
assert set(state['entries'])=={'example@'+system}
assert state['entries']['example@'+system]['tools']==['fixture-os-tool']
PY
}

@test "v3 self-update preserves machine-bound approvals metadata and dirty private state" {
  private_remote v3
  assert_v3_approval
  before=$(git -C "$SOURCE" rev-parse HEAD)
  cp "$SOURCE/tools/dot/manifest.json" "$BATS_TEST_TMPDIR/manifest-before"
  cp "$HOME/.local/state/dot/machine.json" "$BATS_TEST_TMPDIR/machine-before"
  printf 'private local work\n' >> "$SOURCE/home/dot_config/example.conf"
  printf 'target local work\n' > "$HOME/.config/example.conf"
  chmod 600 "$HOME/.config/example.conf"
  cp -p "$HOME/.config/example.conf" "$BATS_TEST_TMPDIR/target-before"
  run "$ENGINE/dot" self-update --yes
  [ "$status" -eq 0 ]
  [ "$before" = "$(git -C "$SOURCE" rev-parse HEAD)" ]
  [ "$(cat "$SOURCE/home/dot_config/example.conf")" = $'old\nprivate local work' ]
  cmp "$BATS_TEST_TMPDIR/manifest-before" "$SOURCE/tools/dot/manifest.json"
  cmp "$BATS_TEST_TMPDIR/machine-before" "$HOME/.local/state/dot/machine.json"
  cmp "$BATS_TEST_TMPDIR/target-before" "$HOME/.config/example.conf"
  [ "$(python3 -c 'import os,sys;print(os.stat(sys.argv[1]).st_mode & 0o777)' "$HOME/.config/example.conf")" = 384 ]
  cmp "$BATS_TEST_TMPDIR/approval-before" "$HOME/.local/state/dot/adoption.json"
}

@test "v3 pull updates only approved mapping without enrolling new profile entries" {
  private_remote v3
  assert_v3_approval
  printf '\n\n' >> "$HOME/.local/state/dot/adoption.json"
  chmod 640 "$HOME/.local/state/dot/adoption.json"
  cp "$HOME/.local/state/dot/adoption.json" "$BATS_TEST_TMPDIR/approval-before"
  public=$(git -C "$ENGINE/../.." rev-parse HEAD)
  author="$BATS_TEST_TMPDIR/private-author"
  printf 'unapproved\n' > "$author/home/dot_config/new.conf"
  python3 - "$author/tools/dot/manifest.json" <<'PY'
import json,sys
p=sys.argv[1]; data=json.load(open(p)); e=dict(next(iter(data['entries'].values())))
e.update(destination='.config/new.conf',source='home/dot_config/new.conf')
data['entries']['new']=e; data['profiles']['fixture'].append('new')
json.dump(data,open(p,'w'))
PY
  git -C "$author" add .
  git -C "$author" -c user.name=Fixture -c user.email=fixture@example.invalid commit -qm opt-in
  git -C "$author" push -q
  run "$ENGINE/dot" pull --yes
  [ "$status" -eq 0 ]
  [ "$(cat "$HOME/.config/example.conf")" = new ]
  [ ! -e "$HOME/.config/new.conf" ]
  [ "$public" = "$(git -C "$ENGINE/../.." rev-parse HEAD)" ]
  cmp "$BATS_TEST_TMPDIR/approval-before" "$HOME/.local/state/dot/adoption.json"
  assert_v3_approval
  python3 - "$HOME/.local/state/dot/adoption.json" <<'PY'
import pathlib,sys
assert pathlib.Path(sys.argv[1]).stat().st_mode & 0o777 == 0o640
PY
}

@test "v3 pull refuses widened OS capabilities preserving source targets modes and approval" {
  private_remote v3
  before=$(git -C "$SOURCE" rev-parse HEAD)
  python3 - "$BATS_TEST_TMPDIR/private-author/tools/dot/manifest.json" <<'PY'
import json,sys
p=sys.argv[1]; data=json.load(open(p))
system='linux' if sys.platform.startswith('linux') else sys.platform
data['tracking']['example']['os_metadata'][system]['tools'].append('new-capability')
data['entries']['example@'+system]['tools'].append('new-capability')
json.dump(data,open(p,'w'))
PY
  git -C "$BATS_TEST_TMPDIR/private-author" -c user.name=Fixture -c user.email=fixture@example.invalid commit -qam capability
  git -C "$BATS_TEST_TMPDIR/private-author" push -q
  mode=$(python3 -c 'import os,sys;print(os.stat(sys.argv[1]).st_mode)' "$HOME/.config/example.conf")
  run "$ENGINE/dot" pull --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'fresh adoption approval required'* ]]
  [ "$before" = "$(git -C "$SOURCE" rev-parse HEAD)" ]
  [ "$(cat "$HOME/.config/example.conf")" = old ]
  [ "$mode" = "$(python3 -c 'import os,sys;print(os.stat(sys.argv[1]).st_mode)' "$HOME/.config/example.conf")" ]
  cmp "$BATS_TEST_TMPDIR/approval-before" "$HOME/.local/state/dot/adoption.json"
}

@test "v3 pull detects target bytes modes or approval changing during review before source mutation" {
  private_remote v3
  before=$(git -C "$SOURCE" rev-parse HEAD)
  public=$(git -C "$ENGINE/../.." rev-parse HEAD)
  export REAL_GIT="$(command -v git)" REVIEW_COUNTER="$BATS_TEST_TMPDIR/counter" REVIEW_DEBUG="$BATS_TEST_TMPDIR/review-debug"
  export REVIEW_SOURCE="$(cd "$SOURCE" && pwd -P)"
  mkdir -p "$BATS_TEST_TMPDIR/review-bin"
  cat > "$BATS_TEST_TMPDIR/review-bin/git" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
has_checkout=no; has_status=no
for arg in "$@"; do
  [ "$arg" != "$REVIEW_SOURCE" ] || has_checkout=yes
  [ "$arg" != status ] || has_status=yes
done
printf 'shim_call checkout_arg=%s status_arg=%s\n' "$has_checkout" "$has_status" >> "$REVIEW_DEBUG"
"$REAL_GIT" "$@"
# The second private status is after candidate preview and before revalidation.
if [[ " $* " == *" -C $REVIEW_SOURCE status "* ]]; then
  count=0
  [ ! -f "$REVIEW_COUNTER" ] || count=$(cat "$REVIEW_COUNTER")
  count=$((count + 1)); printf '%s\n' "$count" > "$REVIEW_COUNTER"
  printf 'status_match=1 count=%s change=%s\n' "$count" "$REVIEW_CHANGE" >> "$REVIEW_DEBUG"
  if [ "$count" = 2 ]; then
    case "$REVIEW_CHANGE" in
      bytes) printf 'concurrent target\n' > "$HOME/.config/example.conf"; printf 'mutation=bytes\n' >> "$REVIEW_DEBUG" ;;
      mode) chmod 600 "$HOME/.config/example.conf"; printf 'mutation=mode\n' >> "$REVIEW_DEBUG" ;;
      approval) printf '\n' >> "$HOME/.local/state/dot/adoption.json"; printf 'mutation=approval\n' >> "$REVIEW_DEBUG" ;;
    esac
    cp -p "$HOME/.config/example.conf" "$REVIEW_COUNTER.target"
    cp "$HOME/.local/state/dot/adoption.json" "$REVIEW_COUNTER.approval"
  fi
fi
SH
  chmod +x "$BATS_TEST_TMPDIR/review-bin/git"
  for change in bytes mode approval; do
    : > "$REVIEW_DEBUG"
    rm -f "$REVIEW_COUNTER.target" "$REVIEW_COUNTER.approval"
    printf 'old\n' > "$HOME/.config/example.conf"
    chmod 644 "$HOME/.config/example.conf"
    cp "$BATS_TEST_TMPDIR/approval-before" "$HOME/.local/state/dot/adoption.json"
    printf '0\n' > "$REVIEW_COUNTER"
    run env PATH="$BATS_TEST_TMPDIR/review-bin:$PATH" REVIEW_CHANGE="$change" "$ENGINE/dot" pull --yes
    if [ ! -f "$REVIEW_COUNTER.target" ]; then
      printf '[DEBUG-rc2] fixture_mutated=no change=%s pull_rc=%s\n' "$change" "$status" >&2
      cat "$REVIEW_DEBUG" >&2
    fi
    if [ "$status" -eq 0 ]; then
      printf '[DEBUG-rc2] pull_rc=%s change=%s target_exists=%s approval_sha256=%s\n' \
        "$status" "$change" "$(test -e "$HOME/.config/example.conf" && echo yes || echo no)" \
        "$(shasum -a 256 "$HOME/.local/state/dot/adoption.json" | cut -d ' ' -f 1)" >&2
      cat "$REVIEW_DEBUG" >&2
    fi
    [ -f "$REVIEW_COUNTER.target" ]
    [ "$status" -ne 0 ]
    [ "$before" = "$(git -C "$SOURCE" rev-parse HEAD)" ]
    [ "$public" = "$(git -C "$ENGINE/../.." rev-parse HEAD)" ]
    cmp "$REVIEW_COUNTER.target" "$HOME/.config/example.conf"
    cmp "$REVIEW_COUNTER.approval" "$HOME/.local/state/dot/adoption.json"
    python3 - "$REVIEW_COUNTER.target" "$HOME/.config/example.conf" <<'PY'
import os,sys
assert os.stat(sys.argv[1]).st_mode == os.stat(sys.argv[2]).st_mode
PY
  done
}
