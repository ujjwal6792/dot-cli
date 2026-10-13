#!/usr/bin/env python3
"""Persistent target-local adoption boundary; never writes source state."""
import json
import difflib
import os
from pathlib import Path
import re
import sys
import subprocess
import tempfile
import state_paths
sys.dont_write_bytecode = True


def source_root(root):
    root = Path(root)
    return root / 'home' if (root / 'home').is_dir() else root


def profile_path(home):
    return state_paths.state_dir(home) / 'adoption.json'


def load(home):
    path = profile_path(home)
    if path.resolve() != state_paths.state_dir(home).resolve() / 'adoption.json':
        raise ValueError('adoption profile must not traverse symlinks')
    if path.is_symlink():
        raise ValueError('adoption profile must not be a symlink')
    if not path.exists():
        return None
    from contract import read_json
    data = read_json(path)
    if not isinstance(data, dict) or type(data.get('version')) is not int:
        raise ValueError('invalid adoption profile version')
    if data.get('version') in (2, 3):
        from intent import metadata, profile_name
        fields = {'version', 'profile', 'entries'} | ({'machine'} if data['version'] == 3 else set())
        if set(data) != fields or not isinstance(data['entries'], dict):
            raise ValueError('invalid exact adoption profile')
        if data['version'] == 3:
            from file_metadata import machine_identifier, SYSTEMS
            if not isinstance(data['machine'], dict) or set(data['machine']) != {'id', 'os'} or data['machine']['os'] not in SYSTEMS:
                raise ValueError('invalid approved machine identity')
            machine_identifier(data['machine']['id'])
        profile_name(data['profile'])
        for key, entry in data['entries'].items():
            if not isinstance(key, str):
                raise ValueError('invalid entry identifier')
            metadata(entry)
        return data
    if data.get('version') != 1 or not isinstance(data.get('groups'), dict):
        raise ValueError('invalid adoption profile')
    for roots in data['groups'].values():
        if not isinstance(roots, list) or any(not isinstance(p, str) or not p or
                p.startswith('/') or '..' in Path(p).parts or
                re.search(r'[\n\r\t*?\[\]#!{}]', p) for p in roots):
            raise ValueError('invalid adoption roots')
    return data


def save(home, data):
    path = profile_path(home)
    if path.resolve() != state_paths.state_dir(home).resolve() / 'adoption.json':
        raise ValueError('local profile must not traverse symlinks')
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=path.parent)
    try:
        with os.fdopen(fd, 'w') as stream:
            json.dump(data, stream, indent=2)
            stream.write('\n')
        os.replace(tmp, path)
    finally:
        if os.path.exists(tmp):
            os.unlink(tmp)


def allows(data, path):
    if data is not None and data['version'] in (2, 3):
        return any(path == entry['destination'] for entry in data['entries'].values())
    return data is None or any(path == root or path.startswith(root + '/')
                              for roots in data['groups'].values() for root in roots)


def attributes(part):
    found = []
    while True:
        match = re.match(r'^(private|readonly|exact|executable|symlink|create|modify|remove|run|once|onchange|before|after|encrypted|empty|dot|literal)_(.*)$', part)
        if not match:
            break
        attribute, part = match.groups()
        found.append(attribute)
        if attribute == 'literal':
            break
    return found, part


def target_name(source, directory=False):
    # Source attributes from chezmoi's source-state naming convention.
    parts = []
    source_parts = Path(source).parts
    for index, part in enumerate(source_parts):
        attrs, part = attributes(part)
        if index == len(source_parts) - 1 and not directory:
            if part.endswith('.literal'):
                part = part[:-8]
            elif part.endswith('.tmpl'):
                part = part[:-5]
        parts.append(('.' if 'dot' in attrs else '') + part)
    return '/'.join(parts)


def templated(source):
    attrs, name = attributes(Path(source).name)
    return name.endswith('.tmpl')


