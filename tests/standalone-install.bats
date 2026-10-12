#!/usr/bin/env bats

setup() {
  REPO="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
  TMP="$(cd -P "$BATS_TEST_TMPDIR" && pwd)"
  export HOME="$TMP/home"
  mkdir -p "$HOME" "$BATS_TEST_TMPDIR/releases"
  export RELEASE_DIR="$TMP/releases"
  export MINIMAL_BIN="$TMP/minimal-bin"
  mkdir -p "$MINIMAL_BIN"
  ln -s /bin/bash "$MINIMAL_BIN/bash"
  ln -s /usr/bin/dirname "$MINIMAL_BIN/dirname"
  ln -s /usr/bin/sed "$MINIMAL_BIN/sed"
  ln -s /bin/cat "$MINIMAL_BIN/cat"
  ln -s /usr/bin/readlink "$MINIMAL_BIN/readlink"
  cp "$REPO/tools/dot/dot" "$TMP/dot"
}

@test "installed CLI help and version do not require Python or private config" {
  export DOT_ENGINE="$REPO/tools/dot"
  run env PATH="$MINIMAL_BIN" "$TMP/dot" --help
  [ "$status" -eq 0 ]
  [[ "$output" == *"usage: dot"* ]]
  run env PATH="$MINIMAL_BIN" "$TMP/dot" --version
  [ "$status" -eq 0 ]
}

@test "installer rejects a checksum mismatch without changing the install root" {
  run bash "$REPO/install.sh" --version 1.2.3 --platform linux-amd64 \
    --url "file://$RELEASE_DIR/bad.tar.gz" --sha256 "$(printf '%064d' 0)" \
    --root "$TMP/install root"
  [ "$status" -ne 0 ]
  [ ! -e "$TMP/install root" ]
}

@test "unsupported platform is rejected before any path is created" {
  run bash "$REPO/install.sh" --version 1.2.3 --platform darwin-amd64 \
    --url "file://$RELEASE_DIR/missing" --sha256 "$(printf '%064d' 0)" \
    --root "$TMP/unsupported root"
  [ "$status" -ne 0 ]
  [[ "$output" == *'unsupported platform'* ]]
  [ ! -e "$TMP/unsupported root" ]
}

@test "system and package-manager prefixes are never installer targets" {
  run bash "$REPO/install.sh" --version 1.2.3 --platform linux-amd64 \
    --url "file://$RELEASE_DIR/missing" --sha256 "$(printf '%064d' 0)" --root /usr/local/share/dot-cli
  [ "$status" -ne 0 ]
  [[ "$output" == *'system/package-manager prefix is protected'* ]]
}

@test "installer refuses symlinked user launcher ancestors" {
  mkdir -p "$TMP/outside-local"
  ln -s "$TMP/outside-local" "$HOME/.local"
  run bash "$REPO/install.sh" --version 1.2.3 --platform linux-amd64 \
    --url "file://$RELEASE_DIR/missing" --sha256 "$(printf '%064d' 0)" \
    --root "$TMP/safe root"
  [ "$status" -ne 0 ]
  [[ "$output" == *'symlink path component refused'* ]]
  [ ! -e "$TMP/outside-local/bin" ]
  [ ! -e "$TMP/safe root" ]
}

@test "legacy release format is rejected without claiming an installation" {
  mkdir -p "$TMP/tree/dot-cli/tools/dot" "$TMP/tree/dot-cli/bin"
  cp "$REPO/tools/dot/dot" "$TMP/tree/dot-cli/tools/dot/dot"
  chmod +x "$TMP/tree/dot-cli/tools/dot/dot"
  for file in contract.py init.py engine.json; do : >"$TMP/tree/dot-cli/tools/dot/$file"; done
  printf '#!/bin/sh\nexit 0\n' >"$TMP/tree/dot-cli/bin/dot-browser"
  chmod +x "$TMP/tree/dot-cli/bin/dot-browser"
  cp "$REPO/install.sh" "$TMP/tree/dot-cli/install.sh"
  chmod +x "$TMP/tree/dot-cli/install.sh"
  printf '{"version": "1.2.3", "platform": "linux-amd64", "engine_api": 1}\n' >"$TMP/tree/dot-cli/release.json"
  tar -czf "$RELEASE_DIR/legacy.tar.gz" -C "$TMP/tree" dot-cli
  digest=$(shasum -a 256 "$RELEASE_DIR/legacy.tar.gz" | awk '{print $1}')
  run bash "$REPO/install.sh" --version 1.2.3 --platform linux-amd64 \
    --url "file://$RELEASE_DIR/legacy.tar.gz" --sha256 "$digest" \
    --root "$TMP/legacy install" --state-dir "$HOME/state/dot"
  [ "$status" -ne 0 ]
  [[ "$output" == *'release identity/format mismatch'* ]]
  [ ! -e "$TMP/legacy install/current" ]
  [ ! -e "$HOME/state/dot/install.json" ]
}

