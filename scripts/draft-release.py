#!/usr/bin/env python3
"""Safely verify and publish a staged GitHub draft release by its release ID."""
import argparse
import json
import subprocess
import sys
from pathlib import Path

from importlib.machinery import SourceFileLoader

ASSETS = SourceFileLoader(
    'verify_release_assets', str(Path(__file__).with_name('verify-release-assets.py'))
).load_module()


def gh(*args):
    return subprocess.run(['gh', 'api', *args], check=True, text=True, capture_output=True).stdout


def list_releases(repository, tag):
    raw = gh('--paginate', f'repos/{repository}/releases')
    decoder = json.JSONDecoder()
    cursor = 0
    found = []
    while cursor < len(raw):
        while cursor < len(raw) and raw[cursor].isspace():
            cursor += 1
        if cursor == len(raw):
            break
        page, cursor = decoder.raw_decode(raw, cursor)
        if not isinstance(page, list):
            raise ValueError('GitHub release listing was not an array')
        found.extend(item for item in page if item.get('tag_name') == tag)
    return found


def select_release(releases):
    if len(releases) > 1:
        raise ValueError('multiple releases have the requested tag; refusing to choose')
    return releases[0] if releases else None


def verify_identity(release, tag, target):
    if (release.get('tag_name') != tag or release.get('target_commitish') != target
            or release.get('draft') is not True or release.get('prerelease') is not True):
        raise ValueError('release identity/state does not match the approved draft')
    if not isinstance(release.get('id'), int):
        raise ValueError('GitHub release has no valid release ID')


def prepare(repository, tag, target, directory):
    matches = list_releases(repository, tag)
    selected = select_release(matches)
    if selected is None:
        raise ValueError('no staged draft release found; create/upload must happen before verification')
    release_id = selected['id']
    release = json.loads(gh(f'repos/{repository}/releases/{release_id}'))
    verify_identity(release, tag, target)
    if release.get('id') != release_id:
        raise ValueError('release ID lookup returned a different release')
    ASSETS.verify(directory, [
        {'name': item.get('name'), 'digest': item.get('digest')}
        for item in release.get('assets', [])
    ])
    return release_id


def publish(repository, release_id):
    gh('--method', 'PATCH', f'repos/{repository}/releases/{release_id}', '-F', 'draft=false', '-F', 'prerelease=true')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repository', required=True)
    parser.add_argument('--tag', required=True)
    parser.add_argument('--target', required=True)
    parser.add_argument('--directory', type=Path, required=True)
    parser.add_argument('--inspect-only', action='store_true')
    parser.add_argument('--allow-missing', action='store_true')
    args = parser.parse_args()
    try:
        try:
            release_id = prepare(args.repository, args.tag, args.target, args.directory)
        except ValueError as error:
            if args.allow_missing and str(error) == 'no staged draft release found; create/upload must happen before verification':
                print('')
                return 0
            raise
        if args.inspect_only:
            print(release_id)
            return 0
        publish(args.repository, release_id)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print(f'draft release publish refused: {error}', file=sys.stderr)
        return 1
    print(f'Published verified release ID {release_id}')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
