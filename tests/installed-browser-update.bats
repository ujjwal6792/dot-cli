#!/usr/bin/env bats
load bootstrap-helper

setup() {
  bootstrap_setup
  browser="${DOT_BROWSER_TEST_BINARY:-$BATS_TEST_TMPDIR/browser}"
  if [ -z "${DOT_BROWSER_TEST_BINARY:-}" ]; then
    HOME="$BATS_TEST_TMPDIR/build-home" GOFLAGS=-modcacherw go build -C "$REPO/tools/dot/browser" -mod=readonly -trimpath -buildvcs=false -o "$browser" ./cmd/dot-browser
  fi
  baseline="${DOT_BROWSER_BASELINE_TEST_BINARY:-$browser}"
  digest="$(shasum -a 256 "$baseline" | awk '{print $1}')"
  install_dot --yes --browser-url "$baseline" --browser-sha256 "$digest"
  public="$HOME/.local/share/dot-cli"
  before="$(git -C "$public" rev-parse HEAD)"
  printf 'reviewed public tooling\n' > "$BATS_TEST_TMPDIR/engine-seed/CHANGELOG"
  git -C "$BATS_TEST_TMPDIR/engine-seed" add .
  git -C "$BATS_TEST_TMPDIR/engine-seed" -c user.name=Fixture -c user.email=fixture@example.org -c commit.gpgsign=false commit -qm tooling
  git -C "$BATS_TEST_TMPDIR/engine-seed" push -q "$FIXTURE_ENGINE_REMOTE" main
}

@test "actual installed prebuilt browser permits review and tooling-only self-update" {
  cp "$HOME/.local/state/dot/install.json" "$BATS_TEST_TMPDIR/record-before"
  private="$(git -C "$HOME/.local/share/dot-config" rev-parse HEAD)"
  run "$HOME/.local/bin/dot" self-update
  printf '%s\n' "$output" >&3
  [ "$status" -eq 0 ]
  [ "$(git -C "$public" rev-parse HEAD)" = "$before" ]
  run "$HOME/.local/bin/dot" self-update --yes
  [ "$status" -eq 0 ]
  [ -f "$public/CHANGELOG" ]
  [ "$(shasum -a 256 "$public/bin/dot-browser" | awk '{print $1}')" = "$digest" ]
  cmp "$BATS_TEST_TMPDIR/record-before" "$HOME/.local/state/dot/install.json"
  [ "$(git -C "$HOME/.local/share/dot-config" rev-parse HEAD)" = "$private" ]
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
}

@test "installed self-update refuses forged browser and unrelated ignored bin work" {
  cp "$public/bin/dot-browser" "$BATS_TEST_TMPDIR/original"
  printf 'forged\n' >> "$public/bin/dot-browser"
  run "$HOME/.local/bin/dot" self-update --yes
  [ "$status" -ne 0 ]
  [ "$(git -C "$public" rev-parse HEAD)" = "$before" ]
  cp "$BATS_TEST_TMPDIR/original" "$public/bin/dot-browser"
  mkdir -p "$public/.git/info"
  printf 'bin/private-notes\n' >> "$public/.git/info/exclude"
  printf 'preserve private work\n' > "$public/bin/private-notes"
  run "$HOME/.local/bin/dot" self-update --yes
  [ "$status" -ne 0 ]
  [ "$(git -C "$public" rev-parse HEAD)" = "$before" ]
  [ "$(cat "$public/bin/private-notes")" = 'preserve private work' ]
}

@test "reviewed browser update verifies checksum before fast-forward and records the artifact coherently" {
  reviewed_digest="$(shasum -a 256 "$browser" | awk '{print $1}')"
  cp "$HOME/.local/state/dot/install.json" "$BATS_TEST_TMPDIR/record-before"
  run "$HOME/.local/bin/dot" self-update --yes --browser-url "$browser" --browser-sha256 "$(printf '%064d' 0)"
  [ "$status" -ne 0 ]
  [ "$(git -C "$public" rev-parse HEAD)" = "$before" ]
  cmp "$BATS_TEST_TMPDIR/record-before" "$HOME/.local/state/dot/install.json"
  run "$HOME/.local/bin/dot" self-update --yes --browser-url "$browser" --browser-sha256 "$reviewed_digest"
  [ "$status" -eq 0 ]
  [ -f "$public/CHANGELOG" ]
  python3 - "$HOME/.local/state/dot/install.json" "$reviewed_digest" <<'PY'
import json,sys
assert json.load(open(sys.argv[1]))['browser']['sha256'] == sys.argv[2]
PY
  [ "$(shasum -a 256 "$public/bin/dot-browser" | awk '{print $1}')" = "$reviewed_digest" ]
  if [ -n "${DOT_BROWSER_BASELINE_TEST_BINARY:-}" ]; then
    [ "$reviewed_digest" != "$digest" ]
  fi
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
}

@test "browser record promotion failure rolls back tooling HEAD and preserves installed bytes" {
  cp "$HOME/.local/state/dot/install.json" "$BATS_TEST_TMPDIR/record-before"
  chmod 500 "$HOME/.local/state/dot"
  reviewed_digest="$(shasum -a 256 "$browser" | awk '{print $1}')"
  run "$HOME/.local/bin/dot" self-update --yes --browser-url "$browser" --browser-sha256 "$reviewed_digest"
  chmod 700 "$HOME/.local/state/dot"
  [ "$status" -ne 0 ]
  [[ "$output" == *'promotion failed'* ]]
  [ "$(git -C "$public" rev-parse HEAD)" = "$before" ]
  [ ! -e "$public/CHANGELOG" ]
  cmp "$BATS_TEST_TMPDIR/record-before" "$HOME/.local/state/dot/install.json"
  [ "$(shasum -a 256 "$public/bin/dot-browser" | awk '{print $1}')" = "$digest" ]
}

@test "browser source update requires a reviewed artifact and missing record cannot exempt a binary" {
  mkdir -p "$BATS_TEST_TMPDIR/engine-seed/tools/dot/browser"
  printf 'reviewed browser source\n' > "$BATS_TEST_TMPDIR/engine-seed/tools/dot/browser/CHANGELOG"
  git -C "$BATS_TEST_TMPDIR/engine-seed" add .
  git -C "$BATS_TEST_TMPDIR/engine-seed" -c user.name=Fixture -c user.email=fixture@example.org -c commit.gpgsign=false commit -qm browser
  git -C "$BATS_TEST_TMPDIR/engine-seed" push -q "$FIXTURE_ENGINE_REMOTE" main
  run "$HOME/.local/bin/dot" self-update --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'browser source changed'* ]]
  [ "$(git -C "$public" rev-parse HEAD)" = "$before" ]
  mv "$HOME/.local/state/dot/install.json" "$BATS_TEST_TMPDIR/record"
  run "$HOME/.local/bin/dot" self-update --yes
  [ "$status" -ne 0 ]
  [ "$(git -C "$public" rev-parse HEAD)" = "$before" ]
  [ "$(shasum -a 256 "$public/bin/dot-browser" | awk '{print $1}')" = "$digest" ]
}
