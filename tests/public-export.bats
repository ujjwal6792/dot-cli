#!/usr/bin/env bats
# Public export boundary: engine contract out, private catalog/history never in.
# Runs against the public checkout itself with synthetic fixtures only.

setup() {
  REPO="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"
  EXPORT="$BATS_TEST_TMPDIR/export"
  mkdir -p "$BATS_TEST_TMPDIR/no-go"
  printf '#!/bin/sh\necho Go-must-not-be-invoked >&2\nexit 97\n' > "$BATS_TEST_TMPDIR/no-go/go"
  chmod +x "$BATS_TEST_TMPDIR/no-go/go"
  export PATH="$BATS_TEST_TMPDIR/no-go:$PATH"
}

@test "release packages reviewed fixture binaries for each platform without invoking Go" {
  mkdir -p "$BATS_TEST_TMPDIR/binaries"
  for platform in darwin-arm64 linux-amd64 linux-arm64; do
    printf '#!/bin/sh\necho fixture-browser\n' > "$BATS_TEST_TMPDIR/binaries/dot-browser-$platform"
  done
  run python3 "$REPO/scripts/build-release.py" --version 0.1.0 --output "$EXPORT" --binaries "$BATS_TEST_TMPDIR/binaries"
  [ "$status" -eq 0 ]
  [ -f "$EXPORT/SHA256SUMS" ]
  [ -f "$EXPORT/BROWSER_SHA256SUMS" ]
  run "$EXPORT/dot-browser-0.1.0-darwin-arm64"
  [ "$status" -eq 0 ]
  [ "$output" = fixture-browser ]
  tar -xzf "$EXPORT/dot-cli-0.1.0-darwin-arm64.tar.gz" -C "$BATS_TEST_TMPDIR"
  run "$BATS_TEST_TMPDIR/dot-cli/bin/dot-browser"
  [ "$status" -eq 0 ]
  [ "$output" = fixture-browser ]
  [ ! -e "$BATS_TEST_TMPDIR/dot-cli/tools/dot/manifest.json" ]
  [ -f "$BATS_TEST_TMPDIR/dot-cli/release.json" ]
  python3 "$REPO/scripts/check-release.py" "$EXPORT"
  printf 'tampered\n' >> "$EXPORT/dot-browser-0.1.0-linux-amd64"
  run python3 "$REPO/scripts/check-release.py" "$EXPORT"
  [ "$status" -ne 0 ]
  [[ "$output" == *'browser checksum mismatch'* ]]
  python3 - "$EXPORT" <<'PY'
import hashlib, pathlib, sys
root = pathlib.Path(sys.argv[1])
lines = (root / 'SHA256SUMS').read_text().splitlines()
assert len(lines) == 3
for line in lines:
    digest, name = line.split('  ')
    assert hashlib.sha256((root / name).read_bytes()).hexdigest() == digest
import tarfile
for archive in root.glob('*.tar.gz'):
    with tarfile.open(archive) as stream:
        assert all(m.uid == 0 and m.gid == 0 and m.uname == '' and m.gname == '' for m in stream)
PY
}

@test "public export carries engine contract but never private catalog or Git history" {
  run python3 "$REPO/scripts/export-public.py" --output "$EXPORT"
  [ "$status" -eq 0 ]
  [ -f "$EXPORT/tools/dot/engine.json" ]
  [ -f "$EXPORT/tools/dot/intent.py" ]
  [ -f "$EXPORT/tools/dot/packages.py" ]
  [ -f "$EXPORT/tools/dot/browser/packages.go" ]
  [ -f "$EXPORT/tools/dot/migration.py" ]
  [ -f "$EXPORT/bootstrap.sh" ]
  [ -f "$EXPORT/skills/dot/SKILL.md" ]
  [ ! -e "$EXPORT/tools/dot/manifest.json" ]
  [ ! -e "$EXPORT/tools/dot/groups.json" ]
  [ ! -e "$EXPORT/home" ]
  [ ! -e "$EXPORT/.git" ]
  [ ! -e "$EXPORT/packages" ]
  [ ! -e "$EXPORT/packages/.backups" ]
}

@test "independent public checkout can export and package itself without private repository" {
  python3 "$REPO/scripts/export-public.py" --output "$EXPORT"
  [ -f "$EXPORT/scripts/build-release.py" ]
  [ -f "$EXPORT/scripts/check-release.py" ]
  [ -f "$EXPORT/.github/workflows/release.yml" ]
  python3 "$EXPORT/scripts/export-public.py" --output "$BATS_TEST_TMPDIR/second"
  diff -r "$EXPORT" "$BATS_TEST_TMPDIR/second"
  mkdir -p "$BATS_TEST_TMPDIR/binaries"
  for platform in darwin-arm64 linux-amd64 linux-arm64; do
    printf '#!/bin/sh\necho fixture-browser\n' > "$BATS_TEST_TMPDIR/binaries/dot-browser-$platform"
  done
  python3 "$EXPORT/scripts/build-release.py" --version 0.1.0 --output "$BATS_TEST_TMPDIR/release" --binaries "$BATS_TEST_TMPDIR/binaries"
  python3 "$EXPORT/scripts/check-release.py" "$BATS_TEST_TMPDIR/release"
}

