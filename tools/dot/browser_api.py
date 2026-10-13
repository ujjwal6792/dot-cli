#!/usr/bin/env python3
"""Line-delimited JSON adapter over the shared intent engine."""
import contextlib
import difflib
import importlib
import io
import json
import os
from pathlib import Path
import re
import stat
import sys
import unicodedata

import contract


def clean(value):
    """Remove terminal/control characters recursively from public response data."""
    if isinstance(value, str):
        return ''.join(c for c in value if unicodedata.category(c) not in ('Cc', 'Cf', 'Cs'))
    if isinstance(value, list):
        return [clean(v) for v in value]
    if isinstance(value, tuple):
        return [clean(v) for v in value]
    if isinstance(value, dict):
        return {clean(k): clean(v) for k, v in value.items()}
    return value


def safe_text(value):
    """Sanitize display text while retaining readable line/tab layout."""
    if not isinstance(value, str):
        return ''
    return ''.join(c for c in value if c in '\n\t' or unicodedata.category(c) not in ('Cc', 'Cf', 'Cs'))


def safe_label(value):
    """Escape controls in path labels so they cannot alter terminal output."""
    if not isinstance(value, str):
        return '[invalid path]'
    return ''.join(c if c.isprintable() and c not in '\\' else f'\\u{ord(c):04x}' for c in value)


def clean_display(value, key=''):
    if isinstance(value, dict):
        return {clean_display(k): clean_display(v, k) for k, v in value.items()}
    if isinstance(value, list):
        return [clean_display(v, key) for v in value]
    if isinstance(value, str):
        if key == 'path':
            return safe_label(value)
        if key in ('text', 'json', 'review', 'summary'):
            return safe_text(value)
        return clean(value)
    return value


def safe_error(error):
    # Exception strings can contain paths, bytes, or private values. Keep only a
    # stable category/message selected by type, never the underlying exception.
    if isinstance(error, json.JSONDecodeError):
        return 'invalid JSON request'
    if isinstance(error, FileNotFoundError):
        return 'required path is missing'
    if isinstance(error, PermissionError):
        return 'required path is not accessible'
    if isinstance(error, (KeyError, TypeError, IndexError)):
        return 'invalid request structure'
    if isinstance(error, ValueError):
        message = str(error)
        if message.startswith('source:'):
            return clean(message)
        if message.startswith('machine:'):
            return clean(message)
        if message.startswith('assignment:'):
            return clean(message)
        if message.startswith('packages:'):
            # Package errors are fixed policy messages, never source bytes or
            # arbitrary manager stderr. The manager identifier is allowlisted.
            return clean(message)
        if message.startswith('explicit approval of the exact plan ID required'):
            return 'explicit approval of the exact plan ID required'
        if message.startswith('Unsafe intent requires --ack-unsafe'):
            return 'Unsafe intent requires --ack-unsafe'
        if message.startswith('compatibility warning/applicability review requires separate --ack-desktop'):
            return 'compatibility warning/applicability review requires separate --ack-desktop'
        if message.startswith('existing singleton skills backup conflict; explicit --replace-skills-backup required'):
            return 'existing singleton skills backup conflict; explicit --replace-skills-backup required'
        if message.startswith('stale/changed plan; generate and review a new plan'):
            return 'stale/changed plan; generate and review a new plan'
        allowed = (
            'unsupported bridge protocol version', 'unsupported bridge command',
            'unsupported bridge request fields', 'explicit engine and source paths required',
            'explicit HOME must be an existing absolute directory',
            'engine and private source must be independent',
            'bridge must execute from the negotiated independent engine',
            'exact plan required', 'exact reviewed plan required',
            'stale/changed plan; generate and review a new plan',
            'selected file does not exist', 'blanket HOME tracking rejected',
            'directory snapshot contains no eligible regular files',
            'snapshot path must be eligible', 'path is not an eligible regular file',
            'preview file unavailable or unsupported', 'consent fields must be booleans',
            'approval of the exact plan ID required', 'Unsafe intent requires --ack-unsafe',
            'compatibility warning/applicability review requires separate --ack-desktop',
            'existing singleton skills backup conflict; explicit --replace-skills-backup required',
            'unsupported/nondeclarative template function', 'unsupported/binary content rejected',
            'credential content detected; raw import rejected; review a secret-free template',
            'stable explicit machine identity required', 'invalid explicit machine identity',
            'unknown or insecure explicit machine identity', 'machine identity OS mismatch',
            'approved machine identity changed; fresh adoption required',
            'unsupported OS mapping', 'native source destination collision',
            'pending tracking transaction; inspect recovery and approve rollback first',
        )
        for prefix in allowed:
            if message == prefix or message.startswith(prefix + ':'):
                return clean(message)
    return 'engine rejected request; verify paths, eligibility, and exact review'


