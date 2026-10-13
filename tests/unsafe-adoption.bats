#!/usr/bin/env bats
# Real public entrypoints, real Git/chezmoi; fixture HOME and command traces only.
setup() {
  REPO="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
  export HOME="$BATS_TEST_TMPDIR/home" DOT_SOURCE="$BATS_TEST_TMPDIR/source"
  export DOT_ENGINE="$REPO/tools/dot"
  export DOT_CONFIG="$HOME/.config/chezmoi/chezmoi.toml" REAL_PYTHON="$(command -v python3)"
  mkdir -p "$HOME/.config/chezmoi" "$BATS_TEST_TMPDIR/bin"
  printf '[data]\nname="Fixture Name"\nemail="fixture@example.org"\nprofile="personal"\ndotEngine="%s"\n' "$DOT_ENGINE" > "$DOT_CONFIG"
  python3 "$BATS_TEST_DIRNAME/helpers/private-config.py" --source "$DOT_SOURCE" \
    --fixtures "$BATS_TEST_DIRNAME/fixtures"
  cp "$REPO/tools/dot/"*.py "$DOT_SOURCE/tools/dot/"
  cp "$BATS_TEST_DIRNAME/fixtures/chezmoiignore.tmpl" "$DOT_SOURCE/home/.chezmoiignore"
  cp "$BATS_TEST_DIRNAME/fixtures/chezmoiremove.tmpl" "$DOT_SOURCE/home/.chezmoiremove"
  git init -q "$DOT_SOURCE"
  git -C "$DOT_SOURCE" config user.name Test
  git -C "$DOT_SOURCE" config user.email test@example.org
  git -C "$DOT_SOURCE" add .
  git -C "$DOT_SOURCE" commit -qm fixture
  DOT="$REPO/tools/dot/dot"
  REQUEST="$BATS_TEST_TMPDIR/request.json" PLAN="$BATS_TEST_TMPDIR/plan.json"
  export TRACE="$BATS_TEST_TMPDIR/runtime-trace" FIXTURE_OS=linux
  # Only replace the public host OS probe, not engine functions/validation.
  cat > "$BATS_TEST_TMPDIR/bin/python3" <<'SH'
#!/usr/bin/env bash
exec "$REAL_PYTHON" -c '
import os, runpy, sys
args = sys.argv[1:]
if args and args[0] == "-B":
    args = args[1:]
    sys.dont_write_bytecode = True
if args and args[0].endswith(("/intent.py", "/scope.py", "/browser_api.py", "/skills.py", "/gitconfig.py")):
    sys.platform = os.environ["FIXTURE_OS"]
    sys.argv = args
    sys.path.insert(0, os.path.dirname(args[0]))
    runpy.run_path(args[0], run_name="__main__")
else:
    os.execv(os.environ["REAL_PYTHON"], [os.environ["REAL_PYTHON"], *sys.argv[1:]])
' "$@"
SH
  chmod +x "$BATS_TEST_TMPDIR/bin/python3"
  for tool in systemctl launchctl sudo apt apt-get pacman brew gh ghostty Hyprland; do
    printf '#!/bin/sh\nprintf "%%s\n" "%s $*" >> "$TRACE"\nexit 97\n' "$tool" > "$BATS_TEST_TMPDIR/bin/$tool"
    chmod +x "$BATS_TEST_TMPDIR/bin/$tool"
  done
  export PATH="$BATS_TEST_TMPDIR/bin:$PATH"
}
request() { printf '{"version":1,"mode":"adopt","profile":"fixture","entries":%s}\n' "$1" > "$REQUEST"; }
approve() {
  TOKEN="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["id"])' "$PLAN")"
  "$DOT" intent apply "$PLAN" --approve "$TOKEN" "$@"
}
native() {
  chezmoi --source "$DOT_SOURCE/home" --destination "$HOME" --config "$DOT_CONFIG" --override-data "{\"chezmoi\":{\"os\":\"$FIXTURE_OS\"}}" "$@"
}
local_machine() {
  python3 - <<'PY' | python3 "$DOT_ENGINE/browser_api.py"
import json, os
print(json.dumps(dict(version=1, command='init-machine', engine=os.environ['DOT_ENGINE'],
    source=os.environ['DOT_SOURCE'], home=os.environ['HOME'], request=dict(action='save', id='fixture_machine'))))
PY
}

