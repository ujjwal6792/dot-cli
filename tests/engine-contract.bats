#!/usr/bin/env bats
# Public seam: `dot contract check --engine ABS --source ABS` against a
# synthetic private checkout. No private repository dependency.
setup() {
  export HOME="$BATS_TEST_TMPDIR/home"
  ENGINE="$BATS_TEST_TMPDIR/public/tools/dot"
  PRIVATE="$BATS_TEST_TMPDIR/private"
  mkdir -p "$HOME" "$ENGINE" "$PRIVATE/home" "$PRIVATE/tools/dot"
  for file in dot contract.py engine.json intent.py scope.py gitconfig.py skills.py layout.py tracking.py file_metadata.py; do
    cp "$BATS_TEST_DIRNAME/../tools/dot/$file" "$ENGINE/"
  done
  printf '{"protocol":1,"engine_api":{"min":1,"max":1},"manifest_version":1,"approval_version":2,"manifest":"tools/dot/manifest.json","home_source":"home","required_adapters":["intent","scope","gitconfig","skills","layout"]}\n' > "$PRIVATE/dot-contract.json"
  printf '{"version":1,"entries":{},"profiles":{}}\n' > "$PRIVATE/tools/dot/manifest.json"
  printf 'unchanged\n' > "$HOME/sentinel"
}

@test "unsupported versions and malformed descriptors stop before mutation" {
  for change in 'd["protocol"]=2' 'd["protocol"]=True' 'd["engine_api"]={"min":2,"max":3}' 'd["manifest_version"]=2' 'd["approval_version"]=1' 'd["unknown"]=1'; do
    cp "$PRIVATE/dot-contract.json" "$BATS_TEST_TMPDIR/original"
    python3 -c 'import json,sys; p=sys.argv[1]; d=json.load(open(p)); exec(sys.argv[2]); open(p,"w").write(json.dumps(d))' "$PRIVATE/dot-contract.json" "$change"
    run "$ENGINE/dot" contract check --engine "$ENGINE" --source "$PRIVATE"
    [ "$status" -ne 0 ]
    [[ "$output" == *'no configuration was applied'* ]]
    cp "$BATS_TEST_TMPDIR/original" "$PRIVATE/dot-contract.json"
  done
  [ "$(cat "$HOME/sentinel")" = unchanged ]
  [ ! -e "$ENGINE/manifest.json" ]
}

@test "independent installed CLI negotiates private contract read-only" {
  run "$ENGINE/dot" contract check --engine "$ENGINE" --source "$PRIVATE"
  [ "$status" -eq 0 ]
  [[ "$output" == *'"engine_api": 1'* ]]
  [[ "$output" == *"$PRIVATE/home"* ]]
  [ "$(cat "$HOME/sentinel")" = unchanged ]
  [ "$(cat "$HOME/sentinel")" = unchanged ]
}

@test "missing engine adapters or manifest never falls back to private tooling" {
  for path in "$ENGINE/engine.json" "$ENGINE/gitconfig.py" "$PRIVATE/tools/dot/manifest.json" "$PRIVATE/dot-contract.json"; do
    mv "$path" "$path.saved"
    run "$ENGINE/dot" contract check --engine "$ENGINE" --source "$PRIVATE"
    [ "$status" -ne 0 ]
    [[ "$output" == *'no configuration was applied'* ]]
    mv "$path.saved" "$path"
  done
  [ "$(cat "$HOME/sentinel")" = unchanged ]
}

@test "explicit bounded paths reject escapes and legacy source boundaries" {
  run "$ENGINE/dot" contract check --engine tools/dot --source "$PRIVATE"
  [ "$status" -ne 0 ]
  for change in 'd["manifest"]="../outside.json"' 'd["home_source"]="."' 'd["manifest"]="/outside.json"'; do
    cp "$PRIVATE/dot-contract.json" "$BATS_TEST_TMPDIR/original"
    python3 -c 'import json,sys; p=sys.argv[1]; d=json.load(open(p)); exec(sys.argv[2]); open(p,"w").write(json.dumps(d))' "$PRIVATE/dot-contract.json" "$change"
    run "$ENGINE/dot" contract check --engine "$ENGINE" --source "$PRIVATE"
    [ "$status" -ne 0 ]
    cp "$BATS_TEST_TMPDIR/original" "$PRIVATE/dot-contract.json"
  done
  mv "$PRIVATE/home" "$BATS_TEST_TMPDIR/outside-home"
  ln -s "$BATS_TEST_TMPDIR/outside-home" "$PRIVATE/home"
  run "$ENGINE/dot" contract check --engine "$ENGINE" --source "$PRIVATE"
  [ "$status" -ne 0 ]
  [ "$(cat "$HOME/sentinel")" = unchanged ]
}

@test "ambiguous JSON fails closed instead of selecting a duplicate version" {
  printf '{"protocol":2,"protocol":1,"engine_api":{"min":1,"max":1},"manifest_version":1,"approval_version":2,"manifest":"tools/dot/manifest.json","home_source":"home","required_adapters":["intent","scope","gitconfig","skills","layout"]}\n' > "$PRIVATE/dot-contract.json"
  run "$ENGINE/dot" contract check --engine "$ENGINE" --source "$PRIVATE"
  [ "$status" -ne 0 ]
  [[ "$output" == *'no configuration was applied'* ]]
}

