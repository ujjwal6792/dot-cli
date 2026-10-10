#!/usr/bin/env bats
# Public commands, fixture HOME, real Git/chezmoi. No live adoption/install.
setup() {
  REPO="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
  export HOME="$BATS_TEST_TMPDIR/home" DOT_SOURCE="$BATS_TEST_TMPDIR/source"
  export DOT_ENGINE="$REPO/tools/dot"
  export DOT_CONFIG="$HOME/.config/chezmoi/chezmoi.toml"
  mkdir -p "$HOME/.config/chezmoi" "$DOT_SOURCE/tools/dot" "$DOT_SOURCE/home/dot_agents/skills/example"
  printf '[data]\nname="Test"\nemail="test@example.org"\nprofile="personal"\ndotEngine="%s"\n' "$DOT_ENGINE" > "$DOT_CONFIG"
  cp "$REPO/tools/dot/"*.py "$DOT_SOURCE/tools/dot/"
  printf '{"protocol":1,"engine_api":{"min":1,"max":1},"manifest_version":1,"approval_version":2,"manifest":"tools/dot/manifest.json","home_source":"home","required_adapters":["intent","scope","gitconfig","skills","layout"]}\n' > "$DOT_SOURCE/dot-contract.json"
  cp "$BATS_TEST_DIRNAME/fixtures/chezmoiignore.tmpl" "$DOT_SOURCE/home/.chezmoiignore"
  cp "$BATS_TEST_DIRNAME/fixtures/chezmoiremove.tmpl" "$DOT_SOURCE/home/.chezmoiremove"
  cp "$BATS_TEST_DIRNAME/fixtures/run_after_sync-skills.sh.tmpl" "$DOT_SOURCE/home/run_after_sync-skills.sh.tmpl"
  mkdir -p "$DOT_SOURCE/scripts"
  cp "$REPO/scripts/sync-skills.sh" "$DOT_SOURCE/scripts/"
  printf 'reviewed skill\n' > "$DOT_SOURCE/home/dot_agents/skills/example/SKILL.md"
  cat > "$DOT_SOURCE/tools/dot/manifest.json" <<'JSON'
{"version":1,"entries":{"example":{"destination":".agents/skills/example/SKILL.md","source":"home/dot_agents/skills/example/SKILL.md","os":["darwin","linux"],"portability":"managed-shared","impact":"Safe","removals":[],"hooks":[],"tools":["fixturemissingtool"],"integrations":["gemini-skills"]}},"profiles":{"skills":["example"]}}
JSON
  git init -q "$DOT_SOURCE"
  git -C "$DOT_SOURCE" config user.name Test
  git -C "$DOT_SOURCE" config user.email test@example.org
  git -C "$DOT_SOURCE" add .
  git -C "$DOT_SOURCE" commit -qm fixture
  DOT="$REPO/tools/dot/dot"
  REQUEST="$BATS_TEST_TMPDIR/request.json" PLAN="$BATS_TEST_TMPDIR/plan.json"
  printf '{"version":1,"mode":"adopt","profile":"skills","entries":["example"]}\n' > "$REQUEST"
}
approve() {
  TOKEN="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["id"])' "$PLAN")"
  "$DOT" intent apply "$PLAN" --approve "$TOKEN" "$@"
}

@test "skills adoption previews exact Gemini output and backs up only approved canonical files" {
  mkdir -p "$HOME/.agents/skills/example" "$HOME/.agents/skills/custom" "$HOME/.gemini/skills/custom"
  printf 'old canonical\n' > "$HOME/.agents/skills/example/SKILL.md"
  printf 'unapproved\n' > "$HOME/.agents/skills/custom/SKILL.md"
  printf 'unapproved mirror\n' > "$HOME/.gemini/skills/custom/SKILL.md"
  printf 'runtime\n' > "$HOME/.agents/skills/example/auth.json"
  run "$DOT" intent plan "$REQUEST" --out "$PLAN"
  [ "$status" -eq 0 ]
  [[ "$output" == *'write: .gemini/skills/example/SKILL.md'* ]]
  [[ "$output" == *'.agents/skills.bak'* ]]
  run approve
  [ "$status" -eq 0 ]
  [[ "$output" == *'Missing tools (advisory): fixturemissingtool'* ]]
  [ "$(cat "$HOME/.agents/skills/example/SKILL.md")" = 'reviewed skill' ]
  [ "$(cat "$HOME/.gemini/skills/example/SKILL.md")" = 'reviewed skill' ]
  [ "$(cat "$HOME/.agents/skills.bak/example/SKILL.md")" = 'old canonical' ]
  [ ! -e "$HOME/.agents/skills.bak/custom" ]
  [ "$(cat "$HOME/.agents/skills/custom/SKILL.md")" = unapproved ]
  [ "$(cat "$HOME/.gemini/skills/custom/SKILL.md")" = 'unapproved mirror' ]
  [ "$(cat "$HOME/.agents/skills/example/auth.json")" = runtime ]
  grep -q fixturemissingtool "$HOME/.local/state/dot/missing-tools.json"
  [ -z "$(git -C "$DOT_SOURCE" status --porcelain)" ]
  [ "$(find "$HOME/.local/state/dot/recovery" -type f ! -name plan.json | wc -l | tr -d ' ')" = 0 ]
}

