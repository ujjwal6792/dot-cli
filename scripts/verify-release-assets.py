#!/usr/bin/env python3
"""Verify GitHub-reported release asset names and SHA-256 digests locally."""
import argparse
import hashlib
import json
from pathlib import Path
import re


def verify(directory, uploaded):
    expected = {path.name: path for path in directory.iterdir() if path.is_file() and not path.is_symlink()}
    if not expected:
        raise ValueError('no local release assets')
    if any(path.is_symlink() or not path.is_file() for path in directory.iterdir()):
        raise ValueError('release directory contains a non-regular entry')
    if not isinstance(uploaded, list):
        raise ValueError('GitHub asset response must be a list')

    actual = {}
    for item in uploaded:
        if not isinstance(item, dict) or set(item) != {'name', 'size', 'digest'}:
            raise ValueError('unexpected GitHub asset metadata')
        name, size, digest = item['name'], item['size'], item['digest']
        if (not isinstance(name, str) or name in actual or not isinstance(size, int)
                or isinstance(size, bool) or size < 0 or not isinstance(digest, str)):
            raise ValueError('invalid or duplicate GitHub asset')
        if not re.fullmatch(r'sha256:[0-9a-f]{64}', digest):
            raise ValueError('GitHub did not report a SHA-256 asset digest: ' + name)
        actual[name] = (size, digest.removeprefix('sha256:'))
    if set(actual) != set(expected):
        raise ValueError('uploaded asset names differ from the complete verified package set')
    for name, path in expected.items():
        size, digest = actual[name]
        if path.stat().st_size != size:
            raise ValueError('uploaded asset size mismatch: ' + name)
        if hashlib.sha256(path.read_bytes()).hexdigest() != digest:
            raise ValueError('uploaded asset digest mismatch: ' + name)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    parser.add_argument('github_assets', type=Path, help='JSON array of GitHub asset name/digest records')
    args = parser.parse_args()
    try:
        verify(args.directory, json.loads(args.github_assets.read_text()))
    except (ValueError, OSError, json.JSONDecodeError) as error:
        parser.exit(1, 'release upload check: ' + str(error) + '\n')
    print('GitHub asset names and SHA-256 digests match the complete local release set.')


if __name__ == '__main__':
    main()