def _entry_by_destination(data):
    return {entry['destination']: entry for entry in data['entries'].values()}


def _path_status(intent, home, rel):
    if not isinstance(rel, str):
        return 'unavailable', None
    try:
        # Distinguish symlinks before path policy (all links are non-followable).
        raw = home / rel
        if raw.is_symlink():
            return 'symlink', raw
    except (TypeError, OSError):
        return 'unavailable', None
    try:
        path = intent.bounded(home, rel)
    except ValueError:
        return 'excluded', None
    try:
        mode = path.lstat().st_mode
    except OSError:
        return 'missing', path
    if stat.S_ISLNK(mode):
        return 'symlink', path
    if stat.S_ISDIR(mode):
        return 'eligible', path
    if not stat.S_ISREG(mode):
        return 'unsupported', path
    try:
        intent.read_text(path)
    except ValueError as error:
        return ('restricted' if 'credential content detected' in str(error) else 'unsupported'), path
    except OSError:
        return 'unavailable', path
    return 'eligible', path


def inventory(intent, home, mode='track'):
    catalog = intent.manifest(intent.ROOT)
    from file_metadata import entries as resolve_entries, applicable, check_scope
    resolved = resolve_entries(catalog, home)
    catalog = dict(catalog, entries=resolved)
    state = intent.scope.load(str(home))
    check_scope(catalog, home, state)
    machines = set(catalog.get('machines', {}))
    if state and state['version'] == 3:
        machines.add(state['machine']['id'])
    for entry in catalog['entries'].values():
        assigned = entry.get('machines', {'include': [], 'exclude': []})
        machines.update(assigned['include'] + assigned['exclude'])
    machines.discard('all')
    rows = []
    if mode in ('adopt', 'untrack'):
        os_name = 'linux' if sys.platform.startswith('linux') else sys.platform
        folders = set()
        for entry in catalog['entries'].values():
            if mode == 'untrack' and os_name not in entry['os']:
                continue
            components = entry['destination'].split('/')[:-1]
            folders.update('/'.join(components[:i]) for i in range(1, len(components) + 1))
        for folder in sorted(folders):
            rows.append({'path': folder, 'kind': 'directory', 'status': 'logical',
                         'destination': folder, 'logical': True})
        for identifier, entry in sorted(catalog['entries'].items()):
            machine_ok = applicable(catalog, home, identifier)
            if mode == 'untrack' and (os_name not in entry['os'] or not machine_ok):
                continue
            destination = entry['destination']
            status, path = _path_status(intent, home, destination)
            os_status = 'supported' if os_name in entry['os'] and machine_ok else 'unsupported'
            kind = 'file'
            approved = bool(state and state.get('version') in (2, 3) and
                            state.get('entries', {}).get(identifier) == entry)
            # Inventory is selection metadata, not target-side write eligibility:
            # adopting over local/private data is reviewed by the shared planner.
            rows.append({'path': destination, 'kind': kind, 'status': 'managed',
                          'destination': destination, 'id': identifier, 'metadata': entry,
                          'target_status': status, 'os_status': os_status, 'approved': approved})
            for logical, definition in catalog.get('tracking', {}).items():
                if identifier.startswith(logical + '@'):
                    rows[-1].update(logical_id=logical, tracking=definition)
        profiles = [{'name': name, 'entries': identifiers}
                    for name, identifiers in sorted(catalog['profiles'].items())]
        for profile in profiles:
            profile['entries'] = [key for key in profile['entries'] if os_name in resolved[key]['os'] and applicable(catalog, home, key)]
        return {'mode': mode, 'os': os_name, 'entries': rows, 'profiles': profiles, 'machines': sorted(machines)}

    # Safe hierarchical inventory. Never enter excluded names, symlinks, or
    # repository internals; the frontend controls which folders are visible.
    def contains_repository_marker(directory):
        try:
            return (directory / '.git').exists() or (directory / '.git').is_symlink()
        except OSError:
            return False

    def walk(directory, prefix=''):
        try:
            children = sorted(directory.iterdir(), key=lambda p: p.name)
        except OSError:
            return
        for path in children:
            rel = path.relative_to(home).as_posix()
            name = path.name.lower()
            if name == '.git':
                continue
            status, target = _path_status(intent, home, rel)
            kind = ('symlink' if path.is_symlink() else
                    'directory' if path.is_dir() and not path.is_symlink() else
                    'file' if path.is_file() and not path.is_symlink() else 'other')
            if kind == 'directory' and contains_repository_marker(path):
                status = 'excluded'
            if status == 'excluded':
                # Display excluded directory node, but never enumerate its content.
                kind = 'directory' if kind == 'directory' else kind
            entry = _entry_by_destination(catalog).get(rel)
            row = {'path': rel, 'kind': kind, 'status': status}
            if entry:
                identifier = next(k for k, e in resolved.items() if e == entry)
                row.update(destination=rel, metadata=entry, id=identifier,
                           approved=bool(state and state.get('version') in (2, 3) and state['entries'].get(identifier) == entry))
                for logical, definition in catalog.get('tracking', {}).items():
                    if identifier.startswith(logical + '@'):
                        row.update(logical_id=logical, tracking=definition)
                if status == 'eligible':
                    row['status'] = 'managed'
            rows.append(row)
            if (kind == 'directory' and status == 'eligible' and not path.is_symlink()
                    and path.name.lower() not in intent.EXCLUDED
                    and path.name.lower() != '.git'
                    and not contains_repository_marker(path)
                    and not (path / '.git').exists()):
                walk(path)

    walk(home)
    from file_metadata import current_os
    return {'mode': mode, 'os': current_os(), 'entries': rows, 'machines': sorted(machines)}


