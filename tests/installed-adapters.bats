#!/usr/bin/env bats

setup() {
  REPO="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
  export HOME="$BATS_TEST_TMPDIR/home"
  PRIVATE="$BATS_TEST_TMPDIR/private"
  export DOT_SOURCE="$PRIVATE" DOT_ENGINE="$BATS_TEST_TMPDIR/public/tools/dot"
  mkdir -p "$HOME" "$PRIVATE/home" "$PRIVATE/tools/dot" "$DOT_ENGINE"
  cp "$REPO/tools/dot/"*.py "$REPO/tools/dot/engine.json" "$REPO/tools/dot/dot" "$DOT_ENGINE/"
  printf '{"protocol":1,"engine_api":{"min":1,"max":1},"manifest_version":1,"approval_version":2,"manifest":"tools/dot/manifest.json","home_source":"home","required_adapters":["intent","scope","gitconfig","skills","layout"]}\n' > "$PRIVATE/dot-contract.json"
  printf '{"version":1,"entries":{},"profiles":{}}\n' > "$PRIVATE/tools/dot/manifest.json"
  printf 'sourceDir = "%s"\n' "$PRIVATE/home" > "$BATS_TEST_TMPDIR/chezmoi.toml"
}

@test "native skills hook uses public adapter for empty scope without private scripts" {
  mkdir -p "$HOME/.local/state/dot"
  printf '{"version":2,"profile":"personal","entries":{}}\n' > "$HOME/.local/state/dot/adoption.json"
  chezmoi --source "$PRIVATE/home" --config "$BATS_TEST_TMPDIR/chezmoi.toml" execute-template < "$BATS_TEST_DIRNAME/fixtures/run_after_sync-skills.sh.tmpl" > "$BATS_TEST_TMPDIR/skills"
  run bash "$BATS_TEST_TMPDIR/skills"
  [ "$status" -eq 0 ]
  [ ! -e "$HOME/.agents" ]
  [ ! -e "$HOME/.gemini" ]
}

@test "bounded standalone skills adapter honors installed public engine and exact empty scope" {
  mkdir -p "$PRIVATE/scripts" "$HOME/.local/state/dot"
  cp "$REPO/scripts/sync-skills.sh" "$PRIVATE/scripts/"
  printf '{"version":2,"profile":"personal","entries":{}}\n' > "$HOME/.local/state/dot/adoption.json"
  run bash "$PRIVATE/scripts/sync-skills.sh"
  [ "$status" -ne 0 ]
  [[ "$output" == *'skills routing is outside adoption scope'* ]]
  [ ! -e "$HOME/.gemini" ]
}

@test "native ignore and removal adapters keep empty exact scope with independent engine" {
  cp "$BATS_TEST_DIRNAME/fixtures/chezmoiignore.tmpl" "$PRIVATE/home/.chezmoiignore"
  cp "$BATS_TEST_DIRNAME/fixtures/chezmoiremove.tmpl" "$PRIVATE/home/.chezmoiremove"
  printf 'repository\n' > "$PRIVATE/home/dot_fixture"
  printf 'target\n' > "$HOME/.fixture"
  mkdir -p "$HOME/.local/state/dot"
  printf '{"version":2,"profile":"personal","entries":{}}\n' > "$HOME/.local/state/dot/adoption.json"
  run chezmoi --source "$PRIVATE/home" --destination "$HOME" --config "$BATS_TEST_TMPDIR/chezmoi.toml" apply
  [ "$status" -eq 0 ]
  [ "$(cat "$HOME/.fixture")" = target ]
}