@test "Unsafe shell selection adds simple gt aliases for Bash and Zsh on both OSes only after consent" {
  for system in darwin linux; do
    export FIXTURE_OS="$system"
    printf 'local bash\n' > "$HOME/.bashrc"
    printf 'local zsh\n' > "$HOME/.zshrc"
    request '[".bashrc",".bash_profile",".zshrc"]'
    run "$DOT" intent plan "$REQUEST" --out "$PLAN"
    [ "$status" -eq 0 ] || { echo "$output"; return 1; }
    [[ "$output" == *'Shell impact: whole-file startup replacement'* ]]
    run approve
    [ "$status" -ne 0 ]
    [[ "$output" == *'--ack-unsafe'* ]]
    [ "$(cat "$HOME/.bashrc")" = 'local bash' ]
    approve --ack-unsafe
    run bash --noprofile --norc -c 'source "$HOME/.bashrc"; alias gt'
    [ "$status" -eq 0 ]
    [[ "$output" == *"gt='lazygit'"* ]]
    run bash --noprofile --norc -c 'source "$HOME/.bash_profile"; alias gt'
    [ "$status" -eq 0 ]
    [[ "$output" == *"gt='lazygit'"* ]]
    # Limit lookup to fixture tools when sourcing Zsh; no installed init hooks.
    mkdir -p "$BATS_TEST_TMPDIR/shell-bin"
    ln -sf "$(command -v zsh)" "$BATS_TEST_TMPDIR/shell-bin/zsh"
    run env PATH="$BATS_TEST_TMPDIR/shell-bin" zsh -dfc 'source "$HOME/.zshrc"; alias gt'
    [ "$status" -eq 0 ] || { echo "$output"; return 1; }
    [[ "$output" == *'gt=lazygit'* ]]
    [ ! -e "$HOME/.local/bin/gt" ]
    [ ! -e "$TRACE" ]
  done
}

@test "desktop and service OS metadata require independent compatibility acknowledgment and never activate runtime" {
  request '[".config/hypr/monitors.conf",".config/systemd/user/xremap.service"]'
  export FIXTURE_OS=darwin
  run "$DOT" intent plan "$REQUEST" --out "$PLAN"
  [ "$status" -ne 0 ]
  [[ "$output" == *'not applicable to this OS'* ]]
  export FIXTURE_OS=linux
  run "$DOT" intent plan "$REQUEST" --out "$PLAN"
  [ "$status" -eq 0 ] || { echo "$output"; return 1; }
  [[ "$output" == *'Ubuntu compatibility is not promised'* ]]
  [[ "$output" == *'Configuration only'* ]]
  run approve --ack-unsafe
  [ "$status" -ne 0 ]
  [[ "$output" == *'--ack-desktop'* ]]
  [ ! -e "$HOME/.config/hypr/monitors.conf" ]
  approve --ack-unsafe --ack-desktop
  [ -f "$HOME/.config/systemd/user/xremap.service" ]
  [ ! -e "$HOME/.config/systemd/user/graphical-session.target.wants" ]
  native apply --force
  [ ! -e "$TRACE" ]
  export FIXTURE_OS=darwin
  request '["Library/LaunchAgents/com.dotfiles.drift.plist"]'
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve --ack-unsafe
  native apply --force
  [ -f "$HOME/Library/LaunchAgents/com.dotfiles.drift.plist" ]
  [ ! -e "$TRACE" ]
}

