#!/usr/bin/env python3
"""Restore executable mode on standalone browsers after artifact transport."""
import argparse
from pathlib import Path
import re
import stat

PLATFORMS = ('darwin-arm64', 'linux-amd64', 'linux-arm64')
VERSION = re.compile(r'[0-9]+\.[0-9]+\.[0-9]+(?:-[A-Za-z0-9.-]+)?')


def restore(directory, version):
    if not VERSION.fullmatch(version):
        raise ValueError('invalid release version')
    if directory.is_symlink() or not directory.is_dir():
        raise ValueError('candidate directory must be a real directory')

    expected = {f'dot-browser-{version}-{platform}' for platform in PLATFORMS}
    matching = {path.name for path in directory.iterdir() if path.name.startswith('dot-browser-')}
    unexpected = matching - expected
    if unexpected:
        raise ValueError('unexpected browser asset: ' + sorted(unexpected)[0])
    if matching != expected:
        raise ValueError('missing standalone browser asset')

    browsers = [directory / name for name in sorted(expected)]
    for browser in browsers:
        if browser.is_symlink() or not browser.is_file():
            raise ValueError('invalid standalone browser asset: ' + browser.name)

    # Validate every target before changing any mode, then let check-release.py
    # independently verify exact names, executability, and all SHA-256 values.
    for browser in browsers:
        browser.chmod(stat.S_IMODE(browser.stat().st_mode) | 0o111)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    parser.add_argument('version')
    args = parser.parse_args()
    try:
        restore(args.directory, args.version)
    except (OSError, ValueError) as error:
        parser.exit(1, f'release mode restore: {error}\n')