@test "singleton backup conflicts require separate consent and stale backup slots reject apply" {
  mkdir -p "$HOME/.agents/skills/example" "$HOME/.agents/skills.bak/example"
  printf 'old canonical\n' > "$HOME/.agents/skills/example/SKILL.md"
  printf 'old backup\n' > "$HOME/.agents/skills.bak/example/SKILL.md"
  printf 'unrelated\n' > "$HOME/.agents/skills.bak/unrelated.txt"
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  run approve
  [ "$status" -ne 0 ]
  [[ "$output" == *'--replace-skills-backup'* ]]
  [ "$(cat "$HOME/.agents/skills/example/SKILL.md")" = 'old canonical' ]
  [ "$(cat "$HOME/.agents/skills.bak/example/SKILL.md")" = 'old backup' ]
  printf 'changed backup\n' > "$HOME/.agents/skills.bak/example/SKILL.md"
  run approve --replace-skills-backup
  [ "$status" -ne 0 ]
  [[ "$output" == *'stale/changed plan'* ]]
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve --replace-skills-backup
  [ "$(cat "$HOME/.agents/skills.bak/example/SKILL.md")" = 'old canonical' ]
  [ "$(cat "$HOME/.agents/skills.bak/unrelated.txt")" = unrelated ]
  "$DOT" intent update --yes
  [ "$(cat "$HOME/.agents/skills.bak/example/SKILL.md")" = 'old canonical' ]
  [ "$(find "$HOME/.agents" -maxdepth 1 -type d -name 'skills.bak*' | wc -l | tr -d ' ')" = 1 ]
}

@test "native skills routing updates exact approved files only and rejects arbitrary hooks" {
  mkdir -p "$HOME/.agents/skills/example" "$HOME/.config/opencode/skills/old" "$HOME/.codex/skills/old" "$HOME/.gemini/skills/custom"
  printf 'old canonical\n' > "$HOME/.agents/skills/example/SKILL.md"
  printf 'not inventoried\n' > "$HOME/.config/opencode/skills/old/SKILL.md"
  printf 'not inventoried\n' > "$HOME/.codex/skills/old/SKILL.md"
  printf 'unapproved\n' > "$HOME/.gemini/skills/custom/SKILL.md"
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve
  printf 'updated\n' > "$DOT_SOURCE/home/dot_agents/skills/example/SKILL.md"
  mkdir -p "$DOT_SOURCE/home/dot_agents/skills/future" "$HOME/.agents/skills/future"
  printf 'future upstream\n' > "$DOT_SOURCE/home/dot_agents/skills/future/SKILL.md"
  printf 'future local\n' > "$HOME/.agents/skills/future/SKILL.md"
  printf 'runtime\n' > "$HOME/.gemini/skills/custom/auth.json"
  git -C "$DOT_SOURCE" add .
  git -C "$DOT_SOURCE" commit -qm update
  chezmoi --source "$DOT_SOURCE/home" --destination "$HOME" --config "$DOT_CONFIG" apply --force
  [ "$(cat "$HOME/.agents/skills/example/SKILL.md")" = updated ]
  [ "$(cat "$HOME/.gemini/skills/example/SKILL.md")" = updated ]
  [ "$(cat "$HOME/.agents/skills.bak/example/SKILL.md")" = 'old canonical' ]
  [ "$(cat "$HOME/.agents/skills/future/SKILL.md")" = 'future local' ]
  [ ! -e "$HOME/.gemini/skills/future" ]
  [ "$(cat "$HOME/.gemini/skills/custom/auth.json")" = runtime ]
  [ "$(cat "$HOME/.gemini/skills/custom/SKILL.md")" = unapproved ]
  [ -f "$HOME/.config/opencode/skills/old/SKILL.md" ]
  [ -f "$HOME/.codex/skills/old/SKILL.md" ]
  printf '\necho bypass > "$HOME/bypassed"\n' >> "$DOT_SOURCE/home/run_after_sync-skills.sh.tmpl"
  run chezmoi --source "$DOT_SOURCE/home" --destination "$HOME" --config "$DOT_CONFIG" apply --force
  [ "$status" -ne 0 ]
  [[ "$output" == *'bounded adapter'* ]]
  [ ! -e "$HOME/bypassed" ]
}

