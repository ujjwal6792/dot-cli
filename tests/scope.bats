#!/usr/bin/env bats
setup() {
  REPO="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
  export HOME="$BATS_TEST_TMPDIR/home" DOT_SOURCE="$BATS_TEST_TMPDIR/source"
  export DOT_ENGINE="$REPO/tools/dot"
  export DOT_CONFIG="$HOME/.config/chezmoi/chezmoi.toml"
  mkdir -p "$HOME/.config/chezmoi" "$DOT_SOURCE/tools/dot" "$DOT_SOURCE/home"
  printf '[data]\nname="Test"\nemail="test@example.org"\nprofile="personal"\ndotEngine="%s"\n' "$DOT_ENGINE" > "$DOT_CONFIG"
  cp "$BATS_TEST_DIRNAME/fixtures/chezmoiremove.tmpl" "$DOT_SOURCE/home/.chezmoiremove"; cp "$BATS_TEST_DIRNAME/fixtures/run_after_sync-skills.sh.tmpl" "$DOT_SOURCE/home/run_after_sync-skills.sh.tmpl"
  mkdir -p "$DOT_SOURCE/scripts"
  cp "$REPO/scripts/sync-skills.sh" "$DOT_SOURCE/scripts/"
  cp "$REPO/tools/dot/"*.py "$DOT_SOURCE/tools/dot/"
  printf '{"protocol":1,"engine_api":{"min":1,"max":1},"manifest_version":1,"approval_version":2,"manifest":"tools/dot/manifest.json","home_source":"home","required_adapters":["intent","scope","gitconfig","skills","layout"]}\n' > "$DOT_SOURCE/dot-contract.json"
  printf '{"version":1,"entries":{},"profiles":{}}\n' > "$DOT_SOURCE/tools/dot/manifest.json"
  cp "$BATS_TEST_DIRNAME/fixtures/chezmoiignore.tmpl" "$DOT_SOURCE/home/.chezmoiignore"
  printf '{"eza":{"risk":"Safe","os":["darwin","linux"],"paths":[".config/eza"]},"shell":{"risk":"Unsafe","os":["darwin","linux"],"paths":[".zshrc"]},"skills":{"risk":"Safe","os":["darwin","linux"],"paths":[".agents/skills",".gemini/skills"]},"opencode":{"risk":"Safe","os":["darwin","linux"],"paths":[".config/opencode/settings"]}}\n' > "$DOT_SOURCE/tools/dot/groups.json"
  mkdir -p "$DOT_SOURCE/home/dot_config/eza"
  printf 'base\n' > "$DOT_SOURCE/home/dot_config/eza/theme.yml"
  printf 'shell\n' > "$DOT_SOURCE/home/dot_zshrc"
  DOT="$REPO/tools/dot/dot"
}
cm() {
  chezmoi --source "$DOT_SOURCE/home" --destination "$HOME" --config "$DOT_CONFIG" "$@"
}

@test "explicit empty scope renders without adopting files or running skills routing" {
  run "$DOT" scope init
  [ "$status" -eq 0 ]
  run cm apply --force
  [ "$status" -eq 0 ]
  [ ! -e "$HOME/.zshrc" ]
  [ ! -e "$HOME/.config/eza/theme.yml" ]
  run cm status
  [ "$status" -eq 0 ]
  [ -z "$output" ]
}