@test "persisted engine resolution rejects incompatible native application before target writes" {
  cp "$BATS_TEST_DIRNAME/fixtures/chezmoiignore.tmpl" "$PRIVATE/home/.chezmoiignore"
  cp "$BATS_TEST_DIRNAME/fixtures/chezmoiremove.tmpl" "$PRIVATE/home/.chezmoiremove"
  printf 'repository\n' > "$PRIVATE/home/dot_fixture"
  printf 'target\n' > "$HOME/.fixture"
  printf '\n[data]\ndotEngine = "%s"\n' "$DOT_ENGINE" >> "$BATS_TEST_TMPDIR/chezmoi.toml"
  unset DOT_ENGINE
  python3 - "$PRIVATE/dot-contract.json" <<'PY'
import json, pathlib, sys
p = pathlib.Path(sys.argv[1]); d = json.loads(p.read_text())
d['engine_api'] = {'min': 2, 'max': 2}; p.write_text(json.dumps(d))
PY
  run chezmoi --source "$PRIVATE/home" --destination "$HOME" --config "$BATS_TEST_TMPDIR/chezmoi.toml" apply
  [ "$status" -ne 0 ]
  [[ "$output" == *'incompatible or missing engine/config contract'* ]]
  [ "$(cat "$HOME/.fixture")" = target ]
}

@test "empty persisted engine fails instead of executing private co-located adapters" {
  cp "$REPO/tools/dot/"*.py "$PRIVATE/tools/dot/"
  cp "$BATS_TEST_DIRNAME/fixtures/chezmoiignore.tmpl" "$PRIVATE/home/.chezmoiignore"
  printf 'repository\n' > "$PRIVATE/home/dot_fixture"
  printf 'target\n' > "$HOME/.fixture"
  printf '\n[data]\ndotEngine = ""\n' >> "$BATS_TEST_TMPDIR/chezmoi.toml"
  unset DOT_ENGINE
  run chezmoi --source "$PRIVATE/home" --destination "$HOME" --config "$BATS_TEST_TMPDIR/chezmoi.toml" apply
  [ "$status" -ne 0 ]
  [[ "$output" == *'explicit public dotEngine is required'* ]]
  [ "$(cat "$HOME/.fixture")" = target ]
}

@test "split installation without engine resolution refuses native private adapter fallback" {
  cp "$REPO/tools/dot/"*.py "$PRIVATE/tools/dot/"
  cp "$BATS_TEST_DIRNAME/fixtures/chezmoiignore.tmpl" "$PRIVATE/home/.chezmoiignore"
  mkdir -p "$HOME/.local/state/dot"
  printf '{"version":2}\n' > "$HOME/.local/state/dot/install.json"
  unset DOT_ENGINE
  run chezmoi --source "$PRIVATE/home" --destination "$HOME" --config "$BATS_TEST_TMPDIR/chezmoi.toml" apply
  [ "$status" -ne 0 ]
  [[ "$output" == *'split installation requires explicit public dotEngine'* ]]
}

@test "managed launcher uses explicit independent engine and private source" {
  chezmoi --source "$PRIVATE/home" --config "$BATS_TEST_TMPDIR/chezmoi.toml" execute-template < "$BATS_TEST_DIRNAME/fixtures/executable_dot.tmpl" > "$BATS_TEST_TMPDIR/dot"
  run bash "$BATS_TEST_TMPDIR/dot" contract check --engine "$DOT_ENGINE" --source "$PRIVATE"
  [ "$status" -eq 0 ]
  [[ "$output" == *'"engine_api": 1'* ]]
}

@test "bounded native Git adapter uses public engine and refuses unapproved Git settings" {
  mkdir -p "$HOME/.local/state/dot"
  printf '{"version":2,"profile":"personal","entries":{}}\n' > "$HOME/.local/state/dot/adoption.json"
  chezmoi --source "$PRIVATE/home" --config "$BATS_TEST_TMPDIR/chezmoi.toml" execute-template < "$BATS_TEST_DIRNAME/fixtures/modify_dot_gitconfig.tmpl" > "$BATS_TEST_TMPDIR/modify"
  printf '[user]\n\tname = Fixture Owner\n\temail = fixture@example.invalid\n' > "$BATS_TEST_TMPDIR/gitconfig"
  run bash -c 'bash "$1" < "$2"' _ "$BATS_TEST_TMPDIR/modify" "$BATS_TEST_TMPDIR/gitconfig"
  [ "$status" -ne 0 ]
  [[ "$output" == *'requires unchanged exact approved metadata'* ]]
  [ "$(git config --file "$BATS_TEST_TMPDIR/gitconfig" user.name)" = 'Fixture Owner' ]
}
