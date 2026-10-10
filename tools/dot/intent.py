#!/usr/bin/env python3
"""Reviewable Track/Adopt intent. Plans are data, never executable programs."""
import argparse
import contextlib
import difflib
import fnmatch
import hashlib
import json
import io
import os
from pathlib import Path
import re
import subprocess
import sys

import scope

MANIFEST = 'tools/dot/manifest.json'
MANIFEST_PATH = None
MANIFEST_RELATIVE = MANIFEST
EXCLUDED = ('auth.json', 'service.json', '*keypair*', '*history*', '*session*',
            '*.sqlite*', '*.db*', '*.log', 'logs', 'cache', '.cache', 'node_modules',
            'secrets.env', 'cli-client-id', 'server-id', '.skill-lock.json',
            'id_rsa*', 'id_ed25519*', '*.pem', '.git', '.aws', '.ssh', '.gnupg',
            '.netrc', '.npmrc', '*credentials*', '*token*', '.env', '.env.*',
            '.zshenv', 'runtime', 'state', '*.pid', '*.sock', '*.socket', '*.lock')


def template_text(text):
    scan(text)
    # Preview may not execute commands or read arbitrary files. Unknown template
    # functions fail closed rather than acquiring privileges before approval.
    functions = {'if', 'else', 'end', 'range', 'with', 'eq', 'ne', 'lt', 'le', 'gt', 'ge',
                 'and', 'or', 'not', 'printf', 'print', 'println', 'joinPath', 'env',
                   'len', 'index', 'true', 'false', 'nil', 'quote', 'trim', 'default', 'toJson', 'replace'}
    for action in re.findall(r'\{\{(.*?)\}\}', text, re.S):
        action = re.sub(r'"(?:\\.|[^"\\])*"|`[^`]*`', '', action)
        for name in re.findall(r'(?<![\w.$])[A-Za-z_]\w*', action):
            if name not in functions:
                raise ValueError('unsupported/nondeclarative template function: ' + name)
    return text


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True).encode()).hexdigest()


def relative(value):
    if (not isinstance(value, str) or not value or
            Path(value).as_posix() != value or value.startswith('/') or
            any(p in ('', '.', '..') for p in value.split('/')) or
            re.search(r'[\x00-\x1f\x7f*?\[\]#!{}\\]', value)):
        raise ValueError('invalid exact relative path')
    if value == '.local/state/dot' or value.startswith('.local/state/'):
        raise ValueError('local-unmanaged state rejected')
    if value == '.agents/skills.bak' or value.startswith('.agents/skills.bak/'):
        raise ValueError('local-unmanaged singleton skills backup rejected')
    if any(fnmatch.fnmatch(p.lower(), pattern) for p in Path(value).parts for pattern in EXCLUDED):
        raise ValueError('local-unmanaged path rejected: ' + value)
    return value


def bounded(base, value):
    path = Path(base) / relative(value)
    if path.resolve() != Path(base).resolve() / value:
        raise ValueError('symlink/path escape rejected: ' + value)
    return path


def scan(text):
    """Conservative detection, not proof of absence. Never print secret values."""
    if not isinstance(text, str) or re.search(r'[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]', text):
        raise ValueError('unsupported/binary content rejected')
    patterns = (r'-----BEGIN (?:[A-Z ]+ )?PRIVATE KEY-----',
                r'\b(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|AKIA[A-Z0-9]{16}|sk-[A-Za-z0-9_-]{20,})',
                r'(?i)\b(?:password|passwd|api[_-]?key|access[_-]?token|secret|authorization)\s*["\x27]?\s*[:=]\s*["\x27]?([^\s"\x27,;]+)',
                r'(?i)\bBearer\s+[A-Za-z0-9._-]{12,}',
                r'://[^\s/:]+:[^\s/@]+@')
    for pattern in patterns:
        for match in re.finditer(pattern, text):
            value = match.group(1) if match.lastindex else match.group()
            # Application-native environment references are data, not literal
            # credentials. Match the complete quoted header, never a prefix.
            if (match.lastindex and value == 'Bearer' and
                    re.match(r'\s+\{env:[A-Za-z_][A-Za-z0-9_]*\}["\x27](?=\s*[,}\n]|\s*$)', text[match.end():])):
                continue
            if match.lastindex and (value.startswith(('$', '{{', '{env:', 'env.')) or value in ('null', 'false', 'true')):
                continue
            raise ValueError('credential content detected; raw import rejected; review a secret-free template')
    return text


def read_text(path, secrets=True):
    try:
        text = path.read_text(encoding='utf-8')
    except UnicodeError as error:
        raise ValueError('unsupported/binary content rejected') from error
    return scan(text) if secrets else text