@test "installer promotes a verified public payload and writes an artifact ownership record" {
  mkdir -p "$TMP/tree/dot-cli/tools/dot" "$TMP/tree/dot-cli/bin"
  cp "$REPO/tools/dot/dot" "$TMP/tree/dot-cli/tools/dot/dot"
  chmod +x "$TMP/tree/dot-cli/tools/dot/dot"
  for file in contract.py init.py engine.json; do : >"$TMP/tree/dot-cli/tools/dot/$file"; done
  printf '#!/bin/sh\nexit 0\n' >"$TMP/tree/dot-cli/bin/dot-browser"
  chmod +x "$TMP/tree/dot-cli/bin/dot-browser"
  cp "$REPO/install.sh" "$TMP/tree/dot-cli/install.sh"
  chmod +x "$TMP/tree/dot-cli/install.sh"
  printf '{"format": 2, "version": "1.2.3", "platform": "linux-amd64", "engine_api": 1}\n' >"$TMP/tree/dot-cli/release.json"
  tar -czf "$RELEASE_DIR/good.tar.gz" -C "$TMP/tree" dot-cli
  digest=$(shasum -a 256 "$RELEASE_DIR/good.tar.gz" | awk '{print $1}')
  mkdir -p "$TMP/install root"
  printf 'keep me\n' >"$TMP/install root/unowned-note"
  run bash "$REPO/install.sh" --version 1.2.3 --platform linux-amd64 \
    --url "file://$RELEASE_DIR/good.tar.gz" --sha256 "$digest" \
    --root "$TMP/install root" --state-dir "$HOME/state/dot"
  [ "$status" -eq 0 ]
  [ -x "$TMP/install root/current/bin/dot-browser" ]
  [ "$(cat "$TMP/install root/unowned-note")" = 'keep me' ]
  [ -L "$HOME/.local/bin/dot" ] || [ -f "$HOME/.local/bin/dot" ]
  grep -q '"kind":"artifact"' "$HOME/state/dot/install.json"
  grep -q '"install_root":"'"$TMP"'/install root"' "$HOME/state/dot/install.json"
  ! grep -q 'private_source\|config_path' "$HOME/state/dot/install.json"
  python3 - "$HOME/state/dot" "$REPO/tools/dot" <<'PY'
import sys
sys.path.insert(0, sys.argv[2])
import install_state
record = install_state.read_install_record(sys.argv[1])
assert record['owner'] == {'kind': 'artifact', 'manager': 'curl'}
assert record['artifact']['release_tag'] == 'v1.2.3'
assert record['install_root'].endswith('/install root')
PY
  run env HOME="$HOME" XDG_STATE_HOME="$HOME/state" PATH="$MINIMAL_BIN" "$HOME/.local/bin/dot" --help
  [ "$status" -eq 0 ]
  run env HOME="$HOME" XDG_STATE_HOME="$HOME/state" PATH="$MINIMAL_BIN" "$HOME/.local/bin/dot" --version
  [ "$status" -eq 0 ]
  [[ "$output" == *'dot 1.2.3'* ]]

  sed 's/1\.2\.3/1.2.4/g' "$TMP/tree/dot-cli/release.json" >"$TMP/tree/dot-cli/release.new"
  mv "$TMP/tree/dot-cli/release.new" "$TMP/tree/dot-cli/release.json"
  tar -czf "$RELEASE_DIR/next.tar.gz" -C "$TMP/tree" dot-cli
  next_digest=$(shasum -a 256 "$RELEASE_DIR/next.tar.gz" | awk '{print $1}')
  printf '{"connection_version":1,"private_source":"%s/private","config_path":"%s/missing-config.toml"}\n' "$TMP" "$TMP" >"$HOME/state/dot/connection.json"
  record_before=$(cat "$HOME/state/dot/install.json")
  run env HOME="$HOME" XDG_STATE_HOME="$HOME/state" "$HOME/.local/bin/dot" self-update \
    --version 1.2.4 --platform linux-amd64 --url "file://$RELEASE_DIR/next.tar.gz" --sha256 "$next_digest" --yes
  [ "$status" -ne 0 ]
  [ "$(readlink "$TMP/install root/current")" = 'versions/1.2.3-linux-amd64' ]
  [ "$(cat "$HOME/state/dot/install.json")" = "$record_before" ]
  rm "$HOME/state/dot/connection.json"
  run env HOME="$HOME" XDG_STATE_HOME="$HOME/state" "$HOME/.local/bin/dot" self-update \
    --version 1.2.4 --platform linux-amd64 --url "file://$RELEASE_DIR/next.tar.gz" --sha256 "$next_digest"
  [ "$status" -eq 0 ]
  [[ "$output" == *'Preview only'* ]]
  [ "$(readlink "$TMP/install root/current")" = 'versions/1.2.3-linux-amd64' ]

  mkdir -p "$TMP/mv-failure"
  cat >"$TMP/mv-failure/mv" <<'SH'
#!/bin/sh
last=
for arg do last=$arg; done
[ "$last" != "$FAIL_MV_TARGET" ] || exit 1
exec /bin/mv "$@"
SH
  chmod +x "$TMP/mv-failure/mv"
  record_before=$(cat "$HOME/state/dot/install.json")
  run env HOME="$HOME" XDG_STATE_HOME="$HOME/state" FAIL_MV_TARGET="$HOME/state/dot/install.json" \
    PATH="$TMP/mv-failure:$PATH" "$HOME/.local/bin/dot" self-update \
    --version 1.2.4 --platform linux-amd64 --url "file://$RELEASE_DIR/next.tar.gz" --sha256 "$next_digest" --yes
  [ "$status" -ne 0 ]
  [ "$(readlink "$TMP/install root/current")" = 'versions/1.2.3-linux-amd64' ]
  [ "$(cat "$HOME/state/dot/install.json")" = "$record_before" ]
  [ ! -e "$TMP/install root/versions/1.2.4-linux-amd64" ]

  run env HOME="$HOME" XDG_STATE_HOME="$HOME/state" "$HOME/.local/bin/dot" self-update \
    --version 1.2.4 --platform linux-amd64 --url "file://$RELEASE_DIR/next.tar.gz" --sha256 "$next_digest" --yes
  [ "$status" -eq 0 ]
  [ "$(readlink "$TMP/install root/current")" = 'versions/1.2.4-linux-amd64' ]
}