@test "selected tool survives upstream pull while shell removals and unknown hooks stay excluded" {
  # Pull now requires the explicit dedicated-home contract and exact approval.
  # Keep the same live-file/upstream assertions, rather than bypassing that gate.
  printf '{"protocol":1,"engine_api":{"min":1,"max":1},"manifest_version":1,"approval_version":2,"manifest":"tools/dot/manifest.json","home_source":"home","required_adapters":["intent","scope","gitconfig","skills","layout"]}\n' > "$DOT_SOURCE/dot-contract.json"
  export DOT_ENGINE="$REPO/tools/dot"
  printf '{"version":1,"entries":{"eza":{"destination":".config/eza/theme.yml","source":"home/dot_config/eza/theme.yml","os":["darwin","linux"],"portability":"managed-shared","impact":"Safe","removals":[],"hooks":[],"tools":[]}},"profiles":{"personal":["eza"]}}\n' > "$DOT_SOURCE/tools/dot/manifest.json"
  mkdir -p "$HOME/.local/state/dot"
  python3 - "$DOT_SOURCE/tools/dot/manifest.json" "$HOME/.local/state/dot/adoption.json" <<'PY'
import json,sys
manifest=json.load(open(sys.argv[1]))
json.dump(dict(version=2,profile='personal',entries=manifest['entries']),open(sys.argv[2],'w'))
PY
  cm() { chezmoi --source "$DOT_SOURCE/home" --destination "$HOME" --config "$DOT_CONFIG" "$@"; }
  git init -q --bare "$BATS_TEST_TMPDIR/remote"
  git init -q "$DOT_SOURCE"
  git -C "$DOT_SOURCE" config user.name Test
  git -C "$DOT_SOURCE" config user.email test@example.org
  printf '#!/bin/sh\ntouch "$HOME/hook-ran"\n' > "$DOT_SOURCE/home/run_after_unknown.sh"
  git -C "$DOT_SOURCE" add .
  git -C "$DOT_SOURCE" commit -qm initial
  git -C "$DOT_SOURCE" remote add origin "$BATS_TEST_TMPDIR/remote"
  git -C "$DOT_SOURCE" push -qu origin HEAD
  mkdir -p "$HOME/.config/opencode/skills"
  printf 'keep\n' > "$HOME/.config/opencode/skills/custom"
  printf 'local shell\n' > "$HOME/.zshrc"
  cm apply --force
  [ "$(cat "$HOME/.config/eza/theme.yml")" = base ]
  [ ! -e "$HOME/hook-ran" ]
  run "$DOT" status
  [ "$status" -eq 0 ]
  [[ "$output" == *'drift: 0'* ]]
  run "$DOT" diff
  [ "$status" -eq 0 ]
  [[ "$output" != *'local shell'* ]]
  git clone -q "$BATS_TEST_TMPDIR/remote" "$BATS_TEST_TMPDIR/other"
  git -C "$BATS_TEST_TMPDIR/other" config user.name Test
  git -C "$BATS_TEST_TMPDIR/other" config user.email test@example.org
  printf 'remote tool\n' > "$BATS_TEST_TMPDIR/other/home/dot_config/eza/theme.yml"
  printf 'remote shell\n' > "$BATS_TEST_TMPDIR/other/home/dot_zshrc"
  printf 'new group\n' > "$BATS_TEST_TMPDIR/other/home/dot_new-tool"
  git -C "$BATS_TEST_TMPDIR/other" add .
  git -C "$BATS_TEST_TMPDIR/other" commit -qm advance
  git -C "$BATS_TEST_TMPDIR/other" push -q
  run "$DOT" pull --yes
  [ "$status" -eq 0 ]
  [ "$(cat "$HOME/.config/eza/theme.yml")" = 'remote tool' ]
  [ "$(cat "$HOME/.zshrc")" = 'local shell' ]
  [ -f "$HOME/.config/opencode/skills/custom" ]
  [ ! -e "$HOME/.new-tool" ]
  [ ! -e "$HOME/hook-ran" ]
  printf 'local tool\n' > "$HOME/.config/eza/theme.yml"
  run "$DOT" push --message capture
  [ "$status" -eq 0 ]
  [ "$(git --git-dir="$BATS_TEST_TMPDIR/remote" show HEAD:home/dot_config/eza/theme.yml)" = 'local tool' ]
  [ "$(git --git-dir="$BATS_TEST_TMPDIR/remote" show HEAD:home/dot_zshrc)" = 'remote shell' ]
  [ ! -e "$DOT_SOURCE/home/dot_local/state/dot/adoption.json" ]
}

@test "scope expansion is preview-only by default and unsafe additions require acknowledgment" {
  "$DOT" scope init
  run "$DOT" scope expand shell
  [ "$status" -eq 0 ]
  [[ "$output" == *'Preview only'* ]]
  run "$DOT" scope expand shell --yes
  [ "$status" -ne 0 ]
  printf 'local\n' > "$HOME/.zshrc"
  run "$DOT" add --class shared "$HOME/.zshrc"
  [ "$status" -ne 0 ]
  run "$DOT" scope expand shell --yes --ack-unsafe
  [ "$status" -eq 0 ]
  run cm apply --force
  [ "$status" -eq 0 ]
  [ "$(cat "$HOME/.zshrc")" = shell ]
  run "$DOT" add --class shared "$HOME/.local/state/dot/adoption.json"
  [ "$status" -ne 0 ]
}

@test "doctor ignores unselected config secrets and direct skills routing cannot escape scope" {
  mkdir -p "$DOT_SOURCE/scripts" "$DOT_SOURCE/home/dot_config/opencode"
  printf '#!/bin/sh\nexit 0\n' > "$DOT_SOURCE/scripts/install-pins.sh"
  printf '#!/bin/sh\ntouch "$HOME/router-ran"\n' > "$DOT_SOURCE/scripts/sync-skills.sh"
  printf '{"key":"{env:UNSELECTED_API_KEY}"}\n' > "$DOT_SOURCE/home/dot_config/opencode/config.json"
  "$DOT" scope init
  "$DOT" scope expand eza --yes
  cm apply --force
  run "$DOT" doctor
  [ "$status" -eq 0 ]
  [[ "$output" != *'UNSELECTED_API_KEY'* ]]
  run "$DOT" run sync-skills
  [ "$status" -ne 0 ]
  [ ! -e "$HOME/router-ran" ]
}