def metadata(entry):
    required = {'destination', 'source', 'os', 'portability', 'impact', 'removals', 'hooks', 'tools'}
    if not isinstance(entry, dict) or not required <= set(entry) or set(entry) - required - {'warnings', 'integrations', 'requires', 'notices', 'git_keys', 'git_source'}:
        raise ValueError('unsupported entry metadata')
    warnings = entry.get('warnings', [])
    if not isinstance(warnings, list) or len(set(warnings)) != len(warnings) or any(w != 'desktop-arch-compatibility' for w in warnings):
        raise ValueError('unsupported compatibility warning')
    relative(entry['destination'])
    relative(entry['source'])
    if not entry['source'].startswith('home/') or scope.target_name(entry['source'][5:]) != entry['destination']:
        raise ValueError('source/destination naming conflict')
    if any(a not in (('private', 'executable', 'dot', 'literal', 'modify') if 'git-config' in entry.get('integrations', []) else ('private', 'executable', 'dot', 'literal'))
           for p in Path(entry['source']).parts for a in scope.attributes(p)[0]):
        raise ValueError('unsupported source attributes')
    systems = entry['os']
    if (not isinstance(systems, list) or not systems or len(set(systems)) != len(systems)
            or any(p not in ('darwin', 'linux') for p in systems)):
        raise ValueError('invalid supported OS')
    if entry['portability'] not in ('managed-shared', 'managed-templated', 'managed-os-only'):
        raise ValueError('unsupported portability')
    if ((entry['portability'] == 'managed-templated' and not scope.templated(entry['source'])) or
            (entry['portability'] == 'managed-shared' and scope.templated(entry['source']))):
        raise ValueError('template attribute/portability conflict')
    if entry['portability'] == 'managed-os-only' and len(systems) != 1:
        raise ValueError('os-only requires exactly one OS')
    if entry['impact'] not in ('Safe', 'Unsafe'):
        raise ValueError('unsupported impact')
    for key in ('removals', 'hooks', 'tools'):
        if not isinstance(entry[key], list) or any(not isinstance(p, str) for p in entry[key]):
            raise ValueError('invalid capability list')
        if len(set(entry[key])) != len(entry[key]):
            raise ValueError('duplicate capability')
    # Arbitrary scripts cannot become bounded merely by declaring a path.
    if entry['hooks']:
        raise ValueError('unsupported hook capability; arbitrary scripts are not authorized')
    for field, supported in (('integrations', {'gemini-skills', 'git-config'}), ('notices', {'codex-project-trust', 'shell-replacement', 'config-only'})):
        values = entry.get(field, [])
        if not isinstance(values, list) or any(not isinstance(v, str) or v not in supported for v in values) or len(set(values)) != len(values):
            raise ValueError('unsupported ' + field)
    if 'gemini-skills' in entry.get('integrations', []) and not entry['destination'].startswith('.agents/skills/'):
        raise ValueError('Gemini integration requires an exact canonical skill file')
    if 'git-config' in entry.get('integrations', []):
        from gitconfig import validate
        validate(entry)
    elif 'git_keys' in entry or 'git_source' in entry:
        raise ValueError('Git keys/source require the bounded Git integration')
    needs = entry.get('requires', [])
    if not isinstance(needs, list) or any(not isinstance(v, str) for v in needs) or len(set(needs)) != len(needs):
        raise ValueError('invalid entry relationships')
    for key in needs:
        relative(key)
    for name in entry['removals']:
        relative(name)
    if any(not re.fullmatch(r'[A-Za-z0-9_-]+', tool) for tool in entry['tools']):
        raise ValueError('invalid advisory tool name')
    return entry


def manifest(root):
    path = Path(MANIFEST_PATH) if MANIFEST_PATH is not None else bounded(root, MANIFEST_RELATIVE)
    if MANIFEST_PATH is not None and (not path.is_absolute() or not path.is_file()):
        raise ValueError('negotiated manifest is unavailable')
    from contract import read_json
    data = read_json(path) if path.exists() else {'version': 1, 'entries': {}, 'profiles': {}}
    fields = {'version', 'entries', 'profiles'} | ({'machines', 'tracking'} if data.get('version') == 2 else set())
    if set(data) != fields or type(data['version']) is not int or data['version'] not in (1, 2):
        raise ValueError('unsupported manifest version/fields')
    if not isinstance(data['entries'], dict) or not isinstance(data['profiles'], dict):
        raise ValueError('invalid manifest')
    destinations, sources = set(), set()
    for key, entry in data['entries'].items():
        relative(key)
        metadata(entry)
        for name in destinations:
            dest = entry['destination']
            if dest == name or dest.startswith(name + '/') or name.startswith(dest + '/'):
                raise ValueError('conflicting destinations')
        if entry['source'] in sources:
            raise ValueError('conflicting source paths')
        destinations.add(entry['destination'])
        sources.add(entry['source'])
        if scope.target_name(entry['source'][5:]) != entry['destination']:
            raise ValueError('source/destination naming conflict')
        if any(a in (('run', 'remove', 'exact', 'symlink', 'encrypted') if 'git-config' in entry.get('integrations', []) else ('run', 'remove', 'exact', 'symlink', 'modify', 'encrypted'))
               for p in Path(entry['source']).parts for a in scope.attributes(p)[0]):
            raise ValueError('unsupported destructive source attributes')
    for name, ids in data['profiles'].items():
        profile_name(name)
        if not isinstance(ids, list) or len(set(ids)) != len(ids) or any(i not in data['entries'] for i in ids):
            raise ValueError('invalid named profile')
    validate_manifest_data(data)
    return data


