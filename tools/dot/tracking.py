"""Source-only tracking transactions. No target apply/forget/destroy command."""
import json
import hashlib
import sys
from pathlib import Path
import state_paths


def source_collisions(root, actions):
    import scope
    source = root / 'home'
    paths = {p.relative_to(root).as_posix() for p in source.rglob('*') if not p.is_dir() or p.is_symlink()}
    for action in actions:
        if action['path'].startswith('home/'):
            if action['remove']:
                paths.discard(action['path'])
            else:
                paths.add(action['path'])
    targets = []
    for path in sorted(paths):
        name = path.removeprefix('home/')
        if any(p.startswith('.chezmoi') for p in Path(name).parts):
            continue
        target = scope.target_name(name)
        if any(target == t or target.startswith(t + '/') or t.startswith(target + '/') for t in targets):
            raise ValueError('native source destination collision')
        targets.append(target)


def index_path(root):
    import intent
    path = Path(intent.git(root, 'rev-parse', '--git-path', 'index').strip())
    return path if path.is_absolute() else root / path


def after_state(action):
    if action['remove']:
        return None
    return {'sha256': hashlib.sha256(action['text'].encode()).hexdigest(),
            'mode': action.get('mode', action['before']['mode'] if action['before'] else 0o600)}


def pending(home):
    from contract import read_json
    recovery = state_paths.state_dir(home) / 'recovery'
    if recovery.resolve() != state_paths.state_dir(home).resolve() / 'recovery':
        raise ValueError('recovery must not traverse symlinks')
    if recovery.exists():
        for directory in recovery.iterdir():
            journal = directory / 'transaction.json'
            if journal.exists():
                record = read_json(journal)
                if (not isinstance(record, dict) or set(record) != {'status', 'live_files'} or
                        record['status'] not in ('pending', 'complete', 'rolled-back') or record['live_files'] != 'unchanged'):
                    raise ValueError('invalid private tracking recovery journal')
                if journal.resolve() != recovery.resolve() / journal.relative_to(recovery) or record['status'] == 'pending':
                    raise ValueError('pending tracking transaction; inspect recovery and approve rollback first')


def recover(root, home, saved, approval):
    import intent
    import scope
    from contract import read_json
    if (saved.get('id') != approval or intent.digest({k: v for k, v in saved.items() if k != 'id'}) != approval or
            saved['request']['mode'] not in ('untrack', 'metadata') or
            saved['root'] != str(root.resolve()) or saved['home'] != str(home.resolve())):
        raise ValueError('explicit approval of the exact tracking recovery plan required')
    backup = state_paths.state_dir(home) / 'recovery' / approval
    if backup.resolve() != state_paths.state_dir(home).resolve() / 'recovery' / approval:
        raise ValueError('recovery must not traverse symlinks')
    if read_json(backup / 'plan.json') != saved or read_json(backup / 'transaction.json')['status'] != 'pending':
        raise ValueError('pending exact tracking recovery required')
    committed = intent.git(root, 'rev-parse', 'HEAD').strip() != saved['head']
    if committed and (intent.git(root, 'rev-parse', 'HEAD^').strip() != saved['head'] or intent.git(root, 'status', '--porcelain')):
        raise ValueError('recovery HEAD changed; inspect private journal before continuing')
    restores = []
    for i, action in enumerate(saved['actions']):
        path = intent.bounded(root, action['path'])
        before = action['before']
        after = after_state(action)
        if intent.file_state(path) not in (before, after):
            raise ValueError('recovery source changed; unrelated edits preserved')
        if committed and intent.file_state(path) != after:
            raise ValueError('committed recovery does not match exact reviewed source changes')
        snapshot = backup / str(i)
        if before and (snapshot.is_symlink() or not snapshot.is_file() or
                       hashlib.sha256(snapshot.read_bytes()).hexdigest() != before['sha256']):
            raise ValueError('recovery backup changed')
        restores.append((path, snapshot, before))
    profile = scope.profile_path(home)
    old_scope_path = backup / 'scope.before'
    if (old_scope_path.is_symlink() or old_scope_path.exists() != (saved['state'] is not None) or
            (saved['scope_file'] is not None and
             intent.file_state(old_scope_path)['sha256'] != saved['scope_file']['sha256'])):
        raise ValueError('recovery scope backup missing or changed')
    old_scope = old_scope_path.read_bytes() if old_scope_path.exists() else None
    if old_scope is not None and json.loads(old_scope) != saved['state']:
        raise ValueError('recovery scope backup changed')
    current_scope = scope.load(str(home))
    if current_scope not in (saved['state'], saved['scope_after']):
        raise ValueError('recovery scope changed; unrelated approvals preserved')
    if committed:
        expected = {a['path'] for a in saved['actions'] if a['before'] != after_state(a)}
        changed = set(intent.git(root, 'diff', '--name-only', saved['head'], 'HEAD').splitlines())
        if changed != expected or current_scope != saved['scope_after']:
            raise ValueError('committed recovery scope/changes do not match exact plan')
        intent.write(backup / 'transaction.json', json.dumps({'status': 'complete', 'live_files': 'unchanged'}) + '\n')
        print('Finalized exact committed source/scope transaction; live files unchanged')
        return
    index = index_path(root)
    before_index = backup / 'index.before'
    if before_index.is_symlink() or intent.file_state(before_index)['sha256'] != saved['source_index']['sha256']:
        raise ValueError('recovery index backup changed')
    indexes = [p.read_bytes() for p in (backup / 'index.before', backup / 'index.after') if p.is_file() and not p.is_symlink()]
    if not indexes or index.read_bytes() not in indexes:
        raise ValueError('recovery index changed; staged work preserved')
    for path, snapshot, before in restores:
        if before:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(snapshot.read_bytes())
            path.chmod(before['mode'])
        else:
            path.unlink(missing_ok=True)
    if old_scope is None:
        profile.unlink(missing_ok=True)
    else:
        profile.write_bytes(old_scope)
        profile.chmod(saved['scope_file']['mode'])
    index.write_bytes((backup / 'index.before').read_bytes())
    intent.write(backup / 'transaction.json', json.dumps({'status': 'rolled-back', 'live_files': 'unchanged'}) + '\n')
    print('Recovered private source/scope; live files unchanged')