@test "public versions adapter paths and private manifest schema are checked" {
  for change in 'd["engine_api"]=2' 'd["protocol"]=2' 'd["approval_version"]=1' 'd["adapters"]["intent"]="../intent.py"' 'd["adapters"]={}' 'd["adapters"]["intent"]=None'; do
    cp "$ENGINE/engine.json" "$BATS_TEST_TMPDIR/original"
    python3 -c 'import json,sys; p=sys.argv[1]; d=json.load(open(p)); exec(sys.argv[2]); open(p,"w").write(json.dumps(d))' "$ENGINE/engine.json" "$change"
    run "$ENGINE/dot" contract check --engine "$ENGINE" --source "$PRIVATE"
    [ "$status" -ne 0 ]
    [[ "$output" != *Traceback* ]]
    cp "$BATS_TEST_TMPDIR/original" "$ENGINE/engine.json"
  done
  printf '{"version":2,"entries":{},"profiles":{}}' > "$PRIVATE/tools/dot/manifest.json"
  run "$ENGINE/dot" contract check --engine "$ENGINE" --source "$PRIVATE"
  [ "$status" -ne 0 ]
  printf 'not JSON' > "$PRIVATE/tools/dot/manifest.json"
  run "$ENGINE/dot" contract check --engine "$ENGINE" --source "$PRIVATE"
  [ "$status" -ne 0 ]
  [ "$(cat "$HOME/sentinel")" = unchanged ]
}

@test "environment defaults and co-located engine are not accepted" {
  export DOT_SOURCE="$PRIVATE" DOT_ENGINE="$ENGINE"
  run "$ENGINE/dot" contract check
  [ "$status" -ne 0 ]
  mkdir -p "$PRIVATE/tools/engine"
  cp "$ENGINE/"* "$PRIVATE/tools/engine/"
  run "$ENGINE/dot" contract check --engine "$PRIVATE/tools/engine" --source "$PRIVATE"
  [ "$status" -ne 0 ]
  mv "$ENGINE/intent.py" "$BATS_TEST_TMPDIR/outside-adapter"
  ln -s "$BATS_TEST_TMPDIR/outside-adapter" "$ENGINE/intent.py"
  run "$ENGINE/dot" contract check --engine "$ENGINE" --source "$PRIVATE"
  [ "$status" -ne 0 ]
  [ "$(cat "$HOME/sentinel")" = unchanged ]
}

@test "synthetic descriptors negotiate when exported separately from private metadata" {
  # Public synthetic equivalent of the private round-trip: a fixture contract
  # plus a fixture manifest with one Safe entry must negotiate cleanly.
  cat > "$PRIVATE/dot-contract.json" <<'JSON'
{"protocol":1,"engine_api":{"min":1,"max":1},"manifest_version":1,"approval_version":2,"manifest":"tools/dot/manifest.json","home_source":"home","required_adapters":["intent","scope","gitconfig","skills","layout"]}
JSON
  cat > "$PRIVATE/tools/dot/manifest.json" <<'JSON'
{"version":1,"entries":{"fixture":{"destination":".config/fixture.conf","source":"home/dot_config/fixture.conf","os":["darwin","linux"],"portability":"managed-shared","impact":"Safe","removals":[],"hooks":[],"tools":[]}},"profiles":{"default":["fixture"]}}
JSON
  mkdir -p "$PRIVATE/home/dot_config"
  printf 'fixture\n' > "$PRIVATE/home/dot_config/fixture.conf"
  run "$ENGINE/dot" contract check --engine "$ENGINE" --source "$PRIVATE"
  [ "$status" -eq 0 ]
  [[ "$output" == *'"approval_version": 2'* ]]
}

@test "healthy CLI rejects missing non-executable and escaping candidate engine entrypoints" {
  CANDIDATE="$BATS_TEST_TMPDIR/candidate/tools/dot"
  mkdir -p "$CANDIDATE"
  cp "$ENGINE/"* "$CANDIDATE/"
  for missing in dot contract.py; do
    mv "$CANDIDATE/$missing" "$BATS_TEST_TMPDIR/saved-$missing"
    run env DOT_ENGINE="$ENGINE" DOT_SOURCE="$PRIVATE" "$ENGINE/dot" contract check --engine "$CANDIDATE" --source "$PRIVATE"
    [ "$status" -ne 0 ]
    [[ "$output" == *'no configuration was applied'* ]]
    mv "$BATS_TEST_TMPDIR/saved-$missing" "$CANDIDATE/$missing"
  done
  chmod 600 "$CANDIDATE/dot"
  run env DOT_ENGINE="$ENGINE" DOT_SOURCE="$PRIVATE" "$ENGINE/dot" contract check --engine "$CANDIDATE" --source "$PRIVATE"
  [ "$status" -ne 0 ]
  chmod 755 "$CANDIDATE/dot"
  for escaped in dot contract.py; do
    mv "$CANDIDATE/$escaped" "$BATS_TEST_TMPDIR/outside-$escaped"
    ln -s "$BATS_TEST_TMPDIR/outside-$escaped" "$CANDIDATE/$escaped"
    run env DOT_ENGINE="$ENGINE" DOT_SOURCE="$PRIVATE" "$ENGINE/dot" contract check --engine "$CANDIDATE" --source "$PRIVATE"
    [ "$status" -ne 0 ]
    rm "$CANDIDATE/$escaped"
    mv "$BATS_TEST_TMPDIR/outside-$escaped" "$CANDIDATE/$escaped"
  done
  run env DOT_ENGINE="$ENGINE" DOT_SOURCE="$PRIVATE" "$ENGINE/dot" contract check --engine "$CANDIDATE" --source "$PRIVATE"
  [ "$status" -eq 0 ]
  [ "$(cat "$HOME/sentinel")" = unchanged ]
  [ "$(cat "$HOME/sentinel")" = unchanged ]
}
