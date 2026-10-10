#!/usr/bin/env python3
"""Restore executable mode on standalone browsers after artifact transport."""
import argparse
from pathlib import Path
import stat


def restore(directory):
    browsers = sorted(directory.glob('dot-browser-*'))
    if not browsers:
        raise ValueError('no standalone browser assets found')
    for browser in browsers:
        if browser.is_symlink() or not browser.is_file():
            raise ValueError('invalid standalone browser asset: ' + browser.name)
        browser.chmod(stat.S_IMODE(browser.stat().st_mode) | 0o111)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    args = parser.parse_args()
    try:
        restore(args.directory)
    except (OSError, ValueError) as error:
        parser.exit(1, f'release mode restore: {error}\n')