def untrack(root, home, request, data, state, actions, selected, inputs):
    import intent
    ids = request.get('entries')
    if not isinstance(ids, list) or not ids or len(set(ids)) != len(ids) or any(i not in data['entries'] for i in ids):
        raise ValueError('unknown exact tracking entries')
    if state is not None and state['version'] not in (2, 3):
        raise ValueError('stop tracking requires exact scope or an unapproved machine')
    if intent.git(root, 'status', '--porcelain'):
        raise ValueError('stop tracking requires a clean source checkout')
    from file_metadata import entries as resolve_entries, applicable
    resolved = resolve_entries(data, home)
    for key in ids:
        entry = resolved[key]
        if not applicable(data, home, key):
            raise ValueError('entry is not applicable to this explicit machine')
        data['entries'].pop(key)
        current_os = 'linux' if sys.platform.startswith('linux') else sys.platform
        if current_os not in entry['os']:
            raise ValueError('entry is not applicable to this OS')
        selected[key] = entry
        target = intent.bounded(home, entry['destination'])
        inputs[entry['destination']] = intent.file_state(target)
        for name in [entry['source']] + ([entry['git_source']] if 'git_source' in entry else []):
            source = intent.bounded(root, name)
            if not source.is_file():
                raise ValueError('tracked source is missing')
            if any(other.get('git_source') == name or other['source'] == name for other in data['entries'].values()):
                raise ValueError('tracked source is shared; stop all owners together')
            actions.append({'path': name, 'repository': True,
                            'before': intent.file_state(source), 'text': None, 'remove': True})
    if data['version'] == 2:
        for logical, definition in list(data['tracking'].items()):
            for system in list(definition['destinations']):
                if logical + '@' + system in selected:
                    definition['destinations'].pop(system)
                    definition['sources'].pop(system)
                    definition['os_metadata'].pop(system, None)
            if not definition['destinations']:
                data['tracking'].pop(logical)
    for name, members in data['profiles'].items():
        data['profiles'][name] = [i for i in members if i not in selected]
    intent.validate_manifest_data(data)
    actions.append({'path': intent.MANIFEST_RELATIVE, 'repository': True,
                    'before': intent.file_state(intent.bounded(root, intent.MANIFEST_RELATIVE)),
                    'text': json.dumps(data, indent=2, sort_keys=True) + '\n', 'remove': False})
    after = json.loads(json.dumps(state)) if state else None
    if after:
        for key in selected:
            after['entries'].pop(key, None)
    return after