def snapshot(intent, home, rel):
    status, path = _path_status(intent, home, rel)
    if path is None or status not in ('eligible',):
        return {'path': clean(rel), 'paths': [], 'skipped': 0, 'status': status}
    # The shared intent snapshot applies exact path/exclusion/secret/binary rules.
    selected = intent.snapshot(home, {'path': rel})
    if path.is_dir() and not selected:
        return {'path': safe_label(rel), 'paths': [], 'skipped': 0, 'status': 'excluded'}
    skipped = 0
    if path.is_dir():
        regular = 0
        pending = [path]
        while pending:
            current = pending.pop()
            for child in current.iterdir():
                child_status, child_path = _path_status(intent, home, child.relative_to(home).as_posix())
                if child_status == 'excluded' or child_status == 'symlink':
                    if child.is_file() or child.is_symlink():
                        skipped += 1
                    continue
                if child.is_dir() and not child.is_symlink():
                    if (child / '.git').exists() or (child / '.git').is_symlink():
                        continue  # never inspect unrelated repository contents for counts
                    pending.append(child)
                elif child.is_file() and not child.is_symlink():
                    regular += 1
                    if child_status != 'eligible':
                        skipped += 1
        skipped = max(skipped, regular - len(selected))
    return {'path': safe_label(rel), 'paths': [safe_label(v) for v in selected], 'skipped': skipped, 'status': 'eligible'}


