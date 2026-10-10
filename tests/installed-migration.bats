setup() {
  REPO="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
  BATS_TEST_TMPDIR="$(cd "$BATS_TEST_TMPDIR" && pwd -P)"
  export HOME="$BATS_TEST_TMPDIR/home" DOT_SOURCE="$BATS_TEST_TMPDIR/home/.local/share/legacy with spaces"
  export DOT_CONFIG="$HOME/.config/chezmoi/chezmoi.toml"
  unset DOT_ENGINE
  DOT="$REPO/tools/dot/dot"
  CANDIDATE="$BATS_TEST_TMPDIR/public with spaces"
  mkdir -p "$BATS_TEST_TMPDIR/no-provision"
  for command in brew apt pacman yay systemctl launchctl chezmoi; do
    printf '#!/bin/sh\necho forbidden-provisioning > "$HOME/provisioning-called"\nexit 97\n' > "$BATS_TEST_TMPDIR/no-provision/$command"
    chmod +x "$BATS_TEST_TMPDIR/no-provision/$command"
  done
  export PATH="$BATS_TEST_TMPDIR/no-provision:$PATH"
  mkdir -p "$DOT_SOURCE/home" "$DOT_SOURCE/tools/dot" "$CANDIDATE/tools/dot" "$HOME/.config/chezmoi" "$HOME/.local/bin"
  cp "$REPO/tools/dot/"*.py "$REPO/tools/dot/dot" "$REPO/tools/dot/engine.json" "$CANDIDATE/tools/dot/"
  printf '{"protocol":1,"engine_api":{"min":1,"max":1},"manifest_version":1,"approval_version":2,"manifest":"tools/dot/manifest.json","home_source":"home","required_adapters":["intent","scope","gitconfig","skills","layout"]}\n' > "$DOT_SOURCE/dot-contract.json"
  printf '{"version":1,"entries":{},"profiles":{}}\n' > "$DOT_SOURCE/tools/dot/manifest.json"
  cp "$REPO/tools/dot/"*.py "$REPO/tools/dot/dot" "$REPO/tools/dot/engine.json" "$DOT_SOURCE/tools/dot/"
  cp "$BATS_TEST_DIRNAME/fixtures/chezmoiignore.tmpl" "$DOT_SOURCE/home/.chezmoiignore"
  for path in "$DOT_SOURCE" "$CANDIDATE"; do
    git init -q "$path"
    git -C "$path" config user.name Fixture
    git -C "$path" config user.email fixture@example.org
    git -C "$path" add .
    git -C "$path" commit -qm fixture
  done
  git init -q --bare "$BATS_TEST_TMPDIR/public-remote"
  git -C "$CANDIDATE" remote add origin "$BATS_TEST_TMPDIR/public-remote"
  git -C "$CANDIDATE" push -qu origin HEAD
  mkdir -p "$HOME/.local/state/dot"
  printf '{"version":2,"profile":"fixture","entries":{}}\n' > "$HOME/.local/state/dot/adoption.json"
  DOT="$CANDIDATE/tools/dot/dot"
  printf 'sourceDir = "%s/home" # keep\r\n[data]\r\nname="Fixture"\r\n' "$DOT_SOURCE" > "$DOT_CONFIG"
  python3 - "$DOT_SOURCE" "$DOT_CONFIG" "$HOME" <<'PY'
import json,pathlib,shlex,sys
source,config,home=sys.argv[1:]
p=pathlib.Path(home)
(p/'.local/state/dot/install.json').write_text(json.dumps(dict(version=1,directory=source,source='fixture-private',config=config,cache='keep-cache',persistent_state='keep-state')))
(p/'.local/bin/dot').write_text('#!/usr/bin/env bash\nexport DOT_SOURCE='+shlex.quote(source)+'\nexport DOT_CONFIG='+shlex.quote(config)+'\nexec '+shlex.quote(source+'/tools/dot/dot')+' "$@"\n')
PY
  chmod 755 "$HOME/.local/bin/dot"
  PLAN="$BATS_TEST_TMPDIR/plan.json"
}
approve_migration() {
  token="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["id"])' "$PLAN")"
  "$DOT" migrate apply "$PLAN" --approve "$token"
}
@test "approved installed split preserves private Git dirty bytes modes symlinks and exact scope" {
  printf 'local commit\n' > "$DOT_SOURCE/local"
  git -C "$DOT_SOURCE" add local
  git -C "$DOT_SOURCE" commit -qm ahead
  printf 'staged\000bytes' > "$DOT_SOURCE/local"
  git -C "$DOT_SOURCE" add local
  printf 'dirty\000bytes' >> "$DOT_SOURCE/local"
  chmod 751 "$DOT_SOURCE/local"
  ln -s local "$DOT_SOURCE/link"
  cp "$HOME/.local/state/dot/adoption.json" "$BATS_TEST_TMPDIR/scope"
  "$HOME/.local/bin/dot" contract check --engine "$CANDIDATE/tools/dot" --source "$DOT_SOURCE"
  before="$(git -C "$DOT_SOURCE" rev-parse HEAD)"
  "$DOT" migrate plan --engine "$CANDIDATE" --out "$PLAN"
  run "$DOT" migrate apply "$PLAN"
  [ "$status" -ne 0 ]
  approve_migration
  new="$HOME/.local/share/dot-config"
  [ "$(git -C "$new" rev-parse HEAD)" = "$before" ]
  cmp "$DOT_SOURCE/local" "$new/local"
  cmp "$DOT_SOURCE/.git/index" "$new/.git/index"
  [ "$(readlink "$new/link")" = local ]
  python3 -c 'import pathlib,stat,sys; assert stat.S_IMODE(pathlib.Path(sys.argv[1]).stat().st_mode)==0o751' "$new/local"
  cmp "$HOME/.local/state/dot/adoption.json" "$BATS_TEST_TMPDIR/scope"
  python3 - "$DOT_CONFIG" "$HOME" <<'PY'
import pathlib,sys,tomllib
p,home=map(pathlib.Path,sys.argv[1:])
raw=p.read_bytes()
assert b'# keep\r\n[data]\r\n' in raw and b'name="Fixture"\r\n' in raw
assert tomllib.loads(raw.decode())['data']['dotEngine']==str(home/'.local/share/dot-cli/tools/dot')
PY
  [ -d "$DOT_SOURCE/.git" ]
  [ ! -e "$HOME/.zshrc" ]
  [ ! -e "$HOME/provisioning-called" ]
  "$HOME/.local/bin/dot" contract check --engine "$HOME/.local/share/dot-cli/tools/dot" --source "$new"
  run "$HOME/.local/bin/dot" migrate plan --engine "$CANDIDATE" --out "$BATS_TEST_TMPDIR/repeat.json"
  [ "$status" -eq 0 ]
  [[ "$output" == *'already split'* ]]
  [ ! -e "$BATS_TEST_TMPDIR/repeat.json" ]
  python3 - "$HOME/.local/state/dot/install.json" "$BATS_TEST_TMPDIR/public-remote" <<'PY'
import json,sys
record=json.load(open(sys.argv[1]))
assert record['engine_source']==sys.argv[2]
assert record['source']=='fixture-private'
assert record['cache']=='keep-cache' and record['persistent_state']=='keep-state'
PY
}
@test "clean ahead and diverged local-remotes preserve branches upstream history and recovery data" {
  git init -q --bare "$BATS_TEST_TMPDIR/remote"
  git -C "$DOT_SOURCE" remote add origin "$BATS_TEST_TMPDIR/remote"
  git -C "$DOT_SOURCE" push -qu origin HEAD
  git clone -q "$BATS_TEST_TMPDIR/remote" "$BATS_TEST_TMPDIR/peer"
  mkdir "$HOME/.local/state/dot/layout-recovery"
  printf 'old backup\000bytes' > "$HOME/.local/state/dot/layout-recovery/original"
  ln -s layout-recovery "$HOME/.local/state/dot/old-recovery-link"
  printf 'ignored\n' > "$DOT_SOURCE/.git/info/exclude"
  printf 'ignored bytes\000' > "$DOT_SOURCE/ignored"
  for state in clean ahead diverged; do
    if [ "$state" != clean ]; then
      printf '%s\n' "$state" >> "$DOT_SOURCE/local-work"
      git -C "$DOT_SOURCE" add local-work
      git -C "$DOT_SOURCE" commit -qm "$state"
    fi
    if [ "$state" = diverged ]; then
      printf peer > "$BATS_TEST_TMPDIR/peer/peer-work"
      git -C "$BATS_TEST_TMPDIR/peer" add peer-work
      git -C "$BATS_TEST_TMPDIR/peer" -c user.name=Fixture -c user.email=fixture@example.org commit -qm peer
      git -C "$BATS_TEST_TMPDIR/peer" push -q
      git -C "$DOT_SOURCE" fetch -q
    fi
    oldhead="$(git -C "$DOT_SOURCE" rev-parse HEAD)"
    oldrefs="$(git -C "$DOT_SOURCE" show-ref)"
    oldstatus="$(git -C "$DOT_SOURCE" status --porcelain=v1 --branch)"
    cp -p "$DOT_CONFIG" "$BATS_TEST_TMPDIR/original-config"
    cp -p "$HOME/.local/bin/dot" "$BATS_TEST_TMPDIR/original-launcher"
    cp -p "$HOME/.local/state/dot/install.json" "$BATS_TEST_TMPDIR/original-record"
    "$DOT" migrate plan --engine "$CANDIDATE" --out "$PLAN"
    approve_migration
    new="$HOME/.local/share/dot-config"
    [ "$(git -C "$new" rev-parse HEAD)" = "$oldhead" ]
    [ "$(git -C "$new" show-ref)" = "$oldrefs" ]
    [ "$(git -C "$new" status --porcelain=v1 --branch)" = "$oldstatus" ]
    cmp "$DOT_SOURCE/.git/config" "$new/.git/config"
    cmp "$DOT_SOURCE/ignored" "$new/ignored"
    backup="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["backup"])' "$HOME/.local/state/dot/installed-migration.json")"
    cmp "$backup/state/layout-recovery/original" "$HOME/.local/state/dot/layout-recovery/original"
    [ "$(readlink "$backup/state/old-recovery-link")" = layout-recovery ]
    cp -p "$BATS_TEST_TMPDIR/original-config" "$DOT_CONFIG"
    cp -p "$BATS_TEST_TMPDIR/original-launcher" "$HOME/.local/bin/dot"
    cp -p "$BATS_TEST_TMPDIR/original-record" "$HOME/.local/state/dot/install.json"
    mkdir "$BATS_TEST_TMPDIR/retained-$state"
    mv "$HOME/.local/share/dot-config" "$HOME/.local/share/dot-cli" "$HOME/.local/state/dot/installed-migration.json" "$HOME/.local/state/dot/installed-recovery" "$PLAN" "$BATS_TEST_TMPDIR/retained-$state/"
  done
}
@test "stale source config launcher record approval candidate and destination fail before reference mutation" {
  for kind in source config launcher record approval candidate destination; do
    "$DOT" migrate plan --engine "$CANDIDATE" --out "$PLAN"
    case "$kind" in
      source) path="$DOT_SOURCE/local"; printf dirty > "$path" ;;
      config) path="$DOT_CONFIG"; cp "$path" "$BATS_TEST_TMPDIR/original"; printf '\n# stale\n' >> "$path" ;;
      launcher) path="$HOME/.local/bin/dot"; cp "$path" "$BATS_TEST_TMPDIR/original"; printf '\n# stale\n' >> "$path" ;;
      record) path="$HOME/.local/state/dot/install.json"; cp "$path" "$BATS_TEST_TMPDIR/original"; printf '\n ' >> "$path" ;;
      approval) path="$HOME/.local/state/dot/adoption.json"; cp "$path" "$BATS_TEST_TMPDIR/original"; printf '\n ' >> "$path" ;;
      candidate) path="$CANDIDATE/extra"; printf stale > "$path" ;;
      destination) path="$HOME/.local/share/dot-config"; mkdir "$path" ;;
    esac
    cp "$DOT_CONFIG" "$BATS_TEST_TMPDIR/current-config"
    cp "$HOME/.local/bin/dot" "$BATS_TEST_TMPDIR/current-launcher"
    cp "$HOME/.local/state/dot/install.json" "$BATS_TEST_TMPDIR/current-record"
    run approve_migration
    [ "$status" -ne 0 ]
    cmp "$DOT_CONFIG" "$BATS_TEST_TMPDIR/current-config"
    cmp "$HOME/.local/bin/dot" "$BATS_TEST_TMPDIR/current-launcher"
    cmp "$HOME/.local/state/dot/install.json" "$BATS_TEST_TMPDIR/current-record"
    [ ! -e "$HOME/.local/state/dot/installed-migration.json" ]
    case "$kind" in
      config|launcher|record|approval) cp "$BATS_TEST_TMPDIR/original" "$path" ;;
      destination) rmdir "$path" ;;
      *) rm "$path" ;;
    esac
    rm "$PLAN"
  done
}
@test "legacy root source and colocated engine references migrate without rewriting unrelated config" {
  printf 'sourceDir = "%s" # root\r\ncacheDir = "keep"\r\n[data]\r\ndotEngine = "%s/tools/dot" # old engine\r\nname = "Fixture"\r\n' "$DOT_SOURCE" "$DOT_SOURCE" > "$DOT_CONFIG"
  chmod 640 "$DOT_CONFIG"
  "$DOT" migrate plan --engine "$CANDIDATE" --out "$PLAN"
  approve_migration
  python3 - "$DOT_CONFIG" "$HOME" <<'PY'
import pathlib,stat,sys,tomllib
p,home=map(pathlib.Path,sys.argv[1:])
raw=p.read_bytes(); data=tomllib.loads(raw.decode())
assert data['sourceDir']==str(home/'.local/share/dot-config/home')
assert data['data']['dotEngine']==str(home/'.local/share/dot-cli/tools/dot')
assert b'# root\r\ncacheDir = "keep"\r\n[data]\r\n' in raw
assert b'# old engine\r\nname = "Fixture"\r\n' in raw
assert stat.S_IMODE(p.stat().st_mode)==0o640
PY
}
@test "missing approval boundary config hooks and redirected metadata or destinations stop read-only" {
  mv "$HOME/.local/state/dot/adoption.json" "$BATS_TEST_TMPDIR/scope-original"
  run "$DOT" migrate plan --engine "$CANDIDATE" --out "$PLAN"
  [ "$status" -ne 0 ]
  mv "$BATS_TEST_TMPDIR/scope-original" "$HOME/.local/state/dot/adoption.json"
  cp "$DOT_CONFIG" "$BATS_TEST_TMPDIR/original-config"
  printf '\n[hooks.read-source.pre]\ncommand="forbidden"\n' >> "$DOT_CONFIG"
  run "$DOT" migrate plan --engine "$CANDIDATE" --out "$PLAN"
  [ "$status" -ne 0 ]
  cp "$BATS_TEST_TMPDIR/original-config" "$DOT_CONFIG"
  for path in "$DOT_CONFIG" "$HOME/.local/state/dot/install.json" "$HOME/.local/bin/dot"; do
    mv "$path" "$BATS_TEST_TMPDIR/redirected"
    ln -s "$BATS_TEST_TMPDIR/redirected" "$path"
    run "$DOT" migrate plan --engine "$CANDIDATE" --out "$PLAN"
    [ "$status" -ne 0 ]
    rm "$path"
    mv "$BATS_TEST_TMPDIR/redirected" "$path"
  done
  ln -s "$DOT_SOURCE" "$HOME/.local/share/dot-config"
  run "$DOT" migrate plan --engine "$CANDIDATE" --out "$PLAN"
  [ "$status" -ne 0 ]
  [ ! -e "$HOME/.local/state/dot/installed-migration.json" ]
  cmp "$DOT_CONFIG" "$BATS_TEST_TMPDIR/original-config"
}
@test "interrupted reference switch retains verified backups and refuses retry with actionable recovery" {
  "$DOT" migrate plan --engine "$CANDIDATE" --out "$PLAN"
  cp "$DOT_CONFIG" "$BATS_TEST_TMPDIR/config-original"
  cp "$HOME/.local/bin/dot" "$BATS_TEST_TMPDIR/launcher-original"
  run env DOT_MIGRATION_TEST_STOP=config-written bash -c 'token=$(python3 -c '\''import json,sys; print(json.load(open(sys.argv[1]))["id"])'\'' "$1"); "$2" migrate apply "$1" --approve "$token"' _ "$PLAN" "$DOT"
  [ "$status" -ne 0 ]
  [[ "$output" == *'recovery:'* ]]
  cmp "$HOME/.local/bin/dot" "$BATS_TEST_TMPDIR/launcher-original"
  backup="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["backup"])' "$HOME/.local/state/dot/installed-migration.json")"
  cmp "$backup/config" "$BATS_TEST_TMPDIR/config-original"
  [ -f "$backup/README" ]
  run approve_migration
  [ "$status" -ne 0 ]
  [[ "$output" == *'no unsafe retry'* ]]
}
@test "linked worktrees and external Git objects fail closed without references changed" {
  git -C "$DOT_SOURCE" worktree add -q "$BATS_TEST_TMPDIR/linked" -b linked
  run "$DOT" migrate plan --engine "$CANDIDATE" --out "$PLAN"
  [ "$status" -ne 0 ]
  [[ "$output" == *'shared Git metadata'* ]]
  git -C "$DOT_SOURCE" worktree remove "$BATS_TEST_TMPDIR/linked"
  mv "$DOT_SOURCE/.git/objects" "$BATS_TEST_TMPDIR/objects"
  ln -s "$BATS_TEST_TMPDIR/objects" "$DOT_SOURCE/.git/objects"
  run "$DOT" migrate plan --engine "$CANDIDATE" --out "$PLAN"
  [ "$status" -ne 0 ]
  [[ "$output" == *'external Git metadata'* ]]
  [ ! -e "$HOME/.local/state/dot/installed-migration.json" ]
}
@test "path-conditioned Git identity cannot silently change at the relocated checkout" {
  printf '[user]\nname = Conditional Fixture\n' > "$BATS_TEST_TMPDIR/identity"
  git -C "$DOT_SOURCE" config --local "includeIf.gitdir:$DOT_SOURCE/.git.path" "$BATS_TEST_TMPDIR/identity"
  [ "$(git -C "$DOT_SOURCE" config user.name)" = 'Conditional Fixture' ]
  "$DOT" migrate plan --engine "$CANDIDATE" --out "$PLAN"
  cp "$DOT_CONFIG" "$BATS_TEST_TMPDIR/config-original"
  run approve_migration
  [ "$status" -ne 0 ]
  [[ "$output" == *'Git identity/configuration'* ]]
  cmp "$DOT_CONFIG" "$BATS_TEST_TMPDIR/config-original"
}
@test "relative remote locations refuse relocation rather than silently changing Git identity" {
  git -C "$DOT_SOURCE" remote add secondary ../relative-private
  run "$DOT" migrate plan --engine "$CANDIDATE" --out "$PLAN"
  [ "$status" -ne 0 ]
  [[ "$output" == *'relative Git remote'* ]]
  [ ! -e "$HOME/.local/state/dot/installed-migration.json" ]
}
@test "every interrupted boundary journals recovery and refuses unsafe retries without discarding old work" {
  for stage in snapshotting snapshots-verified private-copied public-copied references-ready config-written launcher-written record-written; do
    "$DOT" migrate plan --engine "$CANDIDATE" --out "$PLAN"
    cp -p "$DOT_CONFIG" "$BATS_TEST_TMPDIR/original-config"
    cp -p "$HOME/.local/bin/dot" "$BATS_TEST_TMPDIR/original-launcher"
    cp -p "$HOME/.local/state/dot/install.json" "$BATS_TEST_TMPDIR/original-record"
    export DOT_MIGRATION_TEST_STOP="$stage"
    run approve_migration
    unset DOT_MIGRATION_TEST_STOP
    [ "$status" -ne 0 ]
    [[ "$output" == *'recovery:'* ]]
    run approve_migration
    [ "$status" -ne 0 ]
    [[ "$output" == *'no unsafe retry'* ]]
    [ -d "$DOT_SOURCE/.git" ]
    [ ! -e "$HOME/.zshrc" ]
    # Owner-supervised fixture recovery: restore references and quarantine new work, never delete it.
    cp -p "$BATS_TEST_TMPDIR/original-config" "$DOT_CONFIG"
    cp -p "$BATS_TEST_TMPDIR/original-launcher" "$HOME/.local/bin/dot"
    cp -p "$BATS_TEST_TMPDIR/original-record" "$HOME/.local/state/dot/install.json"
    mkdir "$BATS_TEST_TMPDIR/recovery-$stage"
    for name in dot-config dot-cli; do
      [ ! -e "$HOME/.local/share/$name" ] || mv "$HOME/.local/share/$name" "$BATS_TEST_TMPDIR/recovery-$stage/"
    done
    mv "$HOME/.local/state/dot/installed-migration.json" "$HOME/.local/state/dot/installed-recovery" "$PLAN" "$BATS_TEST_TMPDIR/recovery-$stage/"
  done
  "$DOT" migrate plan --engine "$CANDIDATE" --out "$PLAN"
  approve_migration
}