@test "bounded Git edits preserve credential signing includes and unrelated settings without disclosing them" {
  cat > "$HOME/.gitconfig" <<'GIT'
# unrelated comment stays byte-identical
[user]
  name = Local Name
  email = local@example.org
  signingkey = fixture-signing-key
[credential]
  helper = !false
  password = fixture-secret-for-test
[commit]
  gpgsign = true
[include]
  path = ~/not-read.conf
[core]
  hooksPath = ~/not-executed
[alias]
  unchanged = !false
GIT
  printf '[invalid included data\n' > "$HOME/not-read.conf"
  cp "$HOME/.gitconfig" "$BATS_TEST_TMPDIR/original"
  # Repo inspection must not consult fixture global includes either.
  export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
  request '[".gitconfig"]'
  run "$DOT" intent plan "$REQUEST" --out "$PLAN"
  [ "$status" -eq 0 ] || { echo "$output"; return 1; }
  [[ "$output" == *'Git key user.name:'* ]]
  [[ "$output" != *'fixture-secret-for-test'* ]]
  ! grep -q fixture-secret-for-test "$PLAN"
  [ ! -e "$HOME/.local/state/dot/recovery" ]
  cmp "$HOME/.gitconfig" "$BATS_TEST_TMPDIR/original"
  approve --ack-unsafe
  [ "$(git config --no-includes --file "$HOME/.gitconfig" user.name)" = 'Fixture Name' ]
  [ "$(git config --no-includes --file "$HOME/.gitconfig" user.email)" = fixture@example.org ]
  grep -q 'password = fixture-secret-for-test' "$HOME/.gitconfig"
  grep -q 'hooksPath = ~/not-executed' "$HOME/.gitconfig"
  grep -q 'path = ~/not-read.conf' "$HOME/.gitconfig"
  backup="$(find "$HOME/.local/state/dot/recovery" -type f -name 0 | head -1)"
  cmp "$backup" "$BATS_TEST_TMPDIR/original"
  [ "$(stat -f %Lp "$backup" 2>/dev/null || stat -c %a "$backup")" = 600 ]
  # Native rendering previews no backups, then direct apply uses identical edits.
  git config --no-includes --file "$HOME/.gitconfig" user.name 'Native drift'
  cp "$HOME/.gitconfig" "$BATS_TEST_TMPDIR/native-before"
  native status
  [ ! -e "$HOME/.local/state/dot/native-git-recovery" ]
  run native diff
  [ "$status" -ne 0 ]
  [[ "$output" == *'Native Git diff withheld'* ]]
  [[ "$output" != *'fixture-secret-for-test'* ]]
  for flag in --dry-run --dry-run=true -n -n=true -nv -kn; do
    run native apply "$flag"
    [ "$status" -ne 0 ]
    [[ "$output" == *'Native Git diff withheld'* ]] || { echo "$output"; return 1; }
    [[ "$output" != *'fixture-secret-for-test'* ]]
    [ ! -e "$HOME/.local/state/dot/native-git-recovery" ]
    cmp "$HOME/.gitconfig" "$BATS_TEST_TMPDIR/native-before"
  done
  for flag in --verbose -v --verbose=true -v=true -kv --debug=1 --debug=TRUE --verbose=T; do
    run native apply "$flag" --force
    [ "$status" -ne 0 ]
    [[ "$output" == *'Native Git diff withheld'* ]] || { echo "$output"; return 1; }
    [[ "$output" != *'fixture-secret-for-test'* ]]
    [ ! -e "$HOME/.local/state/dot/native-git-recovery" ]
    cmp "$HOME/.gitconfig" "$BATS_TEST_TMPDIR/native-before"
  done
  printf '\nverbose = true\n' > "$BATS_TEST_TMPDIR/verbose.toml"
  cat "$DOT_CONFIG" >> "$BATS_TEST_TMPDIR/verbose.toml"
  run chezmoi --source "$DOT_SOURCE/home" --destination "$HOME" --config "$BATS_TEST_TMPDIR/verbose.toml" apply --force
  [ "$status" -ne 0 ]
  [[ "$output" == *'Native Git diff withheld'* ]] || { echo "$output"; return 1; }
  [[ "$output" != *'fixture-secret-for-test'* ]]
  [ ! -e "$HOME/.local/state/dot/native-git-recovery" ]
  native apply --force
  [ "$(git config --no-includes --file "$HOME/.gitconfig" user.name)" = 'Fixture Name' ]
  backup="$(find "$HOME/.local/state/dot/native-git-recovery" -name config | head -1)"
  cmp "$backup" "$BATS_TEST_TMPDIR/native-before"
  [ ! -e "$TRACE" ]
}

@test "Git ambiguity and selected-key expansion fail closed through agent and native entrypoints" {
  printf '[user]\nname = one\nname = two\n' > "$HOME/.gitconfig"
  request '[".gitconfig"]'
  run "$DOT" intent plan "$REQUEST" --out "$PLAN"
  [ "$status" -ne 0 ]
  [[ "$output" == *'ambiguous repeated selected Git key'* ]]
  printf '[user]\nname = local\n' > "$HOME/.gitconfig"
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve --ack-unsafe
  python3 - "$DOT_SOURCE/tools/dot/manifest.json" <<'PY'
import json, sys
p = sys.argv[1]; d = json.load(open(p)); d['entries']['.gitconfig']['git_keys'].append('core.editor')
open(p, 'w').write(json.dumps(d))
PY
  git -C "$DOT_SOURCE" add .
  git -C "$DOT_SOURCE" commit -qm expansion
  run "$DOT" intent update --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'only reviewed Git keys'* ]]
  run native apply --force
  [ "$status" -ne 0 ]
  [[ "$output" == *'only reviewed Git keys'* ]]
  [ "$(git config --no-includes --file "$HOME/.gitconfig" user.name)" = 'Fixture Name' ]
}