def profile_name(value):
    if not isinstance(value, str) or not re.fullmatch(r'[A-Za-z0-9_-]+', value):
        raise ValueError('invalid profile name')
    return value


def file_state(path):
    if path.is_symlink() or (path.exists() and not path.is_file()):
        raise ValueError('only exact regular files may be replaced/removed')
    return {'sha256': hashlib.sha256(path.read_bytes()).hexdigest(), 'mode': path.stat().st_mode & 0o7777} if path.exists() else None


def source_name(destination, templated=False, executable=False, private=False):
    parts = []
    for part in Path(destination).parts:
        hidden = part.startswith('.')
        name = part[1:] if hidden else part
        if scope.attributes(name)[0]:
            name = 'literal_' + name
        parts.append(('dot_' if hidden else '') + name)
    parts[-1] = ('private_' if private else '') + ('executable_' if executable else '') + parts[-1]
    suffix = '.tmpl' if templated else '.literal' if parts[-1].endswith(('.tmpl', '.literal')) else ''
    return 'home/' + '/'.join(parts) + suffix


def snapshot(home, selection):
    path = bounded(home, selection['path'])
    if not path.exists():
        raise ValueError('selected file does not exist')
    if not path.is_dir():
        return [selection['path']]
    if selection['path'] in ('.', ''):
        raise ValueError('blanket HOME tracking rejected')
    if (path / '.git').exists() or (path / '.git').is_symlink():
        return []
    files = []
    pending = [path]
    while pending:
        directory = pending.pop()
        for entry in sorted(directory.iterdir(), reverse=True):
            name = entry.relative_to(home).as_posix()
            try:
                bounded(home, name)
            except ValueError:
                continue  # excluded ancestors are never descended or enrolled
            if entry.is_symlink():
                continue
            if entry.is_dir():
                if (entry / '.git').exists() or (entry / '.git').is_symlink():
                    continue
                pending.append(entry)
                continue
            if entry.is_file():
                try:
                    read_text(entry)
                except ValueError:
                    continue  # mixed snapshots skip ineligible files, never disclose them
                files.append(name)
    return sorted(files)


def git(root, *args):
    command = ['git', '-C', str(root), '-c', 'core.fsmonitor=false',
               '-c', 'core.hooksPath=' + os.devnull, '-c', 'commit.gpgsign=false']
    environment = dict(os.environ, GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM='1')
    for key in ('GIT_DIR', 'GIT_WORK_TREE', 'GIT_INDEX_FILE', 'GIT_OBJECT_DIRECTORY', 'GIT_ALTERNATE_OBJECT_DIRECTORIES'):
        environment.pop(key, None)
    if args[0] in ('status', 'rev-parse', 'diff'):
        # Preview must not inherit target-global includes, helpers or fsmonitor
        # commands merely to inspect the source checkout. Apply-side commits
        # still use the established identity; config parsing uses --no-includes.
        command += ['--no-optional-locks', '-c', 'core.fsmonitor=false', '-c', 'core.hooksPath=' + os.devnull]
        if args[0] == 'diff':
            args = (args[0], '--no-ext-diff', *args[1:])
    result = subprocess.run([*command, *args], capture_output=True, text=True, env=environment)
    if result.returncode:
        raise ValueError('Git operation failed: ' + result.stderr.strip())
    return result.stdout


