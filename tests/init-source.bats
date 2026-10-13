#!/usr/bin/env bats

setup() {
  REPO="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
  export HOME="$BATS_TEST_TMPDIR/home"
  mkdir -p "$HOME"
  ENGINE="$REPO/tools/dot"
  SOURCE="$HOME/source"
}

bridge() {
  local request="$1"
  python3 - "$ENGINE" "$SOURCE" "$HOME" "$request" <<'PY' | python3 "$ENGINE/browser_api.py"
import json,sys
print(json.dumps(dict(version=1,command='init-source',engine=sys.argv[1],source=sys.argv[2],home=sys.argv[3],request=json.loads(sys.argv[4]))))
PY
}

remote_fixture() {
  REMOTE="$HOME/remote"
  mkdir -p "$REMOTE/home"
  printf 'original\n' > "$REMOTE/home/dot_fixture"
  printf '{"protocol":1,"engine_api":{"min":1,"max":1},"manifest_version":1,"approval_version":2,"manifest":"manifest.json","home_source":"home","required_adapters":["intent","scope","gitconfig","skills","layout"]}\n' > "$REMOTE/dot-contract.json"
  printf '{"version":1,"entries":{},"profiles":{}}\n' > "$REMOTE/manifest.json"
  git init -q "$REMOTE"
  git -C "$REMOTE" add .
  git -C "$REMOTE" -c user.name=Fixture -c user.email=fixture@example.invalid commit -qm fixture
}

@test "HTTPS and SSH source clones verify configured origin and contract using plain Git" {
  remote_fixture
  for url in 'https://example.invalid/owner/dots.git' 'git@example.invalid:owner/dots.git' 'ssh://git@example.invalid/owner/dots.git'; do
    git config --global "url.$REMOTE.insteadOf" "$url"
    SOURCE="$HOME/source-${url%%:*}"
    run bridge "{\"action\":\"clone\",\"url\":\"$url\"}"
    [ "$status" -eq 0 ]
    [[ "$output" == *'"valid": true'* ]]
    [ "$(git -C "$SOURCE" config --get remote.origin.url)" = "$url" ]
    [ "$(cat "$SOURCE/home/dot_fixture")" = original ]
  done
}

@test "unsafe URL rejection is secret-safe and creates no source" {
  for url in 'https://user:secret@example.invalid/repo' 'https://secret@example.invalid/repo' 'ssh://git:secret@example.invalid/repo' 'ssh://git:@example.invalid/repo' 'https://example.invalid/repo?secret' 'git@example.invalid:repo#secret' 'https://[secret/repo'; do
    run bridge "{\"action\":\"clone\",\"url\":\"$url\"}"
    [ "$status" -ne 0 ]
    [[ "$output" != *secret* ]]
    [[ "$output" == *'source URL'* ]]
    [ ! -e "$SOURCE" ]
  done
}

@test "clone with a missing contract is retained for review, not accepted or deleted" {
  remote_fixture
  git -C "$REMOTE" rm dot-contract.json
  git -C "$REMOTE" -c user.name=Fixture -c user.email=fixture@example.invalid commit -qm missing-contract
  git config --global "url.$REMOTE.insteadOf" 'https://example.invalid/repo'
  run bridge '{"action":"clone","url":"https://example.invalid/repo"}'
  [ "$status" -ne 0 ]
  [[ "$output" == *'contract'* ]]
  [ -d "$SOURCE/.git" ]
  [ "$(cat "$SOURCE/home/dot_fixture")" = original ]
  run bridge '{"action":"detect"}'
  [ "$status" -eq 0 ]
  [[ "$output" == *'"exists": true'* ]]
  [[ "$output" == *'"valid": false'* ]]
}

@test "clone rejects a changed origin without exposing it or deleting the checkout" {
  remote_fixture
  git config --global "url.$REMOTE.insteadOf" 'https://example.invalid/repo'
  real_git="$(command -v git)"
  mkdir "$HOME/bin"
  cat > "$HOME/bin/git" <<SH
#!/bin/bash
"$real_git" "\$@" || exit \$?
if [ "\$1" = clone ]; then
  "$real_git" -C "\${!#}" remote set-url origin 'https://secret@example.invalid/other'
fi
SH
  chmod +x "$HOME/bin/git"
  export PATH="$HOME/bin:$PATH"
  run bridge '{"action":"clone","url":"https://example.invalid/repo"}'
  [ "$status" -ne 0 ]
  [[ "$output" == *'origin verification failed'* ]]
  [[ "$output" != *secret* ]]
  [ "$(cat "$SOURCE/home/dot_fixture")" = original ]
}