@test "installed acceptance refuses a swallowed target Go invocation" {
  mkdir -p "$EXPORT/tools/dot" "$EXPORT/bin" "$BATS_TEST_TMPDIR/fake-bats"
  touch "$EXPORT/tools/dot/dot" "$EXPORT/bootstrap.sh" "$EXPORT/bin/dot-browser"
  cat > "$BATS_TEST_TMPDIR/fake-bats/bats" <<'SH'
#!/bin/sh
go version >/dev/null 2>&1 || true
exit 0
SH
  chmod +x "$BATS_TEST_TMPDIR/fake-bats/bats"
  run env PATH="$BATS_TEST_TMPDIR/fake-bats:$PATH" python3 "$REPO/scripts/accept-installed.py" --release "$EXPORT" --temporary-directory "$BATS_TEST_TMPDIR"
  [ "$status" -ne 0 ]
  [[ "$output" == *'target Go was invoked, even if its failure was swallowed'* ]]
}

@test "release verification rejects private metadata smuggled into an archive" {
  mkdir -p "$BATS_TEST_TMPDIR/binaries"
  for platform in darwin-arm64 linux-amd64 linux-arm64; do
    printf '#!/bin/sh\necho fixture-browser\n' > "$BATS_TEST_TMPDIR/binaries/dot-browser-$platform"
  done
  python3 "$REPO/scripts/build-release.py" --version 0.1.0 --output "$EXPORT" --binaries "$BATS_TEST_TMPDIR/binaries"
  run python3 "$REPO/scripts/check-release.py" "$EXPORT"
  [ "$status" -eq 0 ]
  printf 'private history\n' > "$EXPORT/history.json"
  run python3 "$REPO/scripts/check-release.py" "$EXPORT"
  [ "$status" -ne 0 ]
  [[ "$output" == *'unexpected release-directory payload'* ]]
  rm "$EXPORT/history.json"
  python3 - "$EXPORT" <<'PY'
import hashlib, io, pathlib, sys, tarfile
root = pathlib.Path(sys.argv[1]); archive = root / 'dot-cli-0.1.0-linux-amd64.tar.gz'
with tarfile.open(archive) as stream:
    files = [(member, stream.extractfile(member).read() if member.isfile() else None) for member in stream]
with tarfile.open(archive, 'w:gz') as stream:
    for member, content in files:
        stream.addfile(member, io.BytesIO(content) if content is not None else None)
    member = tarfile.TarInfo('dot-cli/tools/dot/manifest.json'); member.size = 2
    stream.addfile(member, io.BytesIO(b'{}'))
(root / 'SHA256SUMS').write_text('\n'.join(hashlib.sha256(p.read_bytes()).hexdigest() + '  ' + p.name for p in sorted(root.glob('*.tar.gz'))) + '\n')
PY
  run python3 "$REPO/scripts/check-release.py" "$EXPORT"
  [ "$status" -ne 0 ]
  [[ "$output" == *'unexpected payload'* ]]
}

@test "release metadata cannot carry private inventory fields even with valid checksums" {
  mkdir -p "$BATS_TEST_TMPDIR/binaries"
  for platform in darwin-arm64 linux-amd64 linux-arm64; do
    printf '#!/bin/sh\necho fixture-browser\n' > "$BATS_TEST_TMPDIR/binaries/dot-browser-$platform"
  done
  python3 "$REPO/scripts/build-release.py" --version 0.1.0 --output "$EXPORT" --binaries "$BATS_TEST_TMPDIR/binaries"
  python3 - "$EXPORT" <<'PY'
import hashlib, io, json, pathlib, sys, tarfile
root = pathlib.Path(sys.argv[1]); archive = root / 'dot-cli-0.1.0-linux-amd64.tar.gz'
with tarfile.open(archive) as stream:
    files = [(member, stream.extractfile(member).read() if member.isfile() else None) for member in stream]
with tarfile.open(archive, 'w:gz') as stream:
    for member, content in files:
        if member.name == 'dot-cli/release.json':
            data = json.loads(content); data['inventory'] = ['private/fixture']
            content = json.dumps(data).encode(); member.size = len(content)
        stream.addfile(member, io.BytesIO(content) if content is not None else None)
(root / 'SHA256SUMS').write_text('\n'.join(hashlib.sha256(p.read_bytes()).hexdigest() + '  ' + p.name for p in sorted(root.glob('*.tar.gz'))) + '\n')
PY
  run python3 "$REPO/scripts/check-release.py" "$EXPORT"
  [ "$status" -ne 0 ]
  [[ "$output" == *'unexpected release metadata'* ]]
}