@test "installed launcher pins runtime state to the approved exact directory" {
  mkdir -p "$TMP/tree/dot-cli/tools" "$TMP/tree/dot-cli/bin" "$TMP/tree/dot-cli"
  cp -R "$REPO/tools/dot" "$TMP/tree/dot-cli/tools/dot"
  printf '#!/bin/sh\nexit 0\n' >"$TMP/tree/dot-cli/bin/dot-browser"
  chmod +x "$TMP/tree/dot-cli/bin/dot-browser"
  cp "$REPO/install.sh" "$TMP/tree/dot-cli/install.sh"
  chmod +x "$TMP/tree/dot-cli/install.sh"
  printf '{"format": 2, "version": "1.2.3", "platform": "linux-amd64", "engine_api": 1}\n' >"$TMP/tree/dot-cli/release.json"
  tar -czf "$RELEASE_DIR/state.tar.gz" -C "$TMP/tree" dot-cli
  digest=$(shasum -a 256 "$RELEASE_DIR/state.tar.gz" | awk '{print $1}')
  legacy="$HOME/.local/state/dot"
  isolated="$HOME/.local/state/dot cli's"
  mkdir -p "$legacy"
  printf 'legacy install bytes\n' >"$legacy/install.json"
  printf 'legacy approval bytes\n' >"$legacy/adoption.json"
  printf 'legacy recovery bytes\n' >"$legacy/recovery"
  cp -R "$legacy" "$TMP/legacy-before"
  run bash "$REPO/install.sh" --version 1.2.3 --platform linux-amd64 \
    --url "file://$RELEASE_DIR/state.tar.gz" --sha256 "$digest" \
    --root "$TMP/install root" --state-dir "$isolated"
  [ "$status" -eq 0 ]
  [ -f "$isolated/install.json" ]
  grep -Fq '"record_version":2' "$isolated/install.json"
  grep -Fq '"state_dir":"'"$isolated"'"' "$isolated/install.json"
  grep -Fq 'DOT_STATE_DIR=' "$HOME/.local/bin/dot"
  mkdir -m 700 "$TMP/relocated-state"
  cp "$isolated/install.json" "$TMP/relocated-state/install.json"
  chmod 600 "$TMP/relocated-state/install.json"
  run bash "$REPO/install.sh" --version 1.2.3 --platform linux-amd64 \
    --url "file://$RELEASE_DIR/state.tar.gz" --sha256 "$digest" \
    --root "$TMP/install root" --state-dir "$TMP/relocated-state"
  [ "$status" -ne 0 ]
  [[ "$output" == *'recorded state directory mismatch'* ]]
  printf '{"version":1,"groups":{}}\n' >"$isolated/adoption.json"
  fixture_python=
  for candidate in /private/tmp/py312bin/python3 python3.16 python3.15 python3.14 python3.13 python3.12 python3.11 python3; do
    if [ -x "$candidate" ]; then python_path="$candidate"; else python_path="$(command -v "$candidate" 2>/dev/null || true)"; fi
    if [ -n "$python_path" ] && "$python_path" -c 'import sys,tomllib; sys.exit(sys.version_info < (3,11))' >/dev/null 2>&1; then
      fixture_python="$python_path"
      break
    fi
  done
  [ -n "$fixture_python" ]
  fixture_path="${fixture_python%/*}:$PATH"
  run env HOME="$HOME" XDG_STATE_HOME="$HOME/xdg" DOT_STATE_DIR="$legacy" PATH="$fixture_path" "$HOME/.local/bin/dot" init --check
  [ "$status" -eq 0 ]
  [[ "$output" == *"Install record: $isolated/install.json"* ]]
  [[ "$output" != *"Install record: $legacy/install.json"* ]]
  run env HOME="$HOME" DOT_STATE_DIR="$isolated" PYTHONPATH="$REPO/tools/dot" "$fixture_python" - "$isolated" <<'PY'
import json, sys
from pathlib import Path
import file_metadata, init, install_state, migration, scope, state_paths, tracking
home = Path.home()
selected = Path(sys.argv[1])
assert state_paths.state_dir() == selected
assert init.local_state_dir() == selected
assert scope.profile_path(home) == selected / 'adoption.json'
assert migration.paths(home)['state'] == selected
assert scope.load(str(home)) == {'version': 1, 'groups': {}}
assert install_state.read_install_record(selected)['owner'] == {'kind': 'artifact', 'manager': 'curl'}
tracking.pending(home)
assert not (home / '.local/state/dot/adoption.json').read_bytes().startswith(b'{"version":1')
PY
  [ "$status" -eq 0 ]
  diff -r "$TMP/legacy-before" "$legacy"
}