@test "skill retirement removes only separately approved exact files preserving silo runtime and future skills" {
  mkdir -p "$HOME/.agents/skills/old" "$HOME/.codex/skills/old" "$HOME/.config/opencode/skills/old" "$HOME/.gemini/skills/old"
  for root in .agents .codex .config/opencode .gemini; do
    printf 'obsolete\n' > "$HOME/$root/skills/old/SKILL.md"
    printf 'runtime\n' > "$HOME/$root/skills/old/auth.json"
    printf 'not approved\n' > "$HOME/$root/skills/old/other.md"
  done
  printf '{"version":1,"mode":"adopt","profile":"skills","entries":["example"],"metadata":{"example":{"removals":[".agents/skills/old/SKILL.md",".codex/skills/old/SKILL.md",".config/opencode/skills/old/SKILL.md",".gemini/skills/old/SKILL.md"]}}}\n' > "$REQUEST"
  run "$DOT" intent plan "$REQUEST" --out "$PLAN"
  [ "$status" -eq 0 ]
  [[ "$output" == *'remove: .codex/skills/old/SKILL.md'* ]]
  [ -f "$HOME/.codex/skills/old/SKILL.md" ]
  approve
  [ "$(cat "$HOME/.agents/skills.bak/old/SKILL.md")" = obsolete ]
  for root in .agents .codex .config/opencode .gemini; do
    [ ! -e "$HOME/$root/skills/old/SKILL.md" ]
    [ "$(cat "$HOME/$root/skills/old/auth.json")" = runtime ]
    [ "$(cat "$HOME/$root/skills/old/other.md")" = 'not approved' ]
    printf 'recreated\n' > "$HOME/$root/skills/old/SKILL.md"
  done
  chezmoi --source "$DOT_SOURCE/home" --destination "$HOME" --config "$DOT_CONFIG" apply --force
  for root in .agents .codex .config/opencode .gemini; do
    [ ! -e "$HOME/$root/skills/old/SKILL.md" ]
    [ -f "$HOME/$root/skills/old/auth.json" ]
    [ -f "$HOME/$root/skills/old/other.md" ]
  done
  [ "$(cat "$HOME/.agents/skills.bak/old/SKILL.md")" = obsolete ]
  # The prior manifest is recoverable, but retired silo skill bytes are not archived.
  ! grep -r -q '^obsolete$\|^recreated$' "$HOME/.local/state/dot"
  printf '{"version":1,"mode":"adopt","profile":"skills","entries":["example"],"metadata":{"example":{"removals":[".codex/skills/old/auth.json"]}}}\n' > "$REQUEST"
  run "$DOT" intent plan "$REQUEST" --out "$PLAN"
  [ "$status" -ne 0 ]
  printf '{"version":1,"mode":"adopt","profile":"skills","entries":["example"],"metadata":{"example":{"removals":[".codex/skills/old"]}}}\n' > "$REQUEST"
  run "$DOT" intent plan "$REQUEST" --out "$PLAN"
  [ "$status" -ne 0 ]
  [ -f "$HOME/.codex/skills/old/auth.json" ]
}

