#!/usr/bin/env bats
load bootstrap-helper

@test "macOS system Bash installs an uppercase-checksum browser before promoting checkouts" {
  printf '#!/bin/sh\necho browser fixture\n' > "$BATS_TEST_TMPDIR/browser"
  digest="$(shasum -a 256 "$BATS_TEST_TMPDIR/browser" | awk '{print toupper($1)}')"
  run /bin/bash "$BOOT" --yes --source-url "$FIXTURE_REMOTE" --engine-url "$FIXTURE_ENGINE_REMOTE" --browser-url "$BATS_TEST_TMPDIR/browser" --browser-sha256 "$digest"
  printf '%s\n' "$output" >&3
  [ "$status" -eq 0 ]
  [ -x "$HOME/.local/share/dot-cli/bin/dot-browser" ]
}

@test "fresh bad browser checksum leaves both checkouts and installation record absent" {
  printf 'browser fixture\n' > "$BATS_TEST_TMPDIR/browser"
  run /bin/bash "$BOOT" --yes --source-url "$FIXTURE_REMOTE" --engine-url "$FIXTURE_ENGINE_REMOTE" --browser-url "$BATS_TEST_TMPDIR/browser" --browser-sha256 "$(printf '%064d' 0)"
  printf '%s\n' "$output" >&3
  [ "$status" -ne 0 ]
  [ ! -e "$HOME/.local/share/dot-cli" ]
  [ ! -e "$HOME/.local/share/dot-config" ]
  [ ! -e "$HOME/.local/state/dot/install.json" ]
}

@test "repeat browser installation restores artifact when record promotion fails" {
  printf '#!/bin/sh\necho first\n' > "$BATS_TEST_TMPDIR/browser"
  digest="$(shasum -a 256 "$BATS_TEST_TMPDIR/browser" | awk '{print $1}')"
  install_dot --yes --browser-url "$BATS_TEST_TMPDIR/browser" --browser-sha256 "$digest"
  cp "$HOME/.local/state/dot/install.json" "$BATS_TEST_TMPDIR/record-before"
  printf '#!/bin/sh\necho second\n' > "$BATS_TEST_TMPDIR/browser"
  second="$(shasum -a 256 "$BATS_TEST_TMPDIR/browser" | awk '{print $1}')"
  chmod 500 "$HOME/.local/state/dot"
  run /bin/bash "$BOOT" --yes --source-url "$FIXTURE_REMOTE" --engine-url "$FIXTURE_ENGINE_REMOTE" --browser-url "$BATS_TEST_TMPDIR/browser" --browser-sha256 "$second"
  chmod 700 "$HOME/.local/state/dot"
  [ "$status" -ne 0 ]
  [ "$(shasum -a 256 "$HOME/.local/share/dot-cli/bin/dot-browser" | awk '{print $1}')" = "$digest" ]
  cmp "$BATS_TEST_TMPDIR/record-before" "$HOME/.local/state/dot/install.json"
}

@test "split bootstrap installs public engine and private source at independent defaults" {
  run install_dot --yes
  [ "$status" -eq 0 ]
  [ -d "$HOME/.local/share/dot-cli/.git" ]
  [ -d "$HOME/.local/share/dot-config/.git" ]
  [ -x "$HOME/.local/bin/dot" ]
  [ -f "$HOME/.local/state/dot/install.json" ]
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
  [ ! -e "$HOME/.local/state/dot/chezmoi-cache" ]
  [ ! -e "$HOME/.local/state/dot/chezmoi-state.boltdb" ]
}

@test "repeat split setup recognizes both installed checkouts without changing selection" {
  run install_dot --yes
  [ "$status" -eq 0 ]
  [ -x "$HOME/.local/bin/dot" ]
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
  run install_dot --yes
  [ "$status" -eq 0 ]
  [ ! -e "$HOME/.local/state/dot/adoption.json" ]
}

@test "contract mismatch fails before either install checkout, launcher, or metadata is created" {
  git -C "$BATS_TEST_TMPDIR/engine-seed" show HEAD:tools/dot/engine.json > "$BATS_TEST_TMPDIR/engine-original"
  printf '{}\n' > "$BATS_TEST_TMPDIR/engine-seed/tools/dot/engine.json"
  git -C "$BATS_TEST_TMPDIR/engine-seed" add tools/dot/engine.json
  git -C "$BATS_TEST_TMPDIR/engine-seed" -c user.name=Fixture -c user.email=fixture@example.org -c commit.gpgsign=false commit -qm incompatible
  git -C "$BATS_TEST_TMPDIR/engine-seed" push -q "$FIXTURE_ENGINE_REMOTE" main
  run install_dot --yes --engine-url "$FIXTURE_ENGINE_REMOTE"
  [ "$status" -ne 0 ]
  [[ "$output" == *'contract'* ]]
  [ ! -e "$HOME/.local/share/dot-cli" ]
  [ ! -e "$HOME/.local/share/dot-config" ]
  [ ! -e "$HOME/.local/state/dot/install.json" ]
  [ ! -e "$HOME/.local/bin/dot" ]
}