def skills_allowed(data):
    if data is not None and data['version'] in (2, 3):
        return any('gemini-skills' in e.get('integrations', []) for e in data['entries'].values())
    return data is None or all(p in data['groups'].get('skills', []) for p in
                              ('.agents/skills', 'sync-skills.sh', '.gemini/skills'))


def is_hook(path):
    return 'run' in attributes(Path(path).name)[0]


def check_destructive_attributes(source):
    attrs = attributes(source.name)[0]
    if 'exact' in attrs and source.is_dir():
        raise ValueError('exact directory cleanup is not supported in selective scope')
    if 'remove' in attrs:
        raise ValueError('native removal entries need explicit bounded removal directives')


def hook_allowed(data, relative):
    if data is not None and data['version'] in (2, 3):
        return str(relative) == 'run_after_sync-skills.sh.tmpl' and skills_allowed(data)
    return data is None or (str(relative) == 'run_after_sync-skills.sh.tmpl' and skills_allowed(data))


def render(root, home, source, config=None, os_name=None):
    cmd = ['chezmoi', '--source', str(source_root(root)), '--destination', home]
    config = config or os.environ.get('DOT_CONFIG')
    if config:
        cmd += ['--config', config]
    if os_name:
        cmd += ['--override-data', json.dumps({'chezmoi': {'os': os_name, 'homeDir': home}})]
    result = subprocess.run(cmd + ['execute-template', '--file', str(source)],
                            capture_output=True, text=True, check=False)
    if result.returncode:
        raise ValueError('preview rendering failed: ' + result.stderr.strip())
    return result.stdout


def removal_targets(text):
    for line in text.splitlines():
        name = line.split('#', 1)[0].strip().rstrip('/')
        if name:
            yield name


def validate_removal(data, home, name):
    if re.search(r'[\n\r*?\[\]{}!]', name):
        raise ValueError('unscoped wildcard removal directive')
    if Path(name).is_absolute() or '..' in Path(name).parts:
        raise ValueError('removal directive must be target-relative')
    intentional_silo = name in ('.config/opencode/skills', '.codex/skills') and skills_allowed(data)
    if allows(data, name) and (Path(home) / name).is_dir() and not intentional_silo:
        raise ValueError('directory-wide removal needs a separately approved integration')


def native_directory_mask(value):
    if os.environ.get('CHEZMOI_UMASK') or value == 'unsupported':
        raise ValueError('unsupported native umask override; ancestor permissions cannot be validated')
    if value is None or str(value) == '-1':
        mask = os.umask(0)
        os.umask(mask)
        return mask
    mask = int(value)
    if not 0 <= mask <= 0o777:
        raise ValueError('invalid native umask')
    return mask


def preserve_directory(home, name, source, mask):
    from intent import bounded
    target = bounded(home, name)
    if not target.exists():
        return  # Creating the necessary parent is not permission to chmod an existing one.
    if not target.is_dir():
        raise ValueError('ancestor is not a directory: ' + name)
    attrs = attributes(source.name)[0]
    desired = (0o700 if 'private' in attrs else 0o777) & ~mask
    if 'readonly' in attrs:
        desired &= ~0o222
    current = target.stat().st_mode & 0o7777
    if current != desired:
        raise ValueError(f'unapproved ancestor permission change: {name} {current:04o} → {desired:04o}; '
                         'existing directory permissions preserved; separate directory selection/preview/approval required')