def plan(root, home, request):
    from tracking import pending
    pending(home)
    if not isinstance(request, dict) or type(request.get('version')) is not int or request.get('version') != 1:
        raise ValueError('unsupported request version')
    mode = request.get('mode')
    if mode not in ('track', 'adopt', 'migrate', 'update', 'untrack', 'metadata'):
        raise ValueError('invalid intent mode')
    allowed = {'version', 'mode', 'profile'} | ({'selections'} if mode == 'track' else {'entries', 'metadata'} if mode == 'adopt' else {'entries'} if mode == 'untrack' else {'metadata'} if mode == 'metadata' else set())
    if set(request) - allowed:
        raise ValueError('unsupported request fields')
    name = profile_name(request.get('profile'))
    original = manifest(root)
    if original['version'] == 2:
        from file_metadata import identity
        identity(home, original)
    data = json.loads(json.dumps(original))
    actions, selected, inputs = [], {}, {}
    os_name = 'linux' if sys.platform.startswith('linux') else sys.platform
    state = scope.load(str(home))
    from file_metadata import check_scope
    # A fresh Adopt may explicitly re-identify; updates/source-only changes may not.
    if mode != 'adopt':
        check_scope(original, home, state)
    edits = request.get('metadata', {})
    if not isinstance(edits, dict) or (mode != 'metadata' and any(k not in data['entries'] for k in edits)):
        raise ValueError('invalid metadata edits')
    for key, changes in (edits.items() if mode != 'metadata' else []):
        if not isinstance(changes, dict) or set(changes) - {'os', 'impact', 'removals', 'tools', 'warnings', 'integrations', 'requires', 'notices', 'git_keys'}:
            raise ValueError('unsupported Adopt metadata edits')
        data['entries'][key] = metadata(dict(data['entries'][key], **changes))
    # Capabilities can create outputs/relationships involving unselected entries.
    # Validate the complete proposed intent, not merely each edited entry.
    validate_manifest_data(data)
    selections = request.get('selections', [])
    if not isinstance(selections, list):
        raise ValueError('invalid selections')
    scope_after = None
    if mode == 'metadata':
        from file_metadata import edit
        scope_after = edit(root, home, data, edits, state, actions, selected, inputs)
    elif mode == 'untrack':
        from tracking import untrack
        scope_after = untrack(root, home, request, data, state, actions, selected, inputs)
    elif mode == 'track':
        from gitconfig import track, track_entry
        for selection in selections:
            if isinstance(selection, dict) and 'logical' in selection:
                from file_metadata import track as track_logical
                track_logical(root, home, data, selection, selected, inputs, actions)
                continue
            if not isinstance(selection, dict) or set(selection) - {'path', 'destination', 'os', 'portability', 'impact', 'template', 'tools', 'removals', 'warnings', 'integrations', 'requires', 'notices', 'git_keys'}:
                raise ValueError('unsupported selection fields')
            dest = selection.get('destination', selection.get('path'))
            previous_id, previous = track_entry(data, dest)
            if previous and 'git-config' in previous.get('integrations', []):
                live = bounded(home, selection['path'])
                text = track(root, home, previous, selection)
                if previous_id in selected:
                    raise ValueError('duplicate selection')
                selected[previous_id] = previous
                inputs[selection['path']] = file_state(live)
                source = bounded(root, previous['git_source'])
                actions.append({'path': previous['git_source'], 'before': file_state(source), 'text': text, 'remove': False})
                continue
            if 'git_keys' in selection:
                raise ValueError('Track Git keys require an existing bounded Git entry')
            templated = 'template' in selection
            if ((selection.get('portability') == 'managed-templated' and not templated) or
                    (selection.get('portability') == 'managed-shared' and templated)):
                raise ValueError('templated imports require an explicitly reviewed secret-free template')
            paths = snapshot(home, selection)
            if 'destination' in selection and len(paths) != 1:
                raise ValueError('destination editing requires one exact file')
            for imported in paths:
                live = bounded(home, imported)
                inputs[imported] = file_state(live)
                dest = selection.get('destination', imported)
                if track_entry(data, dest)[1]:
                    raise ValueError('Track bounded Git by explicitly selecting its exact file and approved git_keys')
                try:
                    raw = read_text(live)
                except ValueError as error:
                    if not templated or not str(error).startswith('credential content detected'):
                        raise
                    raw = None  # Detection blocks raw import, never the reviewed replacement template.
                text = template_text(selection['template']) if templated else raw
                mode_bits = inputs[imported]['mode']
                entry = metadata({'destination': dest, 'source': source_name(dest, templated, bool(mode_bits & 0o111), templated or not bool(mode_bits & 0o044)),
                                  'os': selection.get('os'), 'portability': selection.get('portability'),
                                  'impact': selection.get('impact'), 'tools': selection.get('tools', []),
                                   'removals': selection.get('removals', []), 'hooks': []})
                for field in ('warnings', 'integrations', 'requires', 'notices'):
                    if field in selection:
                        entry[field] = selection[field]
                metadata(entry)
                if dest in selected:
                    raise ValueError('duplicate selection')
                selected[dest] = entry
                previous = data['entries'].get(dest)
                if previous is not None and previous['source'] != entry['source']:
                    old_source = bounded(root, previous['source'])
                    actions.append({'path': previous['source'], 'before': file_state(old_source),
                                    'text': None, 'remove': True})
                data['entries'][dest] = entry
                source = bounded(root, entry['source'])
                actions.append({'path': entry['source'], 'before': file_state(source), 'text': text, 'remove': False})
        data['profiles'][name] = sorted(selected)
        # Validate the entire proposed manifest, not only edited entries.
        validate_manifest_data(data)
        actions.append({'path': MANIFEST_RELATIVE, 'before': file_state(bounded(root, MANIFEST_RELATIVE)),
                        'text': json.dumps(data, indent=2, sort_keys=True) + '\n', 'remove': False})
        if git(root, 'status', '--porcelain'):
            raise ValueError('Track requires a clean source checkout; commit unrelated work separately')
    else:
        from file_metadata import entries as resolve_entries, applicable
        resolved = resolve_entries(data, home)
        if mode == 'update':
            if state is None or state['version'] not in (2, 3):
                raise ValueError('updates require an approved exact scope')
            ids = list(state['entries'])
            for key, entry in state['entries'].items():
                if resolved.get(key) != entry or not applicable(data, home, key):
                    raise ValueError('scope/capability/impact changed; fresh adoption approval required')
        elif mode == 'migrate':
            if state is None or state['version'] != 1:
                raise ValueError('migration requires a legacy local scope')
            if name not in data['profiles']:
                raise ValueError('migration requires an explicitly recorded repository profile')
            ids = [key for key in data['profiles'][name] if scope.allows(state, data['entries'][key]['destination'])]
        else:
            ids = request.get('entries', data['profiles'].get(name))
        if not isinstance(ids, list) or len(set(ids)) != len(ids) or any(key not in data['entries'] for key in ids):
            raise ValueError('unknown entries/profile')
        if mode == 'adopt' and 'entries' not in request and data['version'] == 2:
            ids = [key for key in ids if os_name in resolved[key]['os'] and applicable(data, home, key)]
        if set(edits) - set(ids):
            raise ValueError('Adopt metadata edits must be restricted to selected entries')
        for key in ids:
            entry = resolved[key]
            if not applicable(data, home, key):
                raise ValueError('entry is not applicable to this explicit machine')
            if os_name not in entry['os']:
                raise ValueError('entry is not applicable to this OS')
            selected[key] = entry
            if set(entry.get('requires', [])) - set(ids):
                raise ValueError('required entries must be explicitly selected: ' + ', '.join(entry['requires']))
            source = bounded(root, entry['source'])
            if 'git-config' in entry.get('integrations', []):
                from gitconfig import preferences, merge, raw
                patch = preferences(root, home, entry)
                dest = bounded(home, entry['destination'])
                before = file_state(dest)
                if before and dest.stat().st_mode & 0o7000:
                    raise ValueError('unsupported special Git permission bits; existing permissions preserved')
                merged, old_keys = merge(raw(dest) if before else '', patch)
                for key, values in old_keys.items():
                    try:
                        scan('\n'.join(values))
                    except ValueError:
                        old_keys[key] = ['[existing value withheld]']
                attrs = scope.attributes(source.name)[0]
                creation_mode = (0o600 if 'private' in attrs else 0o644) | (0o111 if 'executable' in attrs else 0)
                actions.append({'path': entry['destination'], 'before': before, 'text': None, 'remove': False,
                                'git_patch': patch, 'git_before': old_keys,
                                'git_result': hashlib.sha256(merged.encode()).hexdigest(),
                                'mode': before['mode'] if before else creation_mode})
                if entry['removals']:
                    raise ValueError('Git preservation does not authorize removals')
                continue
            text = read_text(source)
            if scope.templated(entry['source']):
                template_text(text)
                text = scope.render(root, str(home), source)
            dest = bounded(home, entry['destination'])
            attrs = scope.attributes(Path(entry['source']).name)[0]
            mode_bits = (0o600 if 'private' in attrs else 0o644) | (0o111 if 'executable' in attrs else 0)
            actions.append({'path': entry['destination'], 'before': file_state(dest), 'text': text, 'remove': False, 'mode': mode_bits})
            from skills import output as skills_output
            generated = skills_output(entry)
            if generated:
                scan(text)
                target = bounded(home, generated)
                if target.exists():
                    read_text(target)
                actions.append({'path': generated, 'before': file_state(target), 'text': text, 'remove': False,
                                'mode': mode_bits, 'generated': True})
            for removal in entry['removals']:
                if mode == 'migrate' and not scope.allows(state, removal):
                    raise ValueError('migration cannot broaden removal capabilities')
                if removal in (e['destination'] for e in data['entries'].values()):
                    raise ValueError('removal conflicts with a managed destination')
                target = bounded(home, removal)
                if target.exists():
                    read_text(target)
                actions.append({'path': removal, 'before': file_state(target), 'text': None, 'remove': True})
        if mode == 'adopt':
            if data['version'] == 1 or 'entries' in request:
                data['profiles'][name] = sorted(selected)
            if data != original:
                if git(root, 'status', '--porcelain'):
                    raise ValueError('metadata/profile edits require a clean source checkout')
                actions.append({'path': MANIFEST_RELATIVE, 'repository': True, 'before': file_state(bounded(root, MANIFEST_RELATIVE)),
                                'text': json.dumps(data, indent=2, sort_keys=True) + '\n', 'remove': False})
    if len({a['path'] for a in actions}) != len(actions):
        raise ValueError('conflicting actions')
    if mode in ('track', 'metadata'):
        from tracking import source_collisions
        source_collisions(root, actions)
    result = {'version': 1, 'request': request, 'root': str(root.resolve()), 'home': str(home.resolve()),
              'actions': actions, 'selected': selected, 'inputs': inputs,
              'manifest': original, 'state': state, 'head': git(root, 'rev-parse', 'HEAD').strip()}
    if data['version'] == 2:
        from file_metadata import identity, entries as resolve_entries
        result['machine'] = identity(home, data)
        result['metadata_catalog'] = original if mode == 'untrack' else data
        result['prior_selected'] = {k: e for k, e in resolve_entries(original, home).items() if k in selected}
    if mode in ('untrack', 'metadata'):
        from tracking import index_path
        result['scope_after'] = scope_after
        result['scope_file'] = file_state(scope.profile_path(home))
        result['source_index'] = file_state(index_path(root))
    if mode not in ('track', 'untrack', 'metadata'):
        from skills import plan_backup
        result['skills_backup'] = plan_backup(home, actions, mode)
    result['id'] = digest(result)
    return result