@test "legacy co-located checkout is refused rather than silently reclassified" {
  mkdir -p "$HOME/.local/share/dot/tools/dot" "$HOME/.local/share/dot/home"
  git init -q "$HOME/.local/share/dot"
  run install_dot --yes --engine-url "$FIXTURE_ENGINE_REMOTE"
  [ "$status" -ne 0 ]
  [[ "$output" == *'legacy installation'* ]]
  [ -d "$HOME/.local/share/dot/home" ]
  [ ! -e "$HOME/.local/share/dot-config" ]
  [ ! -e "$HOME/.local/bin/dot" ]
}

@test "browser artifact is installed only when the reviewed checksum matches" {
  printf '#!/bin/sh\necho browser fixture\n' > "$BATS_TEST_TMPDIR/browser"
  digest="$(shasum -a 256 "$BATS_TEST_TMPDIR/browser" | awk '{print $1}')"
  run install_dot --yes --browser-url "$BATS_TEST_TMPDIR/browser" --browser-sha256 "$digest"
  [ "$status" -eq 0 ]
  [ -x "$HOME/.local/share/dot-cli/bin/dot-browser" ]
  run install_dot --yes --browser-url "$BATS_TEST_TMPDIR/browser" --browser-sha256 "$(printf '%064d' 0)"
  [ "$status" -ne 0 ]
  [ -x "$HOME/.local/share/dot-cli/bin/dot-browser" ]
}

@test "public browser URL rejects credentials query and fragment" {
  for url in 'https://user:pass@example.org/browser' 'https://example.org/browser?token=secret' 'https://example.org/browser#fragment'; do
    run install_dot --browser-url "$url" --browser-sha256 "$(printf '%064d' 1)"
    [ "$status" -ne 0 ]
    [[ "$output" == *'must not contain credentials'* ]]
    [ ! -e "$HOME/.local/share/dot-cli" ]
  done
}

@test "browser bin symlink is refused without touching sentinel or creating launcher" {
  sentinel="$BATS_TEST_TMPDIR/browser-sentinel"
  printf 'preserve me\n' > "$sentinel"
  run install_dot --yes
  [ "$status" -eq 0 ]
  mkdir -p "$HOME/.local/share/dot-cli/bin"
  ln -s "$sentinel" "$HOME/.local/share/dot-cli/bin/dot-browser"
  printf '#!/bin/sh\necho browser\n' > "$BATS_TEST_TMPDIR/browser"
  digest="$(shasum -a 256 "$BATS_TEST_TMPDIR/browser" | awk '{print $1}')"
  run install_dot --yes --browser-url "$BATS_TEST_TMPDIR/browser" --browser-sha256 "$digest"
  [ "$status" -ne 0 ]
  [[ "$output" == *'browser destination symlink conflict'* ]]
  [ "$(cat "$sentinel")" = 'preserve me' ]
  [ -x "$HOME/.local/bin/dot" ]
}

@test "browser executable symlink is refused without replacing sentinel" {
  sentinel="$BATS_TEST_TMPDIR/browser-sentinel"
  printf 'preserve me\n' > "$sentinel"
  printf '#!/bin/sh\necho browser fixture\n' > "$BATS_TEST_TMPDIR/browser"
  digest="$(shasum -a 256 "$BATS_TEST_TMPDIR/browser" | awk '{print $1}')"
  run install_dot --yes --browser-url "$BATS_TEST_TMPDIR/browser" --browser-sha256 "$digest"
  [ "$status" -eq 0 ]
  mkdir -p "$HOME/.local/share/dot-cli/bin"
  rm "$HOME/.local/share/dot-cli/bin/dot-browser"
  ln -s "$sentinel" "$HOME/.local/share/dot-cli/bin/dot-browser"
  run install_dot --yes --browser-url "$BATS_TEST_TMPDIR/browser" --browser-sha256 "$digest"
  [ "$status" -ne 0 ]
  [[ "$output" == *'browser destination symlink conflict'* ]]
  [ "$(cat "$sentinel")" = 'preserve me' ]
}