def apply_source_only(root, home, saved, backup):
    """Journal source and scope together; rollback both on ordinary failure.

    An interrupted process leaves private backups plus a pending journal. A
    subsequent plan refuses that pending transaction instead of guessing.
    """
    import intent
    import scope
    profile = scope.profile_path(home)
    old_scope = profile.read_bytes() if profile.exists() else None
    if old_scope is not None:
        (backup / 'scope.before').write_bytes(old_scope)
        (backup / 'scope.before').chmod(0o600)
    index = index_path(root)
    old_index = index.read_bytes() if index.exists() else None
    if intent.file_state(index) != saved['source_index']:
        raise ValueError('source index changed before transaction')
    owned_index = None
    owned_stage = None
    if old_index is not None:
        (backup / 'index.before').write_bytes(old_index)
        (backup / 'index.before').chmod(0o600)
    journal = backup / 'transaction.json'
    intent.write(journal, json.dumps({'status': 'pending', 'live_files': 'unchanged'}) + '\n')
    try:
        for name, before in saved['inputs'].items():
            if intent.file_state(intent.bounded(home, name)) != before:
                raise ValueError('live target changed; review a fresh plan')
        for action in saved['actions']:
            path = intent.bounded(root, action['path'])
            if intent.file_state(path) != action['before']:
                raise ValueError('source changed during apply')
            if action['remove']:
                path.unlink()
            else:
                intent.write(path, action['text'])
                if 'mode' in action:
                    path.chmod(action['mode'])
                elif action['before']:
                    path.chmod(action['before']['mode'])
        if saved['scope_after'] is not None:
            scope.save(str(home), saved['scope_after'])
        for name, before in saved['inputs'].items():
            if intent.file_state(intent.bounded(home, name)) != before:
                raise ValueError('live target changed during source transaction')
        if index.read_bytes() != old_index:
            raise ValueError('source index changed during transaction; staged work preserved')
        intent.git(root, 'add', '--', *[a['path'] for a in saved['actions']])
        staged = intent.git(root, 'diff', '--cached', '--name-only').splitlines()
        if set(staged) - {a['path'] for a in saved['actions']}:
            raise ValueError('unrelated staged work appeared; not committed or discarded')
        owned_index = index.read_bytes()
        owned_stage = intent.git(root, 'ls-files', '--stage')
        (backup / 'index.after').write_bytes(owned_index)
        (backup / 'index.after').chmod(0o600)
        if staged:
            intent.git(root, 'commit', '--only', '-m', 'Review source tracking and exact scope; retain live files',
                       '--', *[a['path'] for a in saved['actions']])
    except (OSError, ValueError):
        # Unknown concurrent work is never overwritten by automatic rollback.
        if (index.read_bytes() not in (old_index, owned_index) and
                (owned_stage is None or intent.git(root, 'ls-files', '--stage') != owned_stage)) or any(
                intent.file_state(intent.bounded(root, a['path'])) not in (a['before'], after_state(a))
                for a in saved['actions']):
            raise ValueError('partial completion; unrelated work preserved; inspect private recovery: ' + str(backup))
        if scope.load(str(home)) not in (saved['state'], saved['scope_after']):
            raise ValueError('partial completion; unrelated approvals preserved; inspect private recovery: ' + str(backup))
        for i, action in enumerate(saved['actions']):
            path = intent.bounded(root, action['path'])
            if action['before']:
                path.write_bytes((backup / str(i)).read_bytes())
                path.chmod(action['before']['mode'])
            else:
                path.unlink(missing_ok=True)
        if old_scope is not None:
            profile.write_bytes(old_scope)
            profile.chmod(saved['scope_file']['mode'])
        elif profile.exists():
            profile.unlink()
        if old_index is not None:
            index.write_bytes(old_index)
        intent.write(journal, json.dumps({'status': 'rolled-back', 'live_files': 'unchanged'}) + '\n')
        raise
    intent.write(journal, json.dumps({'status': 'complete', 'live_files': 'unchanged'}) + '\n')