@test "Git preview never executes target-global included fsmonitor or configured helpers" {
  cat > "$HOME/.gitconfig" <<GIT
[user]
name = Local
[include]
path = $HOME/included.conf
[credential]
helper = !$BATS_TEST_TMPDIR/bin/configured-command
[core]
hooksPath = $HOME/hooks
GIT
  printf '[core]\nfsmonitor = %s\n' "$BATS_TEST_TMPDIR/bin/configured-command" > "$HOME/included.conf"
  printf '#!/bin/sh\necho configured-command >> "$TRACE"\nexit 97\n' > "$BATS_TEST_TMPDIR/bin/configured-command"
  chmod +x "$BATS_TEST_TMPDIR/bin/configured-command"
  request '[".gitconfig"]'
  run "$DOT" intent plan "$REQUEST" --out "$PLAN"
  [ "$status" -eq 0 ] || { echo "$output"; return 1; }
  run "$DOT" intent preview "$PLAN"
  [ "$status" -eq 0 ] || { echo "$output"; return 1; }
  [ ! -e "$TRACE" ]
  [ ! -e "$HOME/.local/state/dot/recovery" ]
}

@test "native Git permission changes fail before any selected file or recovery mutation; intent preserves mode" {
  for mode in 600 666; do
    printf '[user]\nname = Local\n' > "$HOME/.gitconfig"
    chmod "$mode" "$HOME/.gitconfig"
    request '[".gitconfig",".tmux.conf"]'
    "$DOT" intent plan "$REQUEST" --out "$PLAN"
    approve --ack-unsafe
    [ "$(stat -f %Lp "$HOME/.gitconfig" 2>/dev/null || stat -c %a "$HOME/.gitconfig")" = "$mode" ]
    printf '\n# intended content update\n' >> "$DOT_SOURCE/home/.chezmoitemplates/gitconfig.tmpl"
    git -C "$DOT_SOURCE" add .
    git -C "$DOT_SOURCE" commit -qm content
    "$DOT" intent update --yes
    [ "$(stat -f %Lp "$HOME/.gitconfig" 2>/dev/null || stat -c %a "$HOME/.gitconfig")" = "$mode" ]
    git config --no-includes --file "$HOME/.gitconfig" user.name 'native drift'
    printf 'unapplied tmux drift\n' > "$HOME/.tmux.conf"
    cp "$HOME/.gitconfig" "$BATS_TEST_TMPDIR/permission-before"
    run native apply --force
    [ "$status" -ne 0 ]
    [[ "$output" == *'unapproved Git permission change'* ]]
    cmp "$HOME/.gitconfig" "$BATS_TEST_TMPDIR/permission-before"
    [ "$(cat "$HOME/.tmux.conf")" = 'unapplied tmux drift' ]
    [ ! -e "$HOME/.local/state/dot/native-git-recovery" ]
    [ "$(stat -f %Lp "$HOME/.gitconfig" 2>/dev/null || stat -c %a "$HOME/.gitconfig")" = "$mode" ]
  done
}

@test "mixed Safe Unsafe update leaves unselected startup desktop runtime and future profiles untouched" {
  load update-helper
  independent_update_engine
  mkdir -p "$HOME/.config/hypr"
  printf 'local startup\n' > "$HOME/.zshrc"
  printf 'local desktop\n' > "$HOME/.config/hypr/monitors.conf"
  printf 'runtime\n' > "$HOME/.config/hypr/auth.json"
  request '[".tmux.conf",".config/eza/theme.yml"]'
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve --ack-unsafe
  git clone -q --bare "$DOT_SOURCE" "$BATS_TEST_TMPDIR/upstream.git"
  git -C "$DOT_SOURCE" remote add origin "$BATS_TEST_TMPDIR/upstream.git"
  git -C "$DOT_SOURCE" fetch -q origin
  branch="$(git -C "$DOT_SOURCE" branch --show-current)"
  git -C "$DOT_SOURCE" branch --set-upstream-to="origin/$branch" "$branch"
  git clone -q "$BATS_TEST_TMPDIR/upstream.git" "$BATS_TEST_TMPDIR/upstream"
  git -C "$BATS_TEST_TMPDIR/upstream" config user.name Test
  git -C "$BATS_TEST_TMPDIR/upstream" config user.email test@example.org
  printf '\n# selected update\n' >> "$BATS_TEST_TMPDIR/upstream/home/dot_tmux.conf"
  printf 'future upstream desktop\n' > "$BATS_TEST_TMPDIR/upstream/home/dot_config/hypr/monitors.conf"
  printf 'future upstream startup\n' > "$BATS_TEST_TMPDIR/upstream/home/dot_zshrc.tmpl"
  git -C "$BATS_TEST_TMPDIR/upstream" add .
  git -C "$BATS_TEST_TMPDIR/upstream" commit -qm update
  git -C "$BATS_TEST_TMPDIR/upstream" push -q
  "$DOT" pull --yes
  grep -q 'selected update' "$HOME/.tmux.conf"
  native apply --force
  [ "$(cat "$HOME/.zshrc")" = 'local startup' ]
  [ "$(cat "$HOME/.config/hypr/monitors.conf")" = 'local desktop' ]
  [ "$(cat "$HOME/.config/hypr/auth.json")" = runtime ]
  [ ! -e "$TRACE" ]
}

