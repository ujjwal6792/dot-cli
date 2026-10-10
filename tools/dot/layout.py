#!/usr/bin/env python3
"""Approved, recoverable sourceDir migration; never applies target files."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import sys
import tomllib
sys.dont_write_bytecode = True
import intent
import scope


def private_path(home, path):
    path = Path(os.path.abspath(path))
    if not path.is_relative_to(home) or path.resolve() != home.resolve() / path.relative_to(home):
        raise ValueError('migration metadata must be local without symlink traversal')
    return path


def proposed(root, home, config):
    config = private_path(home, config)
    selection = scope.load(str(home))
    if selection is None:
        raise ValueError('source migration requires an explicit existing adoption boundary')
    journal = private_path(home, home / '.local/state/dot/layout-migration.json')
    if journal.exists() and json.loads(journal.read_text()).get('stage') != 'complete':
        raise ValueError('incomplete source migration; inspect journal/backups; no unsafe retry')
    new_source = root / 'home'
    if new_source.resolve() != root.resolve() / 'home' or not (new_source / '.chezmoiignore').is_file():
        raise ValueError('dedicated home/ source missing or redirected')
    raw = config.read_bytes().decode('utf-8')
    data = tomllib.loads(raw)
    if data.get('hooks'):
        raise ValueError('source migration refuses configuration hooks; review without execution')
    if data.get('sourceDir') not in (str(root), str(new_source)):
        raise ValueError('sourceDir ownership conflicts; left untouched')
    if data.get('sourceDir') == str(new_source):
        return {'version': 1, 'complete': True}
    header = re.split(r'(?m)^\s*\[', raw, maxsplit=1)[0]
    pattern = r'(?m)^(sourceDir\s*=\s*)("[^"\r\n]*"|\x27[^\x27\r\n]*\x27)([ \t]*(?:#.*)?)(\r?\n|$)'
    matches = list(re.finditer(pattern, header))
    if len(matches) != 1:
        raise ValueError('unsupported sourceDir syntax; left untouched')
    result = {'version': 1, 'root': str(root), 'home': str(home), 'config': str(config),
              'from': str(root), 'to': str(new_source), 'config_hash': hashlib.sha256(config.read_bytes()).hexdigest(),
              'scope_hash': hashlib.sha256(scope.profile_path(str(home)).read_bytes()).hexdigest(),
              'source_hash': hashlib.sha256((new_source / '.chezmoiignore').read_bytes()).hexdigest()}
    result['id'] = intent.digest(result)
    return result


def display(value):
    if value.get('complete'):
        print('Source layout already migrated; selection unchanged.')
    else:
        print('Source layout migration (no configuration adoption):')
        print(value['from'] + ' → ' + value['to'])
        print('Approved selection, cache/state, identity and unrelated settings remain unchanged.')
        print('Plan: ' + value['id'])


def approved(root, home, config, value, token):
    if value.get('complete'):
        return
    if token != value.get('id') or proposed(root, home, config) != value:
        raise ValueError('stale/unapproved source migration; review a fresh plan')
    directory = private_path(home, home / '.local/state/dot/layout-recovery' / token)
    directory.mkdir(parents=True, mode=0o700, exist_ok=False)
    directory.chmod(0o700)
    for name, path in (('config', config), ('selection', scope.profile_path(str(home)))):
        backup = directory / name
        backup.write_bytes(path.read_bytes())
        backup.chmod(0o600)
        if backup.read_bytes() != path.read_bytes():
            raise ValueError('metadata changed during backup; no migration applied')
    journal = private_path(home, home / '.local/state/dot/layout-migration.json')
    progress = {'version': 1, 'stage': 'backed-up', 'backup': str(directory), 'plan': value}
    intent.write(journal, json.dumps(progress, indent=2) + '\n')
    try:
        if proposed_config_hash(config) != value['config_hash']:
            raise ValueError('configuration changed after backup')
        raw = config.read_bytes().decode('utf-8')
        pattern = r'(?m)^(sourceDir\s*=\s*)("[^"\r\n]*"|\x27[^\x27\r\n]*\x27)([ \t]*(?:#.*)?)(\r?\n|$)'
        raw = re.sub(pattern, lambda m: m[1] + json.dumps(value['to']) + m[3] + m[4], raw, count=1)
        mode = config.stat().st_mode & 0o777
        intent.write(config, raw)
        config.chmod(mode)
        if scope.profile_path(str(home)).read_bytes() != (directory / 'selection').read_bytes():
            raise ValueError('selection changed; inspect recovery before retry')
        progress['stage'] = 'complete'
        intent.write(journal, json.dumps(progress, indent=2) + '\n')
    except (OSError, ValueError) as error:
        raise ValueError(f'incomplete source migration; recovery: {directory}; {error}') from error
    print('Source layout migrated without adoption; recovery: ' + str(directory))


def proposed_config_hash(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('command', choices=('plan', 'apply'))
    parser.add_argument('file', nargs='?')
    parser.add_argument('--out')
    parser.add_argument('--approve')
    args = parser.parse_args()
    root = Path(os.environ['DOT_SOURCE'])
    home = Path(os.environ['HOME'])
    config = Path(os.environ.get('DOT_CONFIG', str(Path(os.environ.get('XDG_CONFIG_HOME', str(home / '.config'))) / 'chezmoi/chezmoi.toml')))
    if args.command == 'plan':
        value = proposed(root, home, config)
        display(value)
        if args.out:
            path = Path(args.out)
            if path.resolve().is_relative_to(root.resolve()) or path.is_symlink() or path.exists():
                raise ValueError('migration plan output must be new and outside source checkout')
            intent.write(path, json.dumps(value, indent=2) + '\n')
    else:
        if not args.file:
            raise ValueError('saved migration plan required')
        value = json.loads(Path(args.file).read_text())
        approved(root, home, config, value, args.approve)


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError) as error:
        print('dot layout: ' + str(error), file=sys.stderr)
        sys.exit(1)