@test "installer refuses package-manager ownership without touching its prefix" {
  mkdir -p "$HOME/state/dot" "$TMP/managed root"
  printf '{"record_version":1,"owner":{"kind":"package-manager","manager":"brew"}}\n' >"$HOME/state/dot/install.json"
  run bash "$REPO/install.sh" --version 1.2.3 --platform darwin-arm64 \
    --url "file://$RELEASE_DIR/missing" --sha256 "$(printf '%064d' 0)" \
    --root "$TMP/managed root" --state-dir "$HOME/state/dot"
  [ "$status" -ne 0 ]
  [[ "$output" == *'brew upgrade dots-cli'* ]]
  [ ! -e "$TMP/managed root/current" ]
}

@test "installer rejects archive symlinks before promotion" {
  mkdir -p "$TMP/tree/dot-cli/tools/dot" "$TMP/tree/dot-cli/bin"
  for file in dot contract.py init.py engine.json; do : >"$TMP/tree/dot-cli/tools/dot/$file"; done
  printf '#!/bin/sh\nexit 0\n' >"$TMP/tree/dot-cli/bin/dot-browser"
  chmod +x "$TMP/tree/dot-cli/bin/dot-browser"
  printf '{"format": 2, "version": "1.2.3", "platform": "linux-amd64", "engine_api": 1}\n' >"$TMP/tree/dot-cli/release.json"
  ln -s ../../../../outside "$TMP/tree/dot-cli/escape"
  tar -czf "$RELEASE_DIR/link.tar.gz" -C "$TMP/tree" dot-cli
  digest=$(shasum -a 256 "$RELEASE_DIR/link.tar.gz" | awk '{print $1}')
  run bash "$REPO/install.sh" --version 1.2.3 --platform linux-amd64 \
    --url "file://$RELEASE_DIR/link.tar.gz" --sha256 "$digest" \
    --root "$TMP/link install" --state-dir "$HOME/state/dot"
  [ "$status" -ne 0 ]
  [ ! -e "$TMP/link install/current" ]
}