@test "catalog additions and malformed profiles cannot silently expand scope" {
  "$DOT" scope init
  "$DOT" scope expand eza --yes
  python3 - "$DOT_SOURCE/tools/dot/groups.json" <<'PY'
import json, pathlib, sys
p = pathlib.Path(sys.argv[1])
d = json.loads(p.read_text())
d['eza']['paths'].append('.new-root')
p.write_text(json.dumps(d))
PY
  printf 'new\n' > "$DOT_SOURCE/home/dot_new-root"
  cm apply --force
  [ ! -e "$HOME/.new-root" ]
  run "$DOT" scope expand eza
  [ "$status" -eq 0 ]
  [[ "$output" == *'.new-root'* ]]
  cm apply --force
  [ ! -e "$HOME/.new-root" ]
  "$DOT" scope expand eza --yes
  cm apply --force
  [ "$(cat "$HOME/.new-root")" = new ]
  printf 'broken\n' > "$HOME/.local/state/dot/adoption.json"
  run cm apply --force
  [ "$status" -ne 0 ]
  [ ! -e "$HOME/.zshrc" ]
}

@test "scoped drift check reports only selected configuration divergence" {
  mkdir -p "$DOT_SOURCE/scripts"
  printf '#!/bin/sh\nexit 0\n' > "$DOT_SOURCE/scripts/install-pins.sh"
  git init -q --bare "$BATS_TEST_TMPDIR/remote"
  git init -q "$DOT_SOURCE"
  git -C "$DOT_SOURCE" config user.name Test
  git -C "$DOT_SOURCE" config user.email test@example.org
  git -C "$DOT_SOURCE" add .
  git -C "$DOT_SOURCE" commit -qm initial
  git -C "$DOT_SOURCE" remote add origin "$BATS_TEST_TMPDIR/remote"
  git -C "$DOT_SOURCE" push -qu origin HEAD
  "$DOT" scope init
  "$DOT" scope expand eza --yes
  cm apply --force
  printf 'local shell\n' > "$HOME/.zshrc"
  run "$DOT" status
  [ "$status" -eq 0 ]
  [[ "$output" == *'drift: 0 managed entries differ'* ]]
  printf 'changed\n' > "$HOME/.config/eza/theme.yml"
  run "$DOT" status
  [ "$status" -eq 0 ]
  [[ "$output" == *'drift: 1 managed entries differ'* ]]
}

@test "literal names nested hooks removal-only targets and dangling profiles fail closed" {
  "$DOT" scope init
  printf 'literal\n' > "$DOT_SOURCE/literal_dot_zshrc"
  printf 'keep\n' > "$HOME/.unselected"
  printf '\n.unselected\n' >> "$DOT_SOURCE/home/.chezmoiremove"
  cm apply --force
  [ ! -e "$HOME/dot_zshrc" ]
  [ "$(cat "$HOME/.unselected")" = keep ]
  "$DOT" scope expand eza --yes
  printf '#!/bin/sh\ntouch "$HOME/nested-hook"\n' > "$DOT_SOURCE/home/dot_config/eza/run_after_evil.sh"
  cm apply --force
  [ ! -e "$HOME/nested-hook" ]
  rm "$HOME/.local/state/dot/adoption.json"
  ln -s "$HOME/missing-profile" "$HOME/.local/state/dot/adoption.json"
  run cm apply --force
  [ "$status" -ne 0 ]
  [ ! -e "$HOME/.zshrc" ]
}