def validate_manifest_data(data):
    if data['version'] == 2:
        from file_metadata import validate
        validate(data)
    # Same invariants as disk validation, without mutating the repository.
    seen = []
    for entry in data['entries'].values():
        metadata(entry)
        if scope.target_name(entry['source'][5:]) != entry['destination']:
            raise ValueError('source/destination naming conflict')
        for other in seen:
            a, b = entry['destination'], other['destination']
            if a == b or a.startswith(b + '/') or b.startswith(a + '/') or entry['source'] == other['source']:
                raise ValueError('conflicting destinations/sources')
        seen.append(entry)
        if any(key not in data['entries'] for key in entry.get('requires', [])):
            raise ValueError('unknown required entry')
    from skills import output as skills_output
    destinations = {e['destination'] for e in seen}
    for entry in seen:
        generated = skills_output(entry)
        if generated and any(generated == d or generated.startswith(d + '/') or d.startswith(generated + '/') for d in destinations):
            raise ValueError('generated skills output conflicts with managed destination')
        if generated:
            destinations.add(generated)
    for entry in seen:
        for removal in entry['removals']:
            if any(removal == d or removal.startswith(d + '/') or d.startswith(removal + '/') for d in destinations):
                raise ValueError('removal conflicts with a managed destination')
    if data['version'] == 2:
        from file_metadata import compile_entries
        for machine in data['machines']:
            expanded = dict(data['entries'])
            for logical, definition in data['tracking'].items():
                expanded.update(compile_entries(logical, definition, machine))
            validate_manifest_data({'version': 1, 'entries': expanded, 'profiles': data['profiles']})