@test "skills integration cannot target unrelated files or overwrite credentials through routing" {
  printf '{"version":1,"mode":"adopt","profile":"skills","entries":["example"]}\n' > "$REQUEST"
  mkdir -p "$HOME/.gemini/skills/example"
  printf 'password = "fixture-secret"\n' > "$HOME/.gemini/skills/example/SKILL.md"
  run "$DOT" intent plan "$REQUEST" --out "$PLAN"
  [ "$status" -ne 0 ]
  [[ "$output" == *'credential content detected'* ]]
  [[ "$output" != *'fixture-secret'* ]]
  [ ! -e "$HOME/.agents/skills/example/SKILL.md" ]
  rm "$HOME/.gemini/skills/example/SKILL.md"
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve
  printf 'password = "fixture-secret"\n' > "$HOME/.gemini/skills/example/SKILL.md"
  run "$DOT" run sync-skills
  [ "$status" -ne 0 ]
  [[ "$output" == *'credential content detected'* ]]
  printf 'unapproved credential-bearing skill\n' > "$HOME/.gemini/skills/example/SKILL.md"
  mkdir -p "$HOME/outside"
  mv "$HOME/.agents/skills/example" "$HOME/outside/example"
  ln -s "$HOME/outside/example" "$HOME/.agents/skills/example"
  run "$DOT" intent update --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'symlink/path escape'* ]]
}

@test "reviewed Adopt selections commit a reusable exact profile without machine state" {
  printf '{"version":1,"mode":"adopt","profile":"my-skills","entries":["example"]}\n' > "$REQUEST"
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve
  python3 - "$DOT_SOURCE/tools/dot/manifest.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
assert d['profiles']['my-skills'] == ['example']
PY
  [ "$(git -C "$DOT_SOURCE" diff HEAD~ --name-only)" = tools/dot/manifest.json ]
  [ -z "$(git -C "$DOT_SOURCE" status --porcelain)" ]
  [ ! -e "$DOT_SOURCE/.local/state/dot" ]
  grep -q 'my-skills' "$HOME/.local/state/dot/adoption.json"
  git -C "$DOT_SOURCE" log -1 --format=%s | grep -q 'Adopt reviewed configuration'
}

@test "secret-free Bearer environment references are eligible but literal credential suffixes are rejected" {
  mkdir -p "$HOME/.config/example"
  printf '{"Authorization": "Bearer {env:EXAMPLE_API_KEY}"}\n' > "$HOME/.config/example/config.json"
  printf '{"version":1,"mode":"track","profile":"example","selections":[{"path":".config/example/config.json","os":["darwin","linux"],"portability":"managed-shared","impact":"Safe"}]}\n' > "$REQUEST"
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve
  printf '{"Authorization": "Bearer {env:EXAMPLE_API_KEY}literal"}\n' > "$HOME/.config/example/config.json"
  run "$DOT" intent plan "$REQUEST" --out "$PLAN"
  [ "$status" -ne 0 ]
  [[ "$output" == *'credential content detected'* ]]
}

@test "empty Adopt scope does not run routing and cleanup inventories never select runtime or backups" {
  mkdir -p "$HOME/.codex/skills/old"
  printf 'retire\n' > "$HOME/.codex/skills/old/SKILL.md"
  printf 'auth\n' > "$HOME/.codex/skills/old/auth.json"
  run "$DOT" intent inventory .codex/skills
  [ "$status" -eq 0 ]
  [[ "$output" == *'.codex/skills/old/SKILL.md'* ]]
  [[ "$output" != *'auth.json'* ]]
  printf '{"version":1,"mode":"adopt","profile":"empty","entries":[]}\n' > "$REQUEST"
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve
  [ ! -e "$HOME/.agents/skills" ]
  [ ! -e "$HOME/.gemini/skills" ]
  [ -f "$HOME/.codex/skills/old/SKILL.md" ]
  run "$DOT" run sync-skills
  [ "$status" -ne 0 ]
  run "$DOT" intent inventory .agents/skills.bak
  [ "$status" -ne 0 ]
  [[ "$output" == *'local-unmanaged singleton'* ]]
}

