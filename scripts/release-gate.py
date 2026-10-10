#!/usr/bin/env python3
"""Fail closed unless this is an explicitly approved dispatch from current main."""
import argparse
import re

REPOSITORY = 'ujjwal6792/dots-cli'
VERSION = re.compile(r'v0\.1\.0-rc\.([1-9][0-9]*)\Z')
TAG = re.compile(r'v0\.1\.0-rc\.([1-9][0-9]*)\Z')
SHA = re.compile(r'[0-9a-f]{40}\Z')


def validate(context):
    if context['repository'] != REPOSITORY:
        raise ValueError('release publication is restricted to ' + REPOSITORY)
    if context['event_name'] != 'workflow_dispatch':
        raise ValueError('release publication requires an explicit workflow_dispatch')
    if context['ref'] != 'refs/heads/main':
        raise ValueError('release publication can only run from refs/heads/main')
    sha = context['sha'].lower()
    main = context['current_main'].lower()
    if not SHA.fullmatch(sha) or not SHA.fullmatch(main) or sha != main:
        raise ValueError('dispatch commit must equal current main')

    match = VERSION.fullmatch(context['version'])
    if not match:
        raise ValueError('version must be a new v0.1.0-rc.N prerelease (N >= 2)')
    requested = int(match.group(1))
    if requested < 2:
        raise ValueError('version must be a new v0.1.0-rc.N prerelease (N >= 2)')
    highest = 1  # rc.1 is immutable and must never be reissued.
    for tag in context['existing_tags'].splitlines():
        tag_match = TAG.fullmatch(tag.strip())
        if tag_match:
            number = int(tag_match.group(1))
            if number == requested:
                raise ValueError('tag already exists: ' + tag.strip())
            highest = max(highest, number)
    if requested <= highest:
        raise ValueError('requested prerelease number must be greater than all existing RCs')
    return context['version']


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('repository', 'event-name', 'ref', 'sha', 'current-main', 'version'):
        parser.add_argument('--' + name, required=True)
    parser.add_argument('--existing-tags', default='')
    args = parser.parse_args()
    context = vars(args)
    try:
        version = validate(context)
    except ValueError as error:
        parser.exit(1, 'release gate: ' + str(error) + '\n')
    print('Eligible release version: ' + version)


if __name__ == '__main__':
    main()
