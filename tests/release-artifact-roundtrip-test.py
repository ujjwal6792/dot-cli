#!/usr/bin/env python3
"""Exercise release validation across artifact mode normalization."""
import hashlib
from pathlib import Path
import subprocess
import sys
import tempfile

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
    candidate = temporary / 'candidate'
    inputs.mkdir()
    for platform in PLATFORMS:
        (inputs / f'dot-browser-{platform}').write_bytes(f'fixture:{platform}'.encode())

    run(sys.executable, 'scripts/build-release.py', '--version', VERSION,
        '--output', str(candidate), '--binaries', str(inputs))
    original_hashes = {
        path.name: hashlib.sha256(path.read_bytes()).hexdigest()
        for path in candidate.glob('dot-browser-*')
    }
    run(sys.executable, 'scripts/check-release.py', str(candidate))

    # actions/download-artifact normalizes file modes; preserve its byte payload.
    for browser in candidate.glob('dot-browser-*'):
        browser.chmod(0o644)
    failed = run(sys.executable, 'scripts/check-release.py', str(candidate), expected=1)
    assert 'browser checksum mismatch: dot-browser-0.1.0-rc.2-darwin-arm64' in failed.stderr

    run(sys.executable, 'scripts/restore-release-executables.py', str(candidate))
    run(sys.executable, 'scripts/check-release.py', str(candidate))
    assert original_hashes == {
        path.name: hashlib.sha256(path.read_bytes()).hexdigest()
        for path in candidate.glob('dot-browser-*')
    }