@test "Replace confirmation preserves tracked, untracked, dirty and Git data in named timestamped backup" {
  remote_fixture
  git clone -q "$REMOTE" "$SOURCE"
  printf 'dirty\n' > "$SOURCE/home/dot_fixture"
  printf 'untracked\n' > "$SOURCE/keep-me"
  before="$(git -C "$SOURCE" rev-parse HEAD)"
  run bridge '{"action":"backup-plan"}'
  [ "$status" -eq 0 ]
  [ -f "$SOURCE/keep-me" ]
  plan="$(python3 -c 'import json,sys; print(json.dumps(json.loads(sys.argv[1])["result"]["plan"]))' "$output")"
  backup="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["backup"])' "$plan")"
  [[ "$backup" == "$SOURCE.backup-"*'/checkout' ]]
  [ ! -e "$backup" ]
  run bridge "{\"action\":\"replace\",\"plan\":$plan}"
  [ "$status" -eq 0 ]
  [ ! -e "$SOURCE" ]
  [ "$(cat "$backup/home/dot_fixture")" = dirty ]
  [ "$(cat "$backup/keep-me")" = untracked ]
  [ "$(git -C "$backup" rev-parse HEAD)" = "$before" ]
  run bridge '{"action":"new","url":"https://example.invalid/new-repo"}'
  [ "$status" -eq 0 ]
  [ "$(cat "$backup/keep-me")" = untracked ]
}

@test "existing backups and stale Replace plans are never overwritten" {
  remote_fixture
  git clone -q "$REMOTE" "$SOURCE"
  run bridge '{"action":"backup-plan"}'
  [ "$status" -eq 0 ]
  plan="$(python3 -c 'import json,sys; print(json.dumps(json.loads(sys.argv[1])["result"]["plan"]))' "$output")"
  backup="$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["backup"])' "$plan")"
  mkdir -p "$backup"
  printf keep > "$backup/sentinel"
  run bridge "{\"action\":\"replace\",\"plan\":$plan}"
  [ "$status" -ne 0 ]
  [ "$(cat "$backup/sentinel")" = keep ]
  [ -d "$SOURCE/.git" ]
  mv "$SOURCE" "$HOME/moved-original"
  mkdir "$SOURCE"
  printf replacement > "$SOURCE/sentinel"
  run bridge "{\"action\":\"replace\",\"plan\":$plan}"
  [ "$status" -ne 0 ]
  [[ "$output" == *'stale'* ]]
  [ "$(cat "$SOURCE/sentinel")" = replacement ]
  [ -d "$HOME/moved-original/.git" ]
}

@test "source replacement refuses HOME and symlink destinations" {
  SOURCE="$HOME"
  run bridge '{"action":"backup-plan"}'
  [ "$status" -ne 0 ]
  [ -d "$HOME" ]
  remote_fixture
  ln -s "$REMOTE" "$HOME/source-link"
  SOURCE="$HOME/source-link"
  run bridge '{"action":"backup-plan"}'
  [ "$status" -ne 0 ]
  [ -L "$SOURCE" ]
  [ "$(cat "$REMOTE/home/dot_fixture")" = original ]
}

@test "new source initializes plain Git and origin without hosting or commits" {
  run bridge '{"action":"new","url":"ssh://git@example.invalid/owner/dots.git"}'
  [ "$status" -eq 0 ]
  [ "$(git -C "$SOURCE" remote get-url origin)" = 'ssh://git@example.invalid/owner/dots.git' ]
  run git -C "$SOURCE" rev-parse --verify HEAD
  [ "$status" -ne 0 ]
  run bridge '{"action":"continue"}'
  [ "$status" -eq 0 ]
  [[ "$output" == *'"valid": true'* ]]
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
}
