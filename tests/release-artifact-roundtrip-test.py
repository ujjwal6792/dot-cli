#!/usr/bin/env python3
"""Exercise release validation across artifact mode normalization."""
import hashlib
from pathlib import Path
import subprocess
import sys
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]
VERSION = '0.1.0-rc.2'
PLATFORMS = ('darwin-arm64', 'linux-amd64', 'linux-arm64')


def run(*args, expected=0):
    result = subprocess.run(args, cwd=ROOT, text=True, capture_output=True)
    if result.returncode != expected:
        raise AssertionError(f'{args} exited {result.returncode}: {result.stderr}')
    return result


with tempfile.TemporaryDirectory(prefix='release-roundtrip-') as temporary:
    temporary = Path(temporary)
    inputs = temporary / 'browsers'
    built = temporary / 'built'
    artifact = temporary / 'candidate.zip'
    downloaded = temporary / 'candidate'
    inputs.mkdir()
    for platform in PLATFORMS:
        (inputs / f'dot-browser-{platform}').write_bytes(f'fixture:{platform}'.encode())

    run(sys.executable, 'scripts/build-release.py', '--version', VERSION,
        '--output', str(built), '--binaries', str(inputs))
    original_hashes = {
        path.name: hashlib.sha256(path.read_bytes()).hexdigest()
        for path in built.glob('dot-browser-*')
    }
    run(sys.executable, 'scripts/check-release.py', str(built))

    # Model an Actions artifact ZIP roundtrip: retain asset bytes and paths, not modes.
    with zipfile.ZipFile(artifact, 'w') as archive:
        for path in built.iterdir():
            archive.write(path, path.name)
    downloaded.mkdir()
    with zipfile.ZipFile(artifact) as archive:
        archive.extractall(downloaded)

    failed = run(sys.executable, 'scripts/check-release.py', str(downloaded), expected=1)
    assert 'browser checksum mismatch: dot-browser-0.1.0-rc.2-darwin-arm64' in failed.stderr
    for path in downloaded.glob('dot-browser-*'):
        assert not path.stat().st_mode & 0o111

    run(sys.executable, 'scripts/restore-release-executables.py', str(downloaded), VERSION)
    run(sys.executable, 'scripts/check-release.py', str(downloaded))
    assert original_hashes == {
        path.name: hashlib.sha256(path.read_bytes()).hexdigest()
        for path in downloaded.glob('dot-browser-*')
    }

    # Mode repair must not make a corrupted browser pass strict SHA-256 checks.
    browser = downloaded / f'dot-browser-{VERSION}-linux-amd64'
    browser.write_bytes(browser.read_bytes() + b'corruption')
    run(sys.executable, 'scripts/restore-release-executables.py', str(downloaded), VERSION)
    failed = run(sys.executable, 'scripts/check-release.py', str(downloaded), expected=1)
    assert f'browser checksum mismatch: {browser.name}' in failed.stderr

    # Unexpected matching names are rejected before changing any file modes.
    for path in downloaded.glob('dot-browser-*'):
        path.chmod(0o644)
    unexpected = downloaded / f'dot-browser-{VERSION}-unsupported'
    unexpected.write_bytes(b'not a supported platform')
    failed = run(sys.executable, 'scripts/restore-release-executables.py',
                 str(downloaded), VERSION, expected=1)
    assert 'unexpected browser asset' in failed.stderr
    assert all(not path.stat().st_mode & 0o111 for path in downloaded.glob('dot-browser-*'))

    # A symlinked expected asset is rejected without partially restoring modes.
    invalid = temporary / 'invalid'
    invalid.mkdir()
    for platform in PLATFORMS:
        (invalid / f'dot-browser-{VERSION}-{platform}').write_bytes(b'fixture')
        (invalid / f'dot-browser-{VERSION}-{platform}').chmod(0o644)
    linked = invalid / f'dot-browser-{VERSION}-linux-arm64'
    linked.unlink()
    linked.symlink_to(invalid / f'dot-browser-{VERSION}-darwin-arm64')
    failed = run(sys.executable, 'scripts/restore-release-executables.py',
                 str(invalid), VERSION, expected=1)
    assert 'invalid standalone browser asset' in failed.stderr
    assert not (invalid / f'dot-browser-{VERSION}-darwin-arm64').stat().st_mode & 0o111
    run(sys.executable, 'scripts/restore-release-executables.py', str(invalid), '../escape', expected=1)

# The same restore-before-check sequence is required in both independent
# artifact consumers: installed-smoke and publish.
workflow = (ROOT / '.github/workflows/release.yml').read_text()
jobs = ('verify', 'build', 'installed-smoke', 'publish')
for job in ('installed-smoke', 'publish'):
    start = workflow.index(f'\n  {job}:\n') + 1
    following = [workflow.find(f'\n  {other}:\n', start) for other in jobs]
    following = [position for position in following if position >= 0]
    section = workflow[start:min(following, default=len(workflow))]
    restore = 'scripts/restore-release-executables.py candidate "$VERSION"'
    check = 'python3 scripts/check-release.py candidate'
    assert restore in section and section.index(restore) < section.index(check)