def review(intent, value):
    with contextlib.redirect_stdout(io.StringIO()):
        intent.preview(value)
    mode = value['request']['mode']
    source_only = mode in ('track', 'untrack', 'metadata')
    base = Path(value['root'] if source_only else value['home'])
    result = []
    for action in value['actions']:
        label = 'REPOSITORY' if action.get('repository') or source_only else 'TARGET'
        target = (Path(value['root']) if action.get('repository') else base) / action['path']
        if action.get('git_patch') is not None:
            old = '\n'.join(x for values in action.get('git_before', {}).values() for x in values)
            new = '\n'.join(k + ' = ' + ', '.join(v) for k, v in action['git_patch'].items())
            try:
                intent.scan(old + '\n' + new)
                text = ''.join(difflib.unified_diff(old.splitlines(True), new.splitlines(True),
                                                   fromfile=label + '/' + action['path'], tofile='PROPOSED/' + action['path'], lineterm='\n'))
                summary = 'Update explicitly selected Git keys; other settings preserved.'
            except ValueError:
                text = 'Existing Git values withheld; selected key updates are bounded.'
                summary = 'Bounded Git key update; sensitive prior values withheld.'
            safe = safe_text(text)
            result.append({'path': safe_label(action['path']), 'summary': safe_text(summary), 'text': safe, 'json': safe})
            continue
        summary = 'Remove exact file' if action['remove'] else ('Replace exact file' if action.get('before') else 'Create exact file')
        old_mode = (action.get('before') or {}).get('mode')
        new_mode = None if action['remove'] else action.get('mode', old_mode if old_mode is not None else 0o600)
        old_label = f'{old_mode:04o}' if old_mode is not None else 'none'
        new_label = f'{new_mode:04o}' if new_mode is not None else 'removed'
        summary += f'; mode {old_label} → {new_label}'
        try:
            before = intent.read_text(target) if target.is_file() and not target.is_symlink() else ''
            proposed = action.get('text') or ''
            intent.scan(proposed)
            text = ''.join(difflib.unified_diff(before.splitlines(True), proposed.splitlines(True),
                                                fromfile=label + '/' + action['path'], tofile='PROPOSED/' + action['path'], lineterm='\n'))
        except (ValueError, OSError):
            text = 'Content withheld: credential, binary, or unsupported content.'
            summary += '; content withheld'
        if action.get('repository'):
            text = text.replace('REPOSITORY/', 'REPOSITORY/').replace('PROPOSED/', 'REPOSITORY-PROPOSED/')
        safe = safe_text(text)
        result.append({'path': safe_label(action['path']), 'summary': safe_text(summary), 'text': safe, 'json': safe})
    return {'diffs': result,
            'direction': ('private source/scope only; live files unchanged' if mode in ('untrack', 'metadata') else
                          'HOME → repository' if mode == 'track' else 'repository → TARGET'),
            'unsafe': bool(intent.unsafe_review(value)),
            'desktop': bool(intent.desktop_review(value)),
            'backup': bool((value.get('skills_backup') or {}).get('exists')),
            'gates_known': True,
            'review': safe_text(intent.preview_text(value))}