def preview(value):
    mode = value['request']['mode']
    print('Stop tracking: private source/scope only; live files unchanged' if mode == 'untrack' else
          'Edit tracking metadata: private source/scope only; live files unchanged' if mode == 'metadata' else
          'Track: HOME → repository' if mode == 'track' else 'Adopt: repository → HOME')
    print('Plan: ' + value['id'])
    print('Secret scanning is detection, not a guarantee of secret absence.')
    if value.get('metadata_catalog'):
        from file_metadata import review
        review(value['metadata_catalog'], Path(value['home']), value['selected'])
    for entry in value['selected'].values():
        print(f"{entry['destination']}: {entry['portability']} / {entry['impact']} / {','.join(entry['os'])}")
        if entry.get('warnings'):
            print('Compatibility warning: Arch/CachyOS desktop configuration; Ubuntu compatibility is not promised. Separate desktop acknowledgment required.')
        if 'codex-project-trust' in entry.get('notices', []):
            print('Codex trust impact: replacing this mixed config removes machine-local project trust/hook approvals; recovery retains the previous file.')
        if 'shell-replacement' in entry.get('notices', []):
            print('Shell impact: whole-file startup replacement changes aliases, PATH and environment; unrelated startup content is retained only in private recovery. No default-shell/session change.')
        if 'config-only' in entry.get('notices', []):
            print('Configuration only: no application installation/restart, desktop provisioning, session switching, or service/timer activation.')
        if 'git-config' in entry.get('integrations', []):
            print('Git preservation: edit only explicitly selected keys; credentials, signing, includes and all other settings remain unchanged. Private backup before apply; includes/hooks are never executed during preview.')
        if entry.get('requires'):
            print('Requires explicit entries: ' + ', '.join(entry['requires']))
    if value.get('skills_backup'):
        print('Canonical skills recovery: ~/.agents/skills.bak (approved affected files only; no other skill archives).')
        if value['skills_backup']['exists']:
            print('Singleton backup conflict: --replace-skills-backup consent required; only inventoried slots replaced, unrelated backup files preserved.')
        for suffix in value['skills_backup']['files']:
            print('backup slot: .agents/skills.bak/' + suffix)
    if desktop_review(value):
        print('Desktop compatibility acknowledgment required, including removed warnings or broadened applicability.')
    if mode in ('untrack', 'metadata'):
        print('Exact scope review: ' + json.dumps(value['scope_after'], sort_keys=True) + '; no live target effects')
    base = Path(value['root'] if mode in ('track', 'untrack', 'metadata') else value['home'])
    for action in value['actions']:
        print(('remove: ' if action['remove'] else 'write: ') + action['path'])
        if 'git_patch' in action:
            for key, proposed in action['git_patch'].items():
                old = action['git_before'][key]
                try:
                    scan('\n'.join(old))
                    print('Git key ' + key + ': ' + json.dumps(old) + ' → ' + json.dumps(proposed))
                except ValueError:
                    print('Git key ' + key + ': existing value withheld → ' + json.dumps(proposed))
            continue
        from skills import skill_file
        if action['remove'] and skill_file(action['path']):
            print('Exact skill retirement only; canonical recovery uses the singleton backup; noncanonical skill files are not archived or merged.')
        old = (Path(value['root']) if action.get('repository') else base) / action['path']
        # Never disclose live credentials or include them in saved plans.
        try:
            text = read_text(old) if old.exists() else ''
        except ValueError:
            print('Existing content withheld: credential/unsupported content; recovery backup required.')
            text = ''
        proposed = action['text'] or ''
        try:
            scan(proposed)
        except ValueError:
            print('Rendered content withheld: local secret/unsupported content. Review the source template.')
            proposed = ''
        print(''.join(difflib.unified_diff(text.splitlines(True), proposed.splitlines(True),
                                         fromfile=action['path'], tofile=action['path'])), end='')