def ignore(root, home, config=None, os_name=None, umask_value=None, permissions='check', native_args='[]'):
    from gitconfig import native_operation
    operation = native_operation(json.loads(native_args))
    if not operation['mutating'] and (operation['command'] in ('status', 'diff', 'managed', 'source-path') or operation['preview']):
        permissions = 'preview'
    if operation['umask_override']:
        umask_value = 'unsupported'
    source_dir = Path(root)
    repository = source_dir.parent if source_dir.name == 'home' else source_dir
    data = load(home)
    if data is None:
        if source_dir.name != 'home':
            return
        data = {'version': 2, 'profile': 'unapproved', 'entries': {}}
    # Deny individual unselected entries, including future/unknown groups and
    # scripts. Keep only ancestors needed to reach selected roots.
    if data['version'] in (2, 3):
        from intent import bounded, file_state, manifest, read_text, template_text
        catalog = manifest(repository)
        from file_metadata import entries as resolve_entries, applicable, check_scope
        check_scope(catalog, Path(home), data)
        resolved = resolve_entries(catalog, Path(home))
        from tracking import pending
        pending(Path(home))
        native_commands = json.loads(native_args)
        mask = native_directory_mask(umask_value)
        for key, entry in data['entries'].items():
            current_os = os_name or ('linux' if sys.platform.startswith('linux') else sys.platform)
            if current_os not in entry['os']:
                raise ValueError('approved entry is not applicable to this OS')
            if resolved.get(key) != entry or not applicable(catalog, Path(home), key):
                raise ValueError('approved metadata changed; review a fresh adoption plan')
            source = bounded(repository, entry['source'])
            if 'git-config' in entry.get('integrations', []):
                from gitconfig import native_validate
                native_validate(repository, Path(home), entry, config, current_os, native_commands, mask)
                continue
            text = read_text(source)
            if templated(entry['source']):
                template_text(text)
            live = bounded(home, entry['destination'])
            if live.exists() and not templated(entry['source']):
                read_text(live)
            from skills import output as skills_output
            generated = skills_output(entry)
            if generated:
                target = bounded(home, generated)
                if file_state(target):
                    read_text(target)
        roots = [entry['destination'] for entry in data['entries'].values()]
        approved_sources = {entry['source'].removeprefix('home/') for entry in data['entries'].values()}
        from gitconfig import native_backup
        native_backup(repository, Path(home), config, os_name, native_commands)
    else:
        roots = [p for paths in data['groups'].values() for p in paths]
        from intent import manifest
        if allows(data, '.gitconfig') and any('git-config' in e.get('integrations', []) for e in manifest(repository)['entries'].values()):
            from gitconfig import approved, native_validate, native_backup
            current_os = os_name or ('linux' if sys.platform.startswith('linux') else sys.platform)
            native_commands = json.loads(native_args)
            entry = approved(repository, Path(home))
            native_validate(repository, Path(home), entry, config, current_os, native_commands, native_directory_mask(umask_value))
            native_backup(repository, Path(home), config, current_os, native_commands)
    mask = native_directory_mask(umask_value)
    for source in sorted(source_dir.rglob('*')):
        rel = source.relative_to(source_dir)
        if any(p.startswith('.chezmoi') or p == '.git' for p in rel.parts):
            continue
        name = target_name(rel, source.is_dir())
        permitted = allows(data, name)
        if data['version'] in (2, 3):
            permitted = str(rel) in approved_sources
        if is_hook(rel):
            permitted = hook_allowed(data, rel)
        if source.is_dir() and not source.is_symlink():
            permitted = permitted or any(p.startswith(name + '/') for p in roots)
        if permitted:
            if data['version'] in (2, 3) and is_hook(rel):
                from skills import supported_hook
                if source.is_symlink() or not supported_hook(source.read_text()):
                    raise ValueError('unsupported native skills hook; only the bounded adapter is authorized')
            check_destructive_attributes(source)
            if permissions != 'preview' and source.is_dir() and not source.is_symlink():
                preserve_directory(home, name, source, mask)
        if not permitted:
            if re.search(r'[\n\r*?\[\]#!{}]', name):
                raise ValueError('unsupported source name in selective profile')
            print(name)
    # Removal-only targets need exclusions even without corresponding source
    # entries. Render as data: execute-template does not run apply scripts.
    removals = source_dir / '.chezmoiremove'
    if removals.exists():
        for name in removal_targets(render(root, home, removals, config, os_name)):
            validate_removal(data, home, name)
            permitted = allows(data, name)
            if data['version'] in (2, 3):
                permitted = any(name in entry['removals'] for entry in data['entries'].values())
                if permitted:
                    target = bounded(home, name)
                    if target.exists():
                        read_text(target)
            if not permitted:
                print(name)


