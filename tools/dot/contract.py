#!/usr/bin/env python3
"""Read-only public/private compatibility negotiation; never imports private code."""
import argparse
import json
import os
from pathlib import Path
import sys


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError('duplicate JSON field')
        result[key] = value
    return result


def read_json(path):
    return json.loads(path.read_text(), object_pairs_hook=unique_object)


def object_fields(data, fields):
    if not isinstance(data, dict) or set(data) != set(fields):
        raise ValueError('unsupported descriptor fields')


def revision(value, expected):
    if type(value) is not int or value != expected:
        raise ValueError('unsupported version')


def bounded(root, value, directory=False):
    if (not isinstance(value, str) or not value or
            any(part in ('', '.', '..') for part in value.split('/')) or
            Path(value).is_absolute() or
            any(ord(char) < 32 or ord(char) == 127 for char in value) or
            '\\' in value):
        raise ValueError('invalid contract path')
    path = root / value
    if path.resolve() != path or not (path.is_dir() if directory else path.is_file()):
        raise ValueError('missing or symlinked contract path')
    return path


def check(engine, source):
    if not engine.is_absolute() or not source.is_absolute():
        raise ValueError('explicit absolute paths required')
    engine, source = engine.resolve(), source.resolve()
    if engine == source or source in engine.parents or engine in source.parents:
        raise ValueError('engine and private source must be independent')
    # A healthy CLI may check a different candidate before installation/update.
    # Validate the candidate's actual entrypoints, not just its descriptor.
    entrypoint = bounded(engine, 'dot')
    bounded(engine, 'contract.py')
    if not os.access(entrypoint, os.X_OK):
        raise ValueError('engine CLI is not executable')
    public = read_json(bounded(engine, 'engine.json'))
    private = read_json(bounded(source, 'dot-contract.json'))
    public_fields = {'protocol', 'engine_api', 'manifest_version', 'approval_version', 'adapters'}
    object_fields(public, public_fields | (set(public) & {'manifest_versions', 'approval_versions'}))
    object_fields(private, ('protocol', 'engine_api', 'manifest_version',
                            'approval_version', 'manifest', 'home_source',
                            'required_adapters'))
    for data in (public, private):
        for key, expected in (('protocol', 1),):
            revision(data[key], expected)
    revision(public['manifest_version'], 1)
    revision(public['approval_version'], 2)
    approvals = public.get('approval_versions', [2])
    if approvals not in ([2], [2, 3]) or any(type(v) is not int for v in approvals):
        raise ValueError('unsupported approval capability')
    if type(private['approval_version']) is not int or private['approval_version'] not in approvals:
        raise ValueError('incompatible approval version')
    versions = public.get('manifest_versions', [1])
    if versions not in ([1], [1, 2]) or any(type(v) is not int for v in versions):
        raise ValueError('unsupported manifest capability')
    if type(private['manifest_version']) is not int or private['manifest_version'] not in versions:
        raise ValueError('incompatible manifest version')
    if (private['manifest_version'], private['approval_version']) not in ((1, 2), (2, 3)):
        raise ValueError('incompatible manifest/approval pair')
    revision(public['engine_api'], 1)
    interval = private['engine_api']
    object_fields(interval, ('min', 'max'))
    if (any(type(interval[key]) is not int for key in ('min', 'max')) or
            not 1 <= interval['min'] <= public['engine_api'] <= interval['max']):
        raise ValueError('incompatible engine API')
    required = private['required_adapters']
    names = {'intent', 'scope', 'gitconfig', 'skills', 'layout'}
    if (not isinstance(required, list) or
            any(not isinstance(name, str) for name in required) or
            len(set(required)) != len(required) or set(required) != names):
        raise ValueError('unsupported adapters')
    object_fields(public['adapters'], names)
    for name in required:
        bounded(engine, public['adapters'][name])
    if 2 in versions or 3 in approvals:
        for module in ('tracking.py', 'file_metadata.py'):
            bounded(engine, module)
    if private['home_source'] != 'home':
        raise ValueError('unsupported home source boundary')
    bounded(source, 'home', directory=True)
    manifest_path = bounded(source, private['manifest'])
    if manifest_path == source / 'home' or source / 'home' in manifest_path.parents:
        raise ValueError('manifest must be outside managed home source')
    catalog = read_json(manifest_path)
    fields = {'version', 'entries', 'profiles'} | ({'machines', 'tracking'} if private['manifest_version'] == 2 else set())
    object_fields(catalog, fields)
    revision(catalog['version'], private['manifest_version'])
    if not isinstance(catalog['entries'], dict) or not isinstance(catalog['profiles'], dict):
        raise ValueError('invalid manifest structure')
    return {'protocol': public['protocol'], 'engine_api': public['engine_api'],
            'manifest_version': private['manifest_version'],
            'approval_version': private['approval_version'],
            'engine': str(engine), 'source': str(source),
            'home_source': str(source / private['home_source']),
            'manifest': str(source / private['manifest'])}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['check'])
    parser.add_argument('--engine', required=True, type=Path)
    parser.add_argument('--source', required=True, type=Path)
    args = parser.parse_args()
    try:
        result = check(args.engine, args.source)
    except (OSError, ValueError, KeyError, TypeError):
        print('dot: incompatible or missing engine/config contract; verify explicit '
              '--engine and --source paths and install compatible versions; '
              'no configuration was applied', file=sys.stderr)
        return 1
    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == '__main__':
    sys.exit(main())