@test "private-role engine markers are rejected before launcher or install record" {
  for marker in home dot-contract.json tools/dot/manifest.json tools/dot/groups.json; do
    rm -rf "$BATS_TEST_TMPDIR/engine-seed"
    mkdir -p "$BATS_TEST_TMPDIR/engine-seed/tools/dot"
    git init -q -b main "$BATS_TEST_TMPDIR/engine-seed"
    cp "$REPO/tools/dot/"*.py "$REPO/tools/dot/engine.json" "$REPO/tools/dot/dot" "$BATS_TEST_TMPDIR/engine-seed/tools/dot/"
    case "$marker" in
      home) mkdir -p "$BATS_TEST_TMPDIR/engine-seed/home"; printf 'private fixture\n' > "$BATS_TEST_TMPDIR/engine-seed/home/sentinel" ;;
      dot-contract.json) printf '{}\n' > "$BATS_TEST_TMPDIR/engine-seed/dot-contract.json" ;;
      *) mkdir -p "$BATS_TEST_TMPDIR/engine-seed/$(dirname "$marker")"; printf '{}\n' > "$BATS_TEST_TMPDIR/engine-seed/$marker" ;;
    esac
    git -C "$BATS_TEST_TMPDIR/engine-seed" add .
    git -C "$BATS_TEST_TMPDIR/engine-seed" -c user.name=Fixture -c user.email=fixture@example.org -c commit.gpgsign=false commit -qm marker
    remote="$BATS_TEST_TMPDIR/engine-marker-remote-${marker//\//-}"
    git clone -q --bare --no-local "$BATS_TEST_TMPDIR/engine-seed" "$remote"
    rm -rf "$HOME/.local/share/dot-cli" "$HOME/.local/share/dot-config" "$HOME/.local/bin/dot" "$HOME/.local/state/dot"
    run install_dot --yes --engine-url "$remote"
    [ "$status" -ne 0 ]
    [[ "$output" == *'private-role marker'* ]]
    [ ! -e "$HOME/.local/share/dot-cli" ]
    [ ! -e "$HOME/.local/share/dot-config" ]
    [ ! -e "$HOME/.local/bin/dot" ]
    [ ! -e "$HOME/.local/state/dot/install.json" ]
  done
}

@test "fresh setup reports init pending and installed launcher gives actionable no-tracker diagnostic" {
  run install_dot
  [ "$status" -eq 0 ]
  [[ "$output" == *'Run dot init'* ]]
  [ ! -e "$HOME/.config/chezmoi/chezmoi.toml" ]
  run "$HOME/.local/bin/dot" status
  [ "$status" -ne 0 ]
  [[ "$output" == *'tracker is not initialized'* ]]
}

@test "launcher quoting is exact across repeats with spaces quotes and shell metacharacters" {
  private="$HOME/private with 'quotes' \$dollar"
  engine="$HOME/engine with spaces & semicolon;"
  run install_dot --yes --directory "$private" --engine-directory "$engine"
  [ "$status" -eq 0 ]
  cp "$HOME/.local/bin/dot" "$BATS_TEST_TMPDIR/launcher-before"
  run install_dot --yes --directory "$private" --engine-directory "$engine"
  [ "$status" -eq 0 ]
  cmp "$BATS_TEST_TMPDIR/launcher-before" "$HOME/.local/bin/dot"
  run "$HOME/.local/bin/dot" contract check --engine "$engine/tools/dot" --source "$private"
  [ "$status" -eq 0 ]
}

@test "unrelated marker-bearing launcher is rejected exactly before browser replacement" {
  install_dot --yes
  mkdir -p "$HOME/.local/share/dot-cli/bin"
  printf 'preserve browser\n' > "$HOME/.local/share/dot-cli/bin/dot-browser"
  printf '#!/bin/sh\nDOT_ENGINE=unrelated\n' > "$HOME/.local/bin/dot"
  printf 'replacement\n' > "$BATS_TEST_TMPDIR/browser"
  digest="$(shasum -a 256 "$BATS_TEST_TMPDIR/browser" | awk '{print $1}')"
  run install_dot --yes --browser-url "$BATS_TEST_TMPDIR/browser" --browser-sha256 "$digest"
  [ "$status" -ne 0 ]
  [[ "$output" == *'unrelated launcher conflict'* ]]
  [ "$(cat "$HOME/.local/share/dot-cli/bin/dot-browser")" = 'preserve browser' ]
}

@test "contract mismatch prevents repeat browser replacement" {
  install_dot --yes
  mkdir -p "$HOME/.local/share/dot-cli/bin"
  printf 'preserve browser\n' > "$HOME/.local/share/dot-cli/bin/dot-browser"
  printf '{}\n' > "$HOME/.local/share/dot-cli/tools/dot/engine.json"
  printf 'replacement\n' > "$BATS_TEST_TMPDIR/browser"
  digest="$(shasum -a 256 "$BATS_TEST_TMPDIR/browser" | awk '{print $1}')"
  run install_dot --yes --browser-url "$BATS_TEST_TMPDIR/browser" --browser-sha256 "$digest"
  [ "$status" -ne 0 ]
  [ "$(cat "$HOME/.local/share/dot-cli/bin/dot-browser")" = 'preserve browser' ]
}