def preview(root, home, data, groups):
    """Render proposed files without changing the profile or executing hooks."""
    proposed = {'version': 1, 'groups': {g: data['groups'][g] for g in groups}}
    source_dir = source_root(root)
    for source in sorted(source_dir.rglob('*')):
        rel = source.relative_to(source_dir)
        if not source.is_file() or any(p in ('.git', 'tools', 'tests', 'docs', 'scripts', 'packages') for p in rel.parts):
            continue
        name = target_name(rel, source.is_dir())
        if not allows(proposed, name):
            continue
        if is_hook(rel) and not hook_allowed(proposed, rel):
            continue
        check_destructive_attributes(source)
        live = Path(home) / name
        print(f'target: {name} ({"replace" if live.exists() else "create"})')
        if rel.name.startswith('run_'):
            print('apply hook: ' + str(rel))
        if rel.name.startswith('symlink_'):
            print('symlink target: ' + source.read_text())
            continue
        if source.suffix == '.tmpl':
            text = render(root, home, source)
        else:
            try:
                text = source.read_text()
            except UnicodeDecodeError:
                print('binary replacement')
                continue
        old = live.read_text(errors='replace') if live.is_file() else ''
        print(''.join(difflib.unified_diff(old.splitlines(True), text.splitlines(True),
                                         fromfile=str(live), tofile=str(source))), end='')
    removals = source_dir / '.chezmoiremove'
    if removals.exists():
        # Render proposed selection in a disposable home, never temporarily
        # widen the real profile. Config/identity still come from the real HOME.
        with tempfile.TemporaryDirectory(prefix='dot-scope-preview-') as temporary:
            save(temporary, proposed)
            os_name = 'linux' if sys.platform.startswith('linux') else sys.platform
            text = render(root, temporary, removals, os_name=os_name)
        for name in removal_targets(text):
            validate_removal(proposed, home, name)
            if allows(proposed, name):
                print('remove: ' + name)
    if skills_allowed(proposed):
        print('router replaces .gemini/skills')