def handle(request):
    if not isinstance(request, dict) or type(request.get('version')) is not int or request.get('version') != 1:
        raise ValueError('unsupported bridge protocol version')
    command = request.get('command')
    common = {'version', 'command', 'engine', 'source', 'home'}
    allowed = {
        'inventory': common | {'mode'}, 'snapshot': common | {'path'},
        'highlight': common | {'path', 'mode', 'request'}, 'plan': common | {'request'},
        'preview': common | {'plan'},
        'apply': common | {'plan', 'approve', 'ack_unsafe', 'ack_desktop', 'replace_skills_backup'},
        'packages': common | {'request', 'plan', 'approve'},
        'init-source': common | {'request'},
        'init-machine': common | {'request'},
    }
    if command not in allowed:
        raise ValueError('unsupported bridge command')
    if set(request) - allowed[command]:
        raise ValueError('unsupported bridge request fields')
    for key in ('engine', 'source'):
        if not isinstance(request.get(key), str) or not Path(request[key]).is_absolute():
            raise ValueError('explicit engine and source paths required')
    if command == 'init-source':
        engine_path = Path(request['engine']).resolve()
        if Path(__file__).resolve().parent != engine_path:
            raise ValueError('bridge must execute from the negotiated independent engine')
        home = request.get('home')
        if not isinstance(home, str) or not Path(home).is_absolute() or not Path(home).is_dir() or home == '/':
            raise ValueError('explicit HOME must be an existing absolute directory')
        os.environ['HOME'] = home
        import init
        result = init.source_flow(Path(request['source']), engine_path, request.get('request'))
        return {'version': 1, 'ok': True, 'result': result}
    negotiated = contract.check(Path(request['engine']), Path(request['source']))
    engine_path, source_path = Path(negotiated['engine']), Path(negotiated['source'])
    if Path(__file__).resolve().parent != engine_path:
        raise ValueError('bridge must execute from the negotiated independent engine')
    if str(engine_path) not in sys.path:
        sys.path.insert(0, str(engine_path))
    if command == 'packages':
        try:
            contract.bounded(engine_path, 'packages.py')
        except ValueError:
            raise ValueError('packages: package capability missing or unsafe; update compatible public engine and browser together') from None
        import packages
        fields = request.get('request')
        if not isinstance(fields, dict) or set(fields) - {'action', 'manager', 'digest', 'text', 'supplied', 'entrypoint', 'ack_aur'}:
            raise ValueError('packages: unsupported request fields')
        action = fields.get('action')
        if fields.get('entrypoint') not in ('browse', 'init'):
            raise ValueError('packages: only browse and init can review installation')
        if type(fields.get('ack_aur', False)) is not bool:
            raise ValueError('packages: AUR acknowledgment must be boolean')
        planner = packages.Planner(source_path)
        if action == 'load':
            result = planner.load(fields.get('manager'))
        elif action == 'save':
            result = {'sources': {fields.get('manager'): planner.save(fields.get('manager'), fields.get('digest'), fields.get('text'))}}
        elif action == 'import':
            result = {'sources': {fields.get('manager'): planner.import_file(fields.get('manager'), fields.get('digest'), fields.get('supplied'))}}
        elif action == 'status':
            result = planner.overview()
        elif action == 'plan':
            plan = planner.plan(fields['entrypoint'])
            result = {'plan': plan, 'review': planner.review(plan), 'aur': any(r['manager'] == 'yay' for r in plan['missing'])}
        elif action == 'apply':
            result = planner.apply(request.get('plan'), request.get('approve'), fields['entrypoint'], fields.get('ack_aur', False))
        else:
            raise ValueError('packages: unsupported action')
        # Plans retain exact transport semantics; only human display is cleaned.
        opaque = result.get('plan')
        # Editor payload is data, not a terminal preview. Sanitizing it here
        # would silently rewrite unknown Ruby/comments and invalidate its digest.
        editor_text = result.get('text')
        source_text = {manager: source['text'] for manager, source in result.get('sources', {}).items()}
        result = clean_display({k: v for k, v in result.items() if k != 'plan'})
        if opaque is not None:
            result['plan'] = opaque
        if editor_text is not None:
            result['text'] = editor_text
        for manager, text in source_text.items():
            result['sources'][manager]['text'] = text
        return {'version': 1, 'ok': True, 'result': result}
    sys.modules.pop('intent', None)
    intent = importlib.import_module('intent')
    intent.MANIFEST_PATH = negotiated['manifest']
    intent.MANIFEST_RELATIVE = Path(negotiated['manifest']).relative_to(source_path).as_posix()
    home_arg = request.get('home', os.environ.get('HOME'))
    if not isinstance(home_arg, str) or not Path(home_arg).is_absolute() or not Path(home_arg).is_dir():
        raise ValueError('explicit HOME must be an existing absolute directory')
    home = Path(home_arg).resolve(strict=True)
    if home == Path('/'):
        raise ValueError('explicit HOME must be an existing absolute directory')
    intent.ROOT = source_path
    sys.modules.pop('scope', None)
    intent.scope = importlib.import_module('scope')

    if command == 'init-machine':
        from file_metadata import machine_setup
        result = machine_setup(home, intent.manifest(source_path), request.get('request'))
    elif command == 'inventory':
        mode = request.get('mode', 'track')
        if mode not in ('track', 'adopt', 'untrack'):
            raise ValueError('unsupported inventory mode')
        result = inventory(intent, home, mode)
    elif command == 'snapshot':
        result = snapshot(intent, home, request.get('path'))
    elif command == 'highlight':
        rel = request.get('path')
        status, path = _path_status(intent, home, rel)
        mode = request.get('mode', 'track')
        if mode not in ('track', 'adopt', 'untrack'):
            raise ValueError('unsupported inventory mode')
        catalog = intent.manifest(source_path)
        from file_metadata import entries as resolve_entries
        catalog = dict(catalog, entries=resolve_entries(catalog, home))
        entry_id = next((key for key, entry in catalog['entries'].items()
                         if entry['destination'] == rel), None)
        request_data = request.get('request')
        if request_data is None and mode in ('adopt', 'untrack') and entry_id is not None:
            request_data = {'version': 1, 'mode': mode, 'profile': 'highlight', 'entries': [entry_id]}
        if request_data is not None or (status == 'eligible' and path.is_file()):
            try:
                metadata = {'path': rel, 'os': ['darwin', 'linux'],
                            'portability': 'managed-shared', 'impact': 'Safe'}
                request_data = request_data or {'version': 1, 'mode': 'track', 'profile': 'highlight', 'selections': [metadata]}
                with contextlib.redirect_stdout(io.StringIO()):
                    plan = intent.plan(source_path, home, request_data)
                rendered = review(intent, plan)
                # Track writes a source path, not the highlighted HOME path.
                # Resolve that relationship here using the engine's selected
                # metadata; the frontend must not infer chezmoi naming/policy.
                if mode in ('track', 'untrack'):
                    sources = {entry.get('git_source', entry['source'])
                               for entry in plan['selected'].values()}
                    matching = [diff for diff in rendered['diffs']
                                if diff['path'] in sources]
                    rendered['text'] = '\n'.join(
                        diff['path'] + ' · ' + diff['summary'] + '\n' + diff['text']
                        for diff in matching)
                result = {'status': status, **rendered,
                          'plan_id': plan['id'], 'direction': rendered.get('direction', 'HOME → repository')}
            except (ValueError, OSError):
                result = {'status': status, 'summary': 'Eligible file; no safe preview available.'}
        else:
            result = {'status': status, 'summary': 'No content preview for ineligible file.'}
    elif command == 'plan':
        with contextlib.redirect_stdout(io.StringIO()):
            plan = intent.plan(source_path, home, request.get('request'))
        rendered = review(intent, plan)
        result = {'plan': plan, **rendered}
    elif command == 'preview':
        plan = request.get('plan')
        if not isinstance(plan, dict):
            raise ValueError('exact plan required')
        with contextlib.redirect_stdout(io.StringIO()):
            fresh = intent.plan(source_path, home, plan['request'])
        if fresh != plan:
            raise ValueError('stale/changed plan; generate and review a new plan')
        rendered = review(intent, plan)
        result = {'plan_id': plan['id'], **rendered}
    else:
        plan = request.get('plan')
        if not isinstance(plan, dict):
            raise ValueError('exact reviewed plan required')
        consent = ('ack_unsafe', 'ack_desktop', 'replace_skills_backup')
        if any(type(request.get(k, False)) is not bool for k in consent):
            raise ValueError('consent fields must be booleans')
        with contextlib.redirect_stdout(io.StringIO()):
            intent.apply(source_path, home, plan, request.get('approve'),
                         request.get('ack_unsafe', False), request.get('ack_desktop', False),
                         request.get('replace_skills_backup', False))
        import shutil
        missing = sorted({tool for entry in plan.get('selected', {}).values()
                          for tool in entry.get('tools', []) if not shutil.which(tool)})
        result = {'plan_id': plan['id'], 'applied': True,
                  'summary': 'Applied exact reviewed plan; recovery backup created.',
                  'missing_tools': [safe_label(tool) for tool in missing]}
    # The exact plan is opaque to the frontend and must remain byte-semantically
    # intact for intent.apply's digest check. Every displayed field is cleaned.
    if isinstance(result, dict) and isinstance(result.get('plan'), dict):
        opaque = result['plan']
        result = clean_display({k: v for k, v in result.items() if k != 'plan'})
        result['plan'] = opaque
    else:
        result = clean_display(result)
    return {'version': 1, 'ok': True, 'result': result}


def main():
    try:
        line = sys.stdin.readline()
        if not line or sys.stdin.readline():
            raise ValueError('unsupported bridge request fields')
        response = handle(json.loads(line, object_pairs_hook=contract.unique_object))
        print(json.dumps(response, sort_keys=True, ensure_ascii=True))
        return 0
    except (ValueError, KeyError, TypeError, OSError, json.JSONDecodeError) as error:
        print(json.dumps({'version': 1, 'ok': False, 'error': safe_error(error)}, sort_keys=True, ensure_ascii=True))
        return 1


if __name__ == '__main__':
    sys.exit(main())