@test "ordinary pull updates approved skills but upstream profile expansion cannot enroll canonical or Gemini files" {
  load update-helper
  independent_update_engine
  mkdir -p "$HOME/.agents/skills/example"
  printf 'original\n' > "$HOME/.agents/skills/example/SKILL.md"
  "$DOT" intent plan "$REQUEST" --out "$PLAN"
  approve
  branch="$(git -C "$DOT_SOURCE" branch --show-current)"
  remote="$BATS_TEST_TMPDIR/remote" upstream="$BATS_TEST_TMPDIR/upstream"
  git clone -q --bare "$DOT_SOURCE" "$remote"
  git -C "$DOT_SOURCE" remote add origin "$remote"
  git -C "$DOT_SOURCE" fetch -q origin
  git -C "$DOT_SOURCE" branch --set-upstream-to="origin/$branch" "$branch"
  git clone -q "$remote" "$upstream"
  git -C "$upstream" config user.name Test
  git -C "$upstream" config user.email test@example.org
  printf 'remote update\n' > "$upstream/home/dot_agents/skills/example/SKILL.md"
  mkdir -p "$upstream/home/dot_agents/skills/future" "$HOME/.agents/skills/future" "$HOME/.gemini/skills/future"
  printf 'future upstream\n' > "$upstream/home/dot_agents/skills/future/SKILL.md"
  printf 'future local\n' > "$HOME/.agents/skills/future/SKILL.md"
  printf 'future local output\n' > "$HOME/.gemini/skills/future/SKILL.md"
  python3 - "$upstream/tools/dot/manifest.json" <<'PY'
import json, sys
p = sys.argv[1]; d = json.load(open(p)); e = dict(d['entries']['example'])
e.update(destination='.agents/skills/future/SKILL.md', source='home/dot_agents/skills/future/SKILL.md')
d['entries']['future'] = e
d['profiles']['skills'].append('future')
json.dump(d, open(p, 'w'))
PY
  git -C "$upstream" add .
  git -C "$upstream" commit -qm 'content plus opt-in future entry'
  git -C "$upstream" push -q origin "$branch"
  run "$DOT" pull --yes
  [ "$status" -eq 0 ] || { printf '%s\n' "$output"; return 1; }
  [ "$(cat "$HOME/.agents/skills/example/SKILL.md")" = 'remote update' ]
  [ "$(cat "$HOME/.gemini/skills/example/SKILL.md")" = 'remote update' ]
  [ "$(cat "$HOME/.agents/skills/future/SKILL.md")" = 'future local' ]
  [ "$(cat "$HOME/.gemini/skills/future/SKILL.md")" = 'future local output' ]
  [ "$(cat "$HOME/.agents/skills.bak/example/SKILL.md")" = original ]
  python3 - "$HOME/.local/state/dot/adoption.json" <<'PY'
import json, sys
assert list(json.load(open(sys.argv[1]))['entries']) == ['example']
PY
}

@test "Adopt integration edits reject conflicts with unselected manifest destinations before committing or applying" {
  mkdir -p "$DOT_SOURCE/home/dot_gemini/skills/example" "$HOME/.gemini/skills/example"
  printf 'independently managed\n' > "$DOT_SOURCE/home/dot_gemini/skills/example/SKILL.md"
  printf 'local independent\n' > "$HOME/.gemini/skills/example/SKILL.md"
  python3 - "$DOT_SOURCE/tools/dot/manifest.json" <<'PY'
import json, sys
p = sys.argv[1]; d = json.load(open(p)); e = d['entries']['example']
e['integrations'] = []
d['entries']['mirror'] = dict(e, destination='.gemini/skills/example/SKILL.md', source='home/dot_gemini/skills/example/SKILL.md')
json.dump(d, open(p, 'w'))
PY
  git -C "$DOT_SOURCE" add .
  git -C "$DOT_SOURCE" commit -qm 'independent destinations'
  before="$(git -C "$DOT_SOURCE" rev-parse HEAD)"
  printf '{"version":1,"mode":"adopt","profile":"skills","entries":["example"],"metadata":{"example":{"integrations":["gemini-skills"]}}}\n' > "$REQUEST"
  run "$DOT" intent plan "$REQUEST" --out "$PLAN"
  [ "$status" -ne 0 ]
  [[ "$output" == *'generated skills output conflicts'* ]]
  [ "$(git -C "$DOT_SOURCE" rev-parse HEAD)" = "$before" ]
  [ -z "$(git -C "$DOT_SOURCE" status --porcelain)" ]
  [ "$(cat "$HOME/.gemini/skills/example/SKILL.md")" = 'local independent' ]
  [ ! -e "$HOME/.agents/skills/example/SKILL.md" ]
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
}
