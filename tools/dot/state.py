#!/usr/bin/env python3
"""Filesystem validation and metadata for the dot process boundary."""
import fnmatch
import os
from pathlib import Path
import re
import sys
import tomllib
from scope import allows, load, target_name, source_root


def target(raw):
    home = Path(os.path.abspath(os.environ['HOME']))
    path = Path(os.path.abspath(os.path.expanduser(raw)))
    relative = path.relative_to(home)
    if not relative.parts or path.resolve() != home.resolve() / relative:
        raise ValueError('target must be inside HOME without symlink traversal')
    return path, relative.as_posix()


def validate(raw):
    path, relative = target(raw)
    if relative == '.local/state/dot' or relative.startswith('.local/state/dot/'):
        raise ValueError('machine-local dot state rejected')
    forbidden = ('auth.json', 'service.json', '*keypair*', '*history*', '*sessions*',
                 '*session_index*', '*.sqlite*', '*.db*', 'node_modules', 'secrets.env',
                 '.zshenv', 'cli-client-id', 'server-id', '.skill-lock.json',
                 'id_rsa*', 'id_ed25519*', '*.pem', '.git', '.aws', '.ssh',
                 '.gnupg', '.netrc', '.npmrc', '*credentials*', '*token*', '.env', '.env.*')
    entries = [path, *path.rglob('*')] if path.is_dir() else [path]
    for entry in entries:
        _, rel = target(str(entry))
        if any(fnmatch.fnmatch(part, pattern) for part in Path(rel).parts for pattern in forbidden):
            raise ValueError(f'local-unmanaged path rejected: {rel}')
    print(relative)


def taxonomy(root, relative, category=None, os_name='-'):
    file = root / 'tools/dot/taxonomy.tsv'
    rows = file.read_text().splitlines() if file.exists() else []
    rows = [row for row in rows if row.split('\t')[0] != relative]
    if category:
        rows.append('\t'.join((relative, 'managed-' + category, os_name)))
    file.parent.mkdir(parents=True, exist_ok=True)
    file.write_text(''.join(row + '\n' for row in sorted(rows)))
    ignore = source_root(root) / '.chezmoiignore'
    text = ignore.read_text() if ignore.exists() else ''
    marker = f'# dot os-only: {relative}'
    text = re.sub(re.escape(marker) + r'\n.*?# dot end\n?', '', text, flags=re.S)
    if category == 'os-only':
        text += f'\n{marker}\n{{{{ if ne .chezmoi.os "{os_name}" -}}}}\n{relative}\n{relative}/**\n{{{{ end -}}}}\n# dot end\n'
    ignore.write_text(text)


def secrets(root, env_file):
    # Values are parsed as data, never sourced or printed. Ambient env does not count.
    values = {}
    if env_file.exists():
        for line in env_file.read_text().splitlines():
            match = re.match(r'^\s*(?:export\s+)?([A-Z][A-Z0-9_]*)\s*=\s*(.*?)\s*$', line)
            if match:
                values[match[1]] = match[2].strip('\"\'')
    keys = set()
    selection = load(os.environ['HOME'])
    source_dir = source_root(root)
    for path in source_dir.rglob('*'):
        relative = path.relative_to(source_dir)
        if any(part in ('.git', 'tools', 'tests', 'docs', 'packages', 'scripts') for part in relative.parts):
            continue
        # Skills are instructional content, not runtime config references.
        if relative.parts[:2] == ('dot_agents', 'skills') or path.suffix == '.md':
            continue
        if not path.is_file() or path.is_symlink():
            continue
        if not allows(selection, target_name(relative)):
            continue
        text = path.read_text(errors='replace')
        keys.update(re.findall(r'(?:\$\{?|\benv[:.])([A-Z][A-Z0-9_]*(?:KEY|TOKEN|SECRET|PASSWORD)[A-Z0-9_]*)', text))
        keys.update(re.findall(r'\{env:([A-Z][A-Z0-9_]*)\}', text))
        # `env` is also used for non-secret runtime paths (e.g. DOT_ENGINE).
        # Treat only credential-shaped names as references; arbitrary env
        # lookups must not produce false secret requirements.
        keys.update(re.findall(r'\benv\s+"([A-Z][A-Z0-9_]*(?:KEY|TOKEN|SECRET|PASSWORD|CREDENTIAL)[A-Z0-9_]*)"', text))
    missing = sorted(key for key in keys if not values.get(key))
    for key in missing:
        print(f'secrets: MISSING {key}', file=sys.stderr)
    if missing:
        raise ValueError('referenced secrets are absent or empty in secrets.env')
    print('secrets: all referenced keys present')


def update(file, name, version):
    text = file.read_text()
    data = tomllib.loads(text)
    old = data['pins'][name]['version']
    if not re.fullmatch(r'\d+\.\d+\.\d+|\d+\.x', version):
        raise ValueError('version must be X.Y.Z or MAJOR.x')
    section = re.compile(r'(^\[pins\.' + re.escape(name) + r'\]\s*\n)(.*?)(?=^\[|\Z)', re.M | re.S)
    def replace(match):
        body = re.sub(r'^version\s*=\s*"[^"]*"', f'version = "{version}"', match[2], count=1, flags=re.M)
        # Keep version-bearing install commands in step; unversioned routes stay explicit.
        body = re.sub(r'^(install_\w+\s*=.*)$', lambda m: m[0].replace(old, version).replace('install ' + old.removesuffix('.x'), 'install ' + version.removesuffix('.x')) if old.endswith('.x') else m[0].replace(old, version), body, flags=re.M)
        return match[1] + body
    updated = section.sub(replace, text, count=1)
    tomllib.loads(updated)
    file.write_text(updated)


try:
    command, *args = sys.argv[1:]
    if command == 'validate':
        validate(args[0])
    elif command == 'relative':
        print(target(args[0])[1])
    elif command == 'taxonomy':
        taxonomy(Path(args[0]), *args[1:])
    elif command == 'secrets':
        secrets(Path(args[0]), Path(args[1]))
    elif command == 'update':
        update(Path(args[0]), *args[1:])
    else:
        raise ValueError('unknown operation')
except (ValueError, KeyError, OSError) as error:
    print(f'dot: {error}', file=sys.stderr)
    sys.exit(1)