@test "Git key subsets preserve unselected identity and payload additions remain opt-in" {
  local_machine
  printf '[user]\nname = Local\nemail = keep@example.org\n' > "$HOME/.gitconfig"
  printf '{"version":1,"mode":"adopt","profile":"fixture","entries":[".gitconfig"],"metadata":{".gitconfig":{"git_keys":["user.name"]}}}\n' > "$REQUEST"
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve --ack-unsafe
  printf '\n[core]\neditor = fixture-editor\n' >> "$DOT_SOURCE/home/.chezmoitemplates/gitconfig.tmpl"
  git -C "$DOT_SOURCE" add .
  git -C "$DOT_SOURCE" commit -qm preference
  "$DOT" intent update --yes
  native apply --force
  [ "$(git config --no-includes --file "$HOME/.gitconfig" user.name)" = 'Fixture Name' ]
  [ "$(git config --no-includes --file "$HOME/.gitconfig" user.email)" = keep@example.org ]
  run git config --no-includes --file "$HOME/.gitconfig" core.editor
  [ "$status" -eq 1 ]
}

@test "Track imports only explicitly selected approved Git keys and cannot remove bounded preservation" {
  export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
  cat > "$HOME/.gitconfig" <<'GIT'
[user]
name = Imported Name
email = do-not-import@example.org
signingkey = do-not-import-key
[credential]
password = do-not-import-secret
[include]
path = ~/do-not-import.conf
GIT
  cp "$HOME/.gitconfig" "$BATS_TEST_TMPDIR/track-before"
  printf '{"version":1,"mode":"track","profile":"tracked-git","selections":[{"path":".gitconfig","git_keys":["user.name"]}]}\n' > "$REQUEST"
  run "$DOT" intent plan "$REQUEST" --out "$PLAN"
  [ "$status" -eq 0 ] || { echo "$output"; return 1; }
  [[ "$output" != *'do-not-import'* ]]
  ! grep -q do-not-import "$PLAN"
  run approve
  [ "$status" -ne 0 ]
  [[ "$output" == *'--ack-unsafe'* ]]
  approve --ack-unsafe
  cmp "$HOME/.gitconfig" "$BATS_TEST_TMPDIR/track-before"
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
  grep -q 'Imported Name' "$DOT_SOURCE/home/.chezmoitemplates/gitconfig.tmpl"
  ! grep -q do-not-import "$DOT_SOURCE/home/.chezmoitemplates/gitconfig.tmpl"
  python3 - "$DOT_SOURCE/tools/dot/manifest.json" <<'PY'
import json, sys
e = json.load(open(sys.argv[1]))['entries']['.gitconfig']
assert e['integrations'] == ['git-config'] and e['impact'] == 'Unsafe'
assert e['git_keys'] == ['user.name', 'user.email']
PY
  for edits in '"impact":"Safe"' '"integrations":[]' '"template":"[user]\nname = replacement"' '"git_keys":["user.name","credential.helper"]'; do
    if [[ "$edits" == '"git_keys"'* ]]; then
      printf '{"version":1,"mode":"track","profile":"bad","selections":[{"path":".gitconfig",%s}]}\n' "$edits" > "$REQUEST"
    else
      printf '{"version":1,"mode":"track","profile":"bad","selections":[{"path":".gitconfig","git_keys":["user.name"],%s}]}\n' "$edits" > "$REQUEST"
    fi
    run "$DOT" intent plan "$REQUEST" --out "$PLAN"
    [ "$status" -ne 0 ]
    cmp "$HOME/.gitconfig" "$BATS_TEST_TMPDIR/track-before"
    [ -z "$(git -C "$DOT_SOURCE" status --porcelain)" ]
  done
  # A raw whole-file import cannot replace the bounded entry either.
  printf '{"version":1,"mode":"track","profile":"bad","selections":[{"path":".gitconfig","os":["darwin","linux"],"portability":"managed-shared","impact":"Unsafe"}]}\n' > "$REQUEST"
  run "$DOT" intent plan "$REQUEST" --out "$PLAN"
  [ "$status" -ne 0 ]
  [[ "$output" == *'explicitly selected approved git_keys'* ]]
}