def preview_text(value):
    """Return the established full review text for a safe adapter to sanitize."""
    output = io.StringIO()
    with contextlib.redirect_stdout(output):
        preview(value)
    return output.getvalue()


def write(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    # Exclusive sibling avoids following an existing final symlink.
    import tempfile
    fd, temporary = tempfile.mkstemp(dir=path.parent)
    try:
        with os.fdopen(fd, 'w') as stream:
            stream.write(text)
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def desktop_review(value):
    old_entries = (value.get('state') or {}).get('entries', {})
    repository_entries = value.get('prior_selected', value['manifest']['entries'])
    for key, entry in value['selected'].items():
        previous = old_entries.get(key, repository_entries.get(key, {}))
        original = repository_entries.get(key, {})
        if (entry.get('warnings') or previous.get('warnings') or original.get('warnings') or
                (previous and set(entry['os']) - set(previous['os'])) or
                (original and set(entry['os']) - set(original['os']))):
            return True
    return False


def unsafe_review(value):
    old_entries = (value.get('state') or {}).get('entries', {})
    original = value.get('prior_selected', value['manifest']['entries'])
    return any(entry['impact'] == 'Unsafe' or old_entries.get(key, {}).get('impact') == 'Unsafe' or
               original.get(key, {}).get('impact') == 'Unsafe' for key, entry in value['selected'].items())


def missing_tools(home, selected):
    import shutil
    missing = sorted({tool for e in selected.values() for tool in e['tools'] if not shutil.which(tool)})
    report = home / '.local/state/dot/missing-tools.json'
    if report.resolve() != home.resolve() / report.relative_to(home):
        raise ValueError('missing-tools report must not traverse symlinks')
    write(report, json.dumps(missing) + '\n')
    print('Missing tools (advisory): ' + ', '.join(missing))


def apply(root, home, saved, approval, ack, ack_desktop=False, replace_skills_backup=False):
    if saved.get('id') != approval or digest({k: v for k, v in saved.items() if k != 'id'}) != approval:
        raise ValueError('explicit approval of the exact plan ID required')
    current = plan(root, home, saved['request'])
    if current != saved:
        raise ValueError('stale/changed plan; generate and review a new plan')
    if saved['request']['mode'] != 'update' and unsafe_review(saved) and not ack:
        raise ValueError('Unsafe intent requires --ack-unsafe')
    mode = saved['request']['mode']
    desktop = desktop_review(saved)
    if mode != 'update' and desktop and not ack_desktop:
        raise ValueError('compatibility warning/applicability review requires separate --ack-desktop')
    if saved.get('skills_backup') and saved['skills_backup']['exists'] and not replace_skills_backup:
        raise ValueError('existing singleton skills backup conflict; explicit --replace-skills-backup required')
    base = root if mode == 'track' else home
    backup = home / '.local/state/dot/recovery' / approval
    # Local state ancestors must not redirect recovery outside HOME.
    if backup.resolve() != home.resolve() / backup.relative_to(home):
        raise ValueError('local recovery state must not traverse symlinks')
    backup.mkdir(parents=True, mode=0o700, exist_ok=False)
    os.chmod(backup, 0o700)
    for index, action in enumerate(saved['actions']):
        path = bounded(root if action.get('repository') else base, action['path'])
        if file_state(path) != action['before']:
            raise ValueError('stale target changed before backup; generate and review a new plan')
        from skills import skill_file
        if path.exists() and not (mode != 'track' and not action.get('repository') and skill_file(action['path'])):
            file = backup / str(index)
            file.write_bytes(path.read_bytes())
            os.chmod(file, 0o600)
    write(backup / 'plan.json', json.dumps(saved, indent=2) + '\n')
    if mode in ('untrack', 'metadata'):
        from tracking import apply_source_only
        apply_source_only(root, home, saved, backup)
        print(('Stopped tracking' if mode == 'untrack' else 'Updated tracking metadata') + '; live files unchanged; recovery: ' + str(backup))
        return
    try:
        if mode != 'track':
            from skills import save_backup
            save_backup(home, saved, replace_skills_backup)
        for action in saved['actions']:
            path = bounded(root if action.get('repository') else base, action['path'])
            if file_state(path) != action['before']:
                raise ValueError('target changed during apply; inspect recovery before retry')
            if action['remove']:
                path.unlink(missing_ok=True)
            else:
                if 'git_patch' in action:
                    from gitconfig import merge, raw
                    text, _ = merge(raw(path) if path.exists() else '', action['git_patch'])
                    if hashlib.sha256(text.encode()).hexdigest() != action['git_result']:
                        raise ValueError('Git target changed during apply; inspect recovery before retry')
                    write(path, text)
                else:
                    write(path, action['text'])
                if 'mode' in action:
                    os.chmod(path, action['mode'])
                elif action['before']:
                    os.chmod(path, action['before']['mode'])
        if mode == 'track' or any(a.get('repository') for a in saved['actions']):
            git(root, 'add', '--', *[a['path'] for a in saved['actions'] if mode == 'track' or a.get('repository')])
            if git(root, 'diff', '--cached', '--name-only'):
                git(root, 'commit', '--only', '-m', 'Track reviewed configuration intent' if mode == 'track' else 'Adopt reviewed configuration metadata',
                    '--', *[a['path'] for a in saved['actions'] if mode == 'track' or a.get('repository')])
        if mode != 'track':
            from file_metadata import approved
            # Update plans already require identical approved capabilities.
            # Preserve the owner's exact approval bytes/mode, not a reserialization.
            if mode != 'update':
                scope.save(str(home), approved(saved['manifest'], home, saved['request']['profile'], saved['selected']))
            missing_tools(home, saved['selected'])
    except (OSError, ValueError) as error:
        raise ValueError(f'partial completion; recovery plan/backups: {backup}; {error}') from error
    print('Applied approved plan; recovery: ' + str(backup))


def main():
    global MANIFEST_PATH, MANIFEST_RELATIVE
    parser = argparse.ArgumentParser()
    parser.add_argument('command', choices=('plan', 'preview', 'apply', 'pick', 'rounds', 'inventory', 'tracked', 'update', 'recover'))
    parser.add_argument('file', nargs='?')
    parser.add_argument('--out')
    parser.add_argument('--approve')
    parser.add_argument('--ack-unsafe', action='store_true')
    parser.add_argument('--ack-desktop', action='store_true')
    parser.add_argument('--replace-skills-backup', action='store_true')
    parser.add_argument('--yes', action='store_true')
    parser.add_argument('--mode', choices=('track', 'adopt'))
    args = parser.parse_args()
    root, home = Path(os.environ['DOT_SOURCE']), Path(os.environ['HOME'])
    if os.environ.get('DOT_ENGINE') or (root / 'dot-contract.json').exists():
        from contract import check
        engine = Path(os.environ.get('DOT_ENGINE', str(Path(__file__).resolve().parent)))
        negotiated = check(engine, root)
        if Path(__file__).resolve().parent != Path(negotiated['engine']):
            raise ValueError('intent must execute from the negotiated independent engine')
        MANIFEST_PATH = negotiated['manifest']
        MANIFEST_RELATIVE = Path(MANIFEST_PATH).relative_to(root.resolve()).as_posix()
    if args.yes and args.command != 'update':
        raise ValueError('--yes only approves freshly generated unchanged-scope updates')
    if args.command == 'tracked':
        from file_metadata import inventory
        print(json.dumps(inventory(manifest(root), home, scope.load(str(home))), indent=2))
        return
    if args.command == 'update':
        state = scope.load(str(home))
        if state is None or state['version'] not in (2, 3):
            raise ValueError('updates require an approved exact scope')
        value = plan(root, home, {'version': 1, 'mode': 'update', 'profile': state['profile']})
        preview(value)
        if args.yes:
            apply(root, home, value, value['id'], False)
        else:
            print('Preview only; --yes approves a freshly generated unchanged-scope update.')
        return
    if args.command == 'pick':
        from picker import pick
        pick(root, home, args.mode)
        return
    if args.command == 'rounds':
        from picker import rounds
        rounds(root, home)
        return
    if args.command == 'inventory':
        if not args.file:
            parser.error('inventory requires an eligible HOME-relative file/directory')
        print(json.dumps(snapshot(home, {'path': args.file}), indent=2))
        return
    if not args.file:
        parser.error('a request/plan file is required')
    from contract import read_json
    data = read_json(Path(args.file))
    if args.command == 'recover':
        from tracking import recover
        recover(root, home, data, args.approve)
    elif args.command == 'plan':
        value = plan(root, home, data)
        preview(value)
        if args.out:
            output = Path(args.out)
            if output.resolve().is_relative_to(root.resolve()):
                raise ValueError('plans must remain local-unmanaged, outside the repository')
            if output.is_symlink():
                raise ValueError('plan output must not be a symlink')
            if output.exists():
                previous = json.loads(output.read_text())
                if (not isinstance(previous, dict) or 'id' not in previous or
                        digest({k: v for k, v in previous.items() if k != 'id'}) != previous['id']):
                    raise ValueError('refusing to overwrite unrelated plan output')
            write(output, json.dumps(value, indent=2) + '\n')
    elif args.command == 'preview':
        if plan(root, home, data['request']) != data:
            raise ValueError('stale/changed plan; generate and review a new plan')
        preview(data)
    else:
        apply(root, home, data, args.approve, args.ack_unsafe, args.ack_desktop, args.replace_skills_backup)


if __name__ == '__main__':
    try:
        main()
    except (ValueError, KeyError, TypeError, OSError) as error:
        print('dot intent: ' + str(error), file=sys.stderr)
        sys.exit(1)