def main():
    command, root, home, *args = sys.argv[1:]
    repository = Path(root).parent if Path(root).name == 'home' else Path(root)
    if os.environ.get('DOT_ENGINE') or (repository / 'dot-contract.json').exists():
        import intent
        from contract import check
        negotiated = check(Path(os.environ.get('DOT_ENGINE', str(Path(__file__).resolve().parent))), repository)
        if Path(__file__).resolve().parent != Path(negotiated['engine']):
            raise ValueError('scope must execute from the negotiated independent engine')
        intent.MANIFEST_PATH = negotiated['manifest']
        intent.MANIFEST_RELATIVE = Path(negotiated['manifest']).relative_to(repository.resolve()).as_posix()
    data = load(home)
    if data is None and source_root(root) != Path(root) and command in ('allowed', 'skills', 'show'):
        data = {'version': 2, 'profile': 'unapproved', 'entries': {}}
    if command == 'exact-removals':
        if data is None or data['version'] not in (2, 3):
            return
        from intent import manifest, bounded, file_state, read_text, write
        catalog = manifest(Path(root))
        from file_metadata import entries as resolve_entries, applicable, check_scope
        check_scope(catalog, Path(home), data)
        resolved = resolve_entries(catalog, Path(home))
        from tracking import pending
        pending(Path(home))
        names = set()
        for key, entry in data['entries'].items():
            if resolved.get(key) != entry or not applicable(catalog, Path(home), key):
                raise ValueError('scope/capability/impact changed; fresh adoption approval required')
            names.update(entry['removals'])
        targets = []
        for name in sorted(names):
            if any(name == e['destination'] for e in catalog['entries'].values()):
                raise ValueError('removal conflicts with managed destination')
            target = bounded(home, name)
            status = file_state(target)
            if status:
                read_text(target)
            targets.append((name, target, status))
        native_args = json.loads(args[0]) if args else []
        from gitconfig import native_operation
        if targets and native_operation(native_args)['mutating']:
            recovery = state_paths.state_dir(home) / 'native-removal-recovery'
            if recovery.resolve() != state_paths.state_dir(home).resolve() / 'native-removal-recovery':
                raise ValueError('native removal recovery must not traverse symlinks')
            recovery.mkdir(parents=True, exist_ok=True, mode=0o700)
            recovery.chmod(0o700)
            backup = Path(tempfile.mkdtemp(dir=recovery))
            records = []
            for index, (name, target, status) in enumerate(targets):
                from skills import skill_file
                if status and not skill_file(name):
                    snapshot = backup / str(index)
                    snapshot.write_bytes(target.read_bytes())
                    snapshot.chmod(0o600)
                    if file_state(target) != status or snapshot.read_bytes() != target.read_bytes():
                        raise ValueError('native removal target changed during backup')
                records.append({'path': name, 'before': status, 'backup': str(index) if status and not skill_file(name) else None})
            write(backup / 'plan.json', json.dumps(records, indent=2) + '\n')
        print('\n'.join(name for name, _, _ in targets))
    elif command == 'ignore':
        ignore(root, home, *args)
    elif command == 'allowed':
        if not allows(data, args[0]):
            raise ValueError('outside adoption scope; use dot scope expand with preview first')
    elif command == 'skills':
        print('yes' if skills_allowed(data) else 'no')
    elif command == 'git-preservation':
        from gitconfig import enabled
        print('yes' if enabled(Path(root), Path(home)) else 'no')
    elif command == 'version':
        print(data['version'] if data is not None else 0)
    elif command == 'init':
        if data is not None:
            raise ValueError('adoption profile already exists')
        save(home, {'version': 1, 'groups': {}})
    elif command == 'show':
        print(json.dumps(data) if data is not None else 'legacy unscoped')
    elif command in ('catalog', 'expand'):
        catalog = json.loads((Path(root) / 'tools/dot/groups.json').read_text())
        if command == 'catalog':
            print(json.dumps(catalog, indent=2))
            return
        if data is None:
            raise ValueError('establish an empty boundary with dot scope init first')
        groups = [arg for arg in args if not arg.startswith('--')]
        if not groups or any(arg not in ('--yes', '--ack-unsafe', '--ack-desktop') for arg in args if arg.startswith('--')):
            raise ValueError('usage: dot scope expand GROUP... [--yes] [--ack-unsafe] [--ack-desktop]')
        os_name = sys.platform
        if os_name.startswith('linux'):
            os_name = 'linux'
        for group in groups:
            entry = catalog[group]
            if os_name not in entry['os']:
                raise ValueError(f'{group} is not applicable to {os_name}')
            print(f"{group}: {entry['risk']} — {', '.join(entry['paths'])}")
            if '--yes' in args:
                if entry['risk'] == 'Unsafe' and '--ack-unsafe' not in args:
                    raise ValueError('Unsafe groups require --ack-unsafe')
                if group == 'desktop' and '--ack-desktop' not in args:
                    raise ValueError('Arch/CachyOS-targeted desktop; other Linux distributions require compatibility review; pass --ack-desktop')
            data['groups'][group] = entry['paths']
        preview(root, home, data, groups)
        if '--yes' in args:
            save(home, data)
        else:
            print('Preview only; rerun with --yes to persist (does not apply).')
    else:
        raise ValueError('unknown scope operation')


if __name__ == '__main__':
    try:
        main()
    except (ValueError, KeyError, OSError) as error:
        print(f'dot scope: {error}', file=sys.stderr)
        sys.exit(1)