@test "Git release allowlist rejects every other key before target or metadata mutation" {
  for key in credential.helper imap.pass sendemail.smtppass push.gpgsign include.path core.editor; do
    printf '{"version":1,"mode":"adopt","profile":"bad","entries":[".gitconfig"],"metadata":{".gitconfig":{"git_keys":["%s"]}}}\n' "$key" > "$REQUEST"
    run "$DOT" intent plan "$REQUEST" --out "$PLAN"
    [ "$status" -ne 0 ]
    [[ "$output" == *'only reviewed Git keys'* ]]
    [ ! -e "$HOME/.gitconfig" ]
    [ ! -e "$HOME/.local/state/dot/adoption.json" ]
    [ -z "$(git -C "$DOT_SOURCE" status --porcelain)" ]
  done
}

@test "selected service dependencies are advisory even when absent and never launch or install" {
  mkdir -p "$BATS_TEST_TMPDIR/missing-bin"
  for tool in bash git chezmoi python3 dirname; do
    ln -s "$(command -v "$tool")" "$BATS_TEST_TMPDIR/missing-bin/$tool"
  done
  request '[".config/systemd/user/xremap.service",".config/systemd/user/cliphist.service"]'
  run env PATH="$BATS_TEST_TMPDIR/missing-bin" "$DOT" intent plan "$REQUEST" --out "$PLAN"
  [ "$status" -eq 0 ] || { echo "$output"; return 1; }
  TOKEN="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["id"])' "$PLAN")"
  run env PATH="$BATS_TEST_TMPDIR/missing-bin" "$DOT" intent apply "$PLAN" --approve "$TOKEN" --ack-unsafe --ack-desktop
  [ "$status" -eq 0 ] || { echo "$output"; return 1; }
  [ -f "$HOME/.config/systemd/user/xremap.service" ]
  [ -f "$HOME/.config/systemd/user/cliphist.service" ]
  python3 - "$HOME/.local/state/dot/missing-tools.json" <<'PY'
import json, sys
assert set(json.load(open(sys.argv[1]))) == {'systemctl', 'xremap', 'wl-paste', 'cliphist'}
PY
  [ ! -e "$TRACE" ]
}

@test "Track stable-ID collision cannot replace a bounded Git entry or delete its adapter" {
  python3 - "$DOT_SOURCE/tools/dot/manifest.json" <<'PY'
import json, sys
p = sys.argv[1]; d = json.load(open(p)); d['entries']['git'] = d['entries'].pop('.gitconfig')
for ids in d['profiles'].values():
    ids[:] = ['git' if entry == '.gitconfig' else entry for entry in ids]
d['profiles']['unsafe-git'] = ['git']; open(p, 'w').write(json.dumps(d))
PY
  git -C "$DOT_SOURCE" add .
  git -C "$DOT_SOURCE" commit -qm stable-id
  printf 'eligible preference\n' > "$HOME/git"
  printf '{"version":1,"mode":"track","profile":"collision","selections":[{"path":"git","os":["darwin","linux"],"portability":"managed-shared","impact":"Safe"}]}\n' > "$REQUEST"
  run "$DOT" intent plan "$REQUEST" --out "$PLAN"
  [ "$status" -ne 0 ]
  [[ "$output" == *'cannot overwrite a bounded Git manifest ID'* ]]
  [ -f "$DOT_SOURCE/home/modify_dot_gitconfig.tmpl" ]
  [ -z "$(git -C "$DOT_SOURCE" status --porcelain)" ]
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
  python3 - "$DOT_SOURCE/tools/dot/manifest.json" <<'PY'
import json, sys
p = sys.argv[1]; d = json.load(open(p)); d['entries']['preferences/git'] = d['entries'].pop('git')
for ids in d['profiles'].values():
    ids[:] = ['preferences/git' if entry == 'git' else entry for entry in ids]
d['profiles']['unsafe-git'] = ['preferences/git']; open(p, 'w').write(json.dumps(d))
PY
  git -C "$DOT_SOURCE" add .
  git -C "$DOT_SOURCE" commit -qm nested-stable-id
  mkdir -p "$HOME/preferences"
  printf 'eligible directory snapshot member\n' > "$HOME/preferences/git"
  printf '{"version":1,"mode":"track","profile":"collision","selections":[{"path":"preferences","os":["darwin","linux"],"portability":"managed-shared","impact":"Unsafe"}]}\n' > "$REQUEST"
  run "$DOT" intent plan "$REQUEST" --out "$PLAN"
  [ "$status" -ne 0 ]
  [[ "$output" == *'cannot overwrite a bounded Git manifest ID'* ]]
  [ -f "$DOT_SOURCE/home/modify_dot_gitconfig.tmpl" ]
  [ -z "$(git -C "$DOT_SOURCE" status --porcelain)" ]
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
}