@test "selected skills authorize only their explicit router and removals" {
  mkdir -p "$DOT_SOURCE/home/dot_agents/skills/example" "$DOT_SOURCE/scripts"
  cp "$REPO/scripts/sync-skills.sh" "$DOT_SOURCE/scripts/"
  printf 'canonical\n' > "$DOT_SOURCE/home/dot_agents/skills/example/SKILL.md"
  python3 - "$DOT_SOURCE/tools/dot/manifest.json" <<'PY'
import json, sys
p = sys.argv[1]
entry = {'destination': '.agents/skills/example/SKILL.md',
         'source': 'home/dot_agents/skills/example/SKILL.md',
         'os': ['darwin', 'linux'], 'portability': 'managed-shared',
         'impact': 'Safe', 'removals': [], 'hooks': [], 'tools': [],
         'integrations': ['gemini-skills']}
json.dump({'version': 1, 'entries': {'example-skill': entry},
           'profiles': {'skills': ['example-skill']}}, open(p, 'w'))
PY
  git init -q "$DOT_SOURCE"
  git -C "$DOT_SOURCE" config user.name Test
  git -C "$DOT_SOURCE" config user.email test@example.org
  git -C "$DOT_SOURCE" add .
  git -C "$DOT_SOURCE" commit -qm 'synthetic skill catalog'
  printf '{"version":1,"mode":"adopt","profile":"skills","entries":["example-skill"]}\n' > "$BATS_TEST_TMPDIR/skills-request.json"
  "$DOT" intent plan "$BATS_TEST_TMPDIR/skills-request.json" --out "$BATS_TEST_TMPDIR/skills-plan.json"
  token="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["id"])' "$BATS_TEST_TMPDIR/skills-plan.json")"
  "$DOT" intent apply "$BATS_TEST_TMPDIR/skills-plan.json" --approve "$token"
  mkdir -p "$HOME/.config/opencode/skills" "$HOME/.codex/skills" "$HOME/.gemini/skills"
  printf 'obsolete\n' > "$HOME/.config/opencode/skills/old"
  printf 'old mirror\n' > "$HOME/.gemini/skills/old"
  cm apply --force
  [ "$(cat "$HOME/.config/opencode/skills/old")" = obsolete ]
  [ ! -e "$HOME/.codex/skills/old" ]
  [ "$(cat "$HOME/.gemini/skills/old")" = 'old mirror' ]
  [ "$(cat "$HOME/.gemini/skills/example/SKILL.md")" = canonical ]
  printf '{"version":1,"groups":{"skills":[]}}\n' > "$HOME/.local/state/dot/adoption.json"
  run "$DOT" run sync-skills
  [ "$status" -ne 0 ]
}

@test "empty profile supports chezmoi initialization and selected tools preserve credentials" {
  cp "$BATS_TEST_DIRNAME/fixtures/.chezmoi.toml.tmpl" "$DOT_SOURCE/"
  "$DOT" scope init
  run cm init --no-tty --promptString 'Full name (used by git)=Test' \
    --promptString 'Email address (used by git)=test@example.org' \
    --promptString 'Machine profile (personal or work)=personal'
  [ "$status" -eq 0 ]
  cm apply --force
  [ ! -e "$HOME/.zshrc" ]
  mkdir -p "$DOT_SOURCE/home/dot_config/opencode" "$HOME/.config/opencode/skills"
  printf 'preferences\n' > "$DOT_SOURCE/home/dot_config/opencode/settings"
  printf 'credential\n' > "$HOME/.config/opencode/service.json"
  printf 'custom\n' > "$HOME/.config/opencode/skills/local"
  "$DOT" scope expand opencode --yes
  cm apply --force
  [ "$(cat "$HOME/.config/opencode/service.json")" = credential ]
  [ "$(cat "$HOME/.config/opencode/skills/local")" = custom ]
  [ "$(cat "$HOME/.config/opencode/settings")" = preferences ]
}

@test "exact directories and native removal entries cannot delete unmanaged children" {
  "$DOT" scope init
  "$DOT" scope expand eza --yes
  mkdir -p "$HOME/.config/unselected"
  printf 'preserve\n' > "$HOME/.config/unselected/local"
  mv "$DOT_SOURCE/home/dot_config" "$DOT_SOURCE/home/exact_dot_config"
  run cm apply --force
  [ "$status" -ne 0 ]
  [ "$(cat "$HOME/.config/unselected/local")" = preserve ]
  mv "$DOT_SOURCE/home/exact_dot_config" "$DOT_SOURCE/home/dot_config"
  cm apply --force
  printf 'credential\n' > "$HOME/.config/eza/auth.json"
  rm -r "$DOT_SOURCE/home/dot_config/eza"
  touch "$DOT_SOURCE/home/dot_config/remove_eza"
  run cm apply --force
  [ "$status" -ne 0 ]
  [ "$(cat "$HOME/.config/eza/auth.json")" = credential ]
}

@test "expansion previews removal-only targets without deleting them" {
  "$DOT" scope init
  mkdir -p "$HOME/.config/eza"
  printf 'obsolete\n' > "$HOME/.config/eza/obsolete"
  printf '\n.config/eza/obsolete\n' >> "$DOT_SOURCE/home/.chezmoiremove"
  run "$DOT" scope expand eza
  [ "$status" -eq 0 ]
  [[ "$output" == *'remove: .config/eza/obsolete'* ]]
  [ "$(cat "$HOME/.config/eza/obsolete")" = obsolete ]
  run "$DOT" scope expand eza --yes
  [ "$status" -eq 0 ]
  [[ "$output" == *'remove: .config/eza/obsolete'* ]]
  [ "$(cat "$HOME/.config/eza/obsolete")" = obsolete ]
  cm apply --force
  [ ! -e "$HOME/.config/eza/obsolete" ]
}