@test "installer rejects traversal archive entries before extraction" {
  python3 - "$RELEASE_DIR/traversal.tar.gz" <<'PY'
import io, sys, tarfile
with tarfile.open(sys.argv[1], 'w:gz') as archive:
    info = tarfile.TarInfo('dot-cli/../escaped')
    content = b'not written'
    info.size = len(content)
    archive.addfile(info, io.BytesIO(content))
PY
  digest=$(shasum -a 256 "$RELEASE_DIR/traversal.tar.gz" | awk '{print $1}')
  run bash "$REPO/install.sh" --version 1.2.3 --platform linux-amd64 \
    --url "file://$RELEASE_DIR/traversal.tar.gz" --sha256 "$digest" \
    --root "$TMP/traversal install"
  [ "$status" -ne 0 ]
  [[ "$output" == *'unsafe archive member paths'* ]]
  [ ! -e "$TMP/escaped" ]
  [ ! -e "$TMP/traversal install" ]
}

@test "dot self-update advises the package manager and never writes its prefix" {
  mkdir -p "$HOME/.local/state/dot"
  printf '{"record_version":1,"owner":{"kind":"package-manager","manager":"pacman"}}\n' >"$HOME/.local/state/dot/install.json"
  export DOT_ENGINE="$REPO/tools/dot"
  run "$REPO/tools/dot/dot" self-update --yes
  [ "$status" -ne 0 ]
  [[ "$output" == *'pacman -Syu'* ]]
  [ "$(cat "$HOME/.local/state/dot/install.json")" = '{"record_version":1,"owner":{"kind":"package-manager","manager":"pacman"}}' ]
}

@test "fresh tooling install and CLI help/version need no Python Git or Go" {
  mkdir -p "$TMP/tree/dot-cli/tools/dot" "$TMP/tree/dot-cli/bin" "$TMP/minimal-tools"
  cp "$REPO/tools/dot/dot" "$TMP/tree/dot-cli/tools/dot/dot"
  chmod +x "$TMP/tree/dot-cli/tools/dot/dot"
  for file in contract.py init.py engine.json; do : >"$TMP/tree/dot-cli/tools/dot/$file"; done
  printf '#!/bin/sh\nexit 0\n' >"$TMP/tree/dot-cli/bin/dot-browser"
  chmod +x "$TMP/tree/dot-cli/bin/dot-browser"
  printf '{"format": 2, "version": "2.0.0", "platform": "linux-amd64", "engine_api": 1}\n' >"$TMP/tree/dot-cli/release.json"
  cp "$REPO/install.sh" "$TMP/tree/dot-cli/install.sh"
  chmod +x "$TMP/tree/dot-cli/install.sh"
  tar -czf "$RELEASE_DIR/minimal.tar.gz" -C "$TMP/tree" dot-cli
  digest=$(shasum -a 256 "$RELEASE_DIR/minimal.tar.gz" | awk '{print $1}')
  for tool in sh awk grep dirname mktemp cp tar wc tr mkdir mv readlink chmod cat rm ln uname shasum sed env; do
    command -v "$tool" >/dev/null 2>&1 && ln -s "$(command -v "$tool")" "$TMP/minimal-tools/$tool"
  done
  ln -s /bin/bash "$TMP/minimal-tools/bash"
  ! PATH="$TMP/minimal-tools" command -v python3
  ! PATH="$TMP/minimal-tools" command -v git
  ! PATH="$TMP/minimal-tools" command -v go
  run /bin/bash "$REPO/install.sh" --version 2.0.0 --platform linux-amd64 \
    --url "file://$RELEASE_DIR/minimal.tar.gz" --sha256 "$digest" \
    --root "$TMP/no-runtime install" --state-dir "$HOME/state/dot"
  [ "$status" -eq 0 ]
  run env HOME="$HOME" PATH="$TMP/minimal-tools" "$HOME/.local/bin/dot" --help
  [ "$status" -eq 0 ]
  run env HOME="$HOME" PATH="$TMP/minimal-tools" "$HOME/.local/bin/dot" --version
  [ "$status" -eq 0 ]
  [[ "$output" == *'dot 2.0.0'* ]]
}