@test "legacy approved Git uses bounded preferences without scope conversion and rejects permission changes before mutation" {
  export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
  mkdir -p "$HOME/.local/state/dot" "$HOME/.config/eza"
  printf '{"version":1,"groups":{"git":[".gitconfig"],"eza":[".config/eza/theme.yml"]}}\n' > "$HOME/.local/state/dot/adoption.json"
  cp "$HOME/.local/state/dot/adoption.json" "$BATS_TEST_TMPDIR/legacy-scope"
  printf '[user]\nname = Legacy\nemail = legacy@example.org\n[credential]\npassword = legacy-fixture-secret\n[include]\npath = ~/not-read.conf\n' > "$HOME/.gitconfig"
  printf 'local eza\n' > "$HOME/.config/eza/theme.yml"
  cp "$HOME/.gitconfig" "$BATS_TEST_TMPDIR/legacy-before"
  native status
  [ ! -e "$HOME/.local/state/dot/native-git-recovery" ]
  run native diff
  [ "$status" -ne 0 ]
  [[ "$output" != *'legacy-fixture-secret'* ]]
  native apply --force
  [ "$(git config --no-includes --file "$HOME/.gitconfig" user.name)" = 'Fixture Name' ]
  grep -q 'password = legacy-fixture-secret' "$HOME/.gitconfig"
  grep -q 'path = ~/not-read.conf' "$HOME/.gitconfig"
  backup="$(find "$HOME/.local/state/dot/native-git-recovery" -name config | head -1)"
  cmp "$backup" "$BATS_TEST_TMPDIR/legacy-before"
  cmp "$HOME/.local/state/dot/adoption.json" "$BATS_TEST_TMPDIR/legacy-scope"
  load update-helper
  independent_update_engine
  git clone -q --bare "$DOT_SOURCE" "$BATS_TEST_TMPDIR/legacy-upstream.git"
  git -C "$DOT_SOURCE" remote add origin "$BATS_TEST_TMPDIR/legacy-upstream.git"
  git -C "$DOT_SOURCE" fetch -q origin
  branch="$(git -C "$DOT_SOURCE" branch --show-current)"
  git -C "$DOT_SOURCE" branch --set-upstream-to="origin/$branch" "$branch"
  git clone -q "$BATS_TEST_TMPDIR/legacy-upstream.git" "$BATS_TEST_TMPDIR/legacy-upstream"
  git -C "$BATS_TEST_TMPDIR/legacy-upstream" config user.name Test
  git -C "$BATS_TEST_TMPDIR/legacy-upstream" config user.email test@example.org
  printf '\n# legacy content update\n' >> "$BATS_TEST_TMPDIR/legacy-upstream/home/.chezmoitemplates/gitconfig.tmpl"
  git -C "$BATS_TEST_TMPDIR/legacy-upstream" add .
  git -C "$BATS_TEST_TMPDIR/legacy-upstream" commit -qm update
  git -C "$BATS_TEST_TMPDIR/legacy-upstream" push -q
  run "$DOT" pull --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'approved exact scope'* ]]
  [[ "$output" != *'legacy-fixture-secret'* ]]
  cmp "$HOME/.local/state/dot/adoption.json" "$BATS_TEST_TMPDIR/legacy-scope"
  chmod 600 "$HOME/.gitconfig"
  printf 'unapplied eza\n' > "$HOME/.config/eza/theme.yml"
  cp "$HOME/.gitconfig" "$BATS_TEST_TMPDIR/legacy-permission-before"
  before_count="$(find "$HOME/.local/state/dot/native-git-recovery" -name plan.json | wc -l | tr -d ' ')"
  run native apply --force
  [ "$status" -ne 0 ]
  [[ "$output" == *'unapproved Git permission change'* ]]
  cmp "$HOME/.gitconfig" "$BATS_TEST_TMPDIR/legacy-permission-before"
  [ "$(cat "$HOME/.config/eza/theme.yml")" = 'unapplied eza' ]
  [ "$(find "$HOME/.local/state/dot/native-git-recovery" -name plan.json | wc -l | tr -d ' ')" = "$before_count" ]
  cmp "$HOME/.local/state/dot/adoption.json" "$BATS_TEST_TMPDIR/legacy-scope"
  [ ! -e "$TRACE" ]
}