@test "pre-clone ancestor symlink and overlapping checkouts fail without mutation" {
  mkdir -p "$HOME/redirect" "$HOME/custom"
  printf 'preserve\n' > "$HOME/redirect/sentinel"
  ln -s "$HOME/redirect" "$HOME/custom/link"
  run install_dot --yes --engine-directory "$HOME/custom/link/deep/engine"
  [ "$status" -ne 0 ]
  [[ "$output" == *'symlink'* ]]
  [ ! -e "$HOME/redirect/deep" ]
  [ ! -e "$HOME/.local/share/dot-config" ]
  run install_dot --yes --engine-directory "$HOME/nested" --directory "$HOME/nested/private"
  [ "$status" -ne 0 ]
  [[ "$output" == *'non-nested'* ]]
  [ ! -e "$HOME/nested" ]
}

@test "pre-clone mismatched tracker remains byte-identical and invokes no hooks" {
  mkdir -p "$HOME/.config/chezmoi"
  printf 'sourceDir="/unrelated"\n[data]\ndotEngine="/unrelated"\n[hooks.read-source-state.pre]\ncommand="touch"\nargs=["%s/hook-ran"]\n' "$HOME" > "$HOME/.config/chezmoi/chezmoi.toml"
  cp "$HOME/.config/chezmoi/chezmoi.toml" "$BATS_TEST_TMPDIR/config-before"
  run install_dot --yes
  [ "$status" -ne 0 ]
  cmp "$BATS_TEST_TMPDIR/config-before" "$HOME/.config/chezmoi/chezmoi.toml"
  [ ! -e "$HOME/hook-ran" ]
  [ ! -e "$HOME/.local/share/dot-cli" ]
  [ ! -e "$HOME/.local/share/dot-config" ]
}

@test "private repository identity and visibility must match before cloning" {
  export FAKE_REPO_IDENTITY=unrelated/reused-name
  run install_dot --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'identity/visibility mismatch'* ]]
  [ ! -e "$HOME/.local/share/dot-cli" ]
  [ ! -e "$HOME/.local/share/dot-config" ]
  unset FAKE_REPO_IDENTITY
  export FAKE_REPO_PRIVATE=false
  run install_dot --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'identity/visibility mismatch'* ]]
  [ ! -e "$HOME/.local/bin/dot" ]
}

@test "candidate entrypoint failures stop before either checkout is installed" {
  printf 'preserve\n' > "$BATS_TEST_TMPDIR/sentinel"
  printf 'import pathlib; pathlib.Path(%s).write_text("executed")\n' "\"$BATS_TEST_TMPDIR/sentinel\"" > "$BATS_TEST_TMPDIR/outside-checker"
  for variant in missing-dot missing-checker nonexecutable-dot escaping-dot escaping-checker; do
    for name in dot contract.py; do
      rm -f "$BATS_TEST_TMPDIR/engine-seed/tools/dot/$name"
      cp "$REPO/tools/dot/$name" "$BATS_TEST_TMPDIR/engine-seed/tools/dot/$name"
    done
    case "$variant" in
      missing-dot) rm "$BATS_TEST_TMPDIR/engine-seed/tools/dot/dot" ;;
      missing-checker) rm "$BATS_TEST_TMPDIR/engine-seed/tools/dot/contract.py" ;;
      nonexecutable-dot) chmod 600 "$BATS_TEST_TMPDIR/engine-seed/tools/dot/dot" ;;
      escaping-dot) rm "$BATS_TEST_TMPDIR/engine-seed/tools/dot/dot"; ln -s "$REPO/tools/dot/dot" "$BATS_TEST_TMPDIR/engine-seed/tools/dot/dot" ;;
      escaping-checker) rm "$BATS_TEST_TMPDIR/engine-seed/tools/dot/contract.py"; ln -s "$BATS_TEST_TMPDIR/outside-checker" "$BATS_TEST_TMPDIR/engine-seed/tools/dot/contract.py" ;;
    esac
    git -C "$BATS_TEST_TMPDIR/engine-seed" add -A
    git -C "$BATS_TEST_TMPDIR/engine-seed" -c user.name=Fixture -c user.email=fixture@example.org -c commit.gpgsign=false commit -qm "$variant"
    git -C "$BATS_TEST_TMPDIR/engine-seed" push -q "$FIXTURE_ENGINE_REMOTE" main
    run install_dot --yes
    [ "$status" -ne 0 ]
    [[ "$output" == *'contract'* ]]
    [ ! -e "$HOME/.local/share/dot-cli" ]
    [ ! -e "$HOME/.local/share/dot-config" ]
    [ ! -e "$HOME/.local/bin/dot" ]
    [ "$(cat "$BATS_TEST_TMPDIR/sentinel")" = preserve ]
  done
}