@test "Git identity quoting and native argument transport preserve hashes quotes and spaced paths" {
  mv "$DOT_SOURCE" "$BATS_TEST_TMPDIR/source with spaces"
  export DOT_SOURCE="$BATS_TEST_TMPDIR/source with spaces"
  mv "$HOME" "$BATS_TEST_TMPDIR/home with spaces"
  export HOME="$BATS_TEST_TMPDIR/home with spaces" DOT_CONFIG="$BATS_TEST_TMPDIR/home with spaces/.config/chezmoi/chezmoi.toml"
  python3 - "$DOT_CONFIG" "$DOT_ENGINE" <<'PY'
import json, sys
name = 'Jane # "Quoted" Doe'
open(sys.argv[1], 'w').write('[data]\nname=' + json.dumps(name) + '\nemail="jane@example.org"\nprofile="personal"\ndotEngine=' + json.dumps(sys.argv[2]) + '\n')
PY
  request '[".gitconfig"]'
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve --ack-unsafe
  [ "$(git config --no-includes --file "$HOME/.gitconfig" user.name)" = 'Jane # "Quoted" Doe' ]
  git config --no-includes --file "$HOME/.gitconfig" user.name 'native drift'
  native apply --force
  [ "$(git config --no-includes --file "$HOME/.gitconfig" user.name)" = 'Jane # "Quoted" Doe' ]
  [ ! -e "$TRACE" ]
}

@test "native option values are not mistaken for dry-run flags or mutation commands" {
  printf '[user]\nname = Local\n' > "$HOME/.gitconfig"
  chmod 600 "$HOME/.gitconfig"
  request '[".gitconfig"]'
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve --ack-unsafe
  git config --no-includes --file "$HOME/.gitconfig" user.name 'native drift'
  cp "$HOME/.gitconfig" "$BATS_TEST_TMPDIR/argument-before"
  run bash -c 'cd "$BATS_TEST_TMPDIR"; chezmoi --source "$DOT_SOURCE/home" --destination "$HOME" --config "$DOT_CONFIG" --output apply status'
  [ "$status" -eq 0 ] || { echo "$output"; return 1; }
  [ ! -e "$HOME/.local/state/dot/native-git-recovery" ]
  cmp "$HOME/.gitconfig" "$BATS_TEST_TMPDIR/argument-before"
  run bash -c 'cd "$BATS_TEST_TMPDIR"; chezmoi --source "$DOT_SOURCE/home" --destination "$HOME" --config "$DOT_CONFIG" --output -n apply --force'
  [ "$status" -ne 0 ]
  [[ "$output" == *'unapproved Git permission change'* ]] || { echo "$output"; return 1; }
  [ ! -e "$HOME/.local/state/dot/native-git-recovery" ]
  cmp "$HOME/.gitconfig" "$BATS_TEST_TMPDIR/argument-before"
  run native --color auto apply --force
  [ "$status" -ne 0 ]
  [[ "$output" == *'unapproved Git permission change'* ]] || { echo "$output"; return 1; }
  [ ! -e "$HOME/.local/state/dot/native-git-recovery" ]
  cmp "$HOME/.gitconfig" "$BATS_TEST_TMPDIR/argument-before"
  run bash -c 'cd "$BATS_TEST_TMPDIR"; chezmoi --source "$DOT_SOURCE/home" --destination "$HOME" --config "$DOT_CONFIG" --output=--umask status'
  [ "$status" -eq 0 ] || { echo "$output"; return 1; }
}
