"""Versioned private logical-file metadata; resolved at every authorization seam."""
import json
from pathlib import Path
import re
import sys

SYSTEMS = {'darwin', 'linux'}
FIELDS = {'portability', 'impact', 'tools', 'removals', 'hooks', 'warnings',
          'integrations', 'requires', 'notices'}


def current_os():
    name = 'linux' if sys.platform.startswith('linux') else sys.platform
    if name not in SYSTEMS:
        raise ValueError('unsupported current OS')
    return name


def identifier(value):
    if not isinstance(value, str) or not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_-]{0,127}', value):
        raise ValueError('invalid explicit machine/logical identity')
    return value


def identity(home, data):
    from intent import file_state
    path = Path(home) / '.local/state/dot/machine.json'
    if path.resolve() != Path(home).resolve() / '.local/state/dot/machine.json' or not path.is_file() or path.is_symlink():
        raise ValueError('stable explicit machine identity required')
    from contract import read_json
    record = read_json(path)
    if not isinstance(record, dict) or set(record) != {'version', 'id'} or type(record['version']) is not int or record['version'] != 1:
        raise ValueError('invalid explicit machine identity')
    key = identifier(record['id'])
    if path.stat().st_mode & 0o7777 != 0o600 or key not in data['machines']:
        raise ValueError('unknown or insecure explicit machine identity')
    machine = data['machines'][key]
    if machine['os'] != current_os():
        raise ValueError('machine identity OS mismatch')
    return {'id': key, 'label': machine['label'], 'os': machine['os'], 'file': file_state(path)}


def layer(value):
    if not isinstance(value, dict) or set(value) - FIELDS:
        raise ValueError('unsupported logical metadata layer')
    return value


def compile_entries(logical, definition, machine_id=None):
    from intent import metadata, source_name, relative
    result = {}
    for system, destination in sorted(definition['destinations'].items()):
        values = dict(portability='managed-shared', impact='Safe', tools=[], removals=[], hooks=[])
        values.update(definition['common'])
        if machine_id is not None:
            values.update(definition['machine'].get(machine_id, {}))
        values.update(definition['os_metadata'].get(system, {}))
        source = definition['sources'][system]
        relative(destination)
        result[logical + '@' + system] = metadata(dict(values, destination=destination, source=source, os=[system]))
    return result


def validate(data):
    from intent import metadata, relative
    if not isinstance(data['machines'], dict) or not data['machines'] or not isinstance(data['tracking'], dict):
        raise ValueError('invalid private machine/tracking catalog')
    for key, machine in data['machines'].items():
        identifier(key)
        if (not isinstance(machine, dict) or set(machine) != {'label', 'os'} or
                machine['os'] not in SYSTEMS or not isinstance(machine['label'], str) or not machine['label'] or
                any(ord(c) < 32 or ord(c) == 127 for c in machine['label'])):
            raise ValueError('invalid private machine declaration')
    owned = set()
    for logical, definition in data['tracking'].items():
        identifier(logical)
        if not isinstance(definition, dict) or set(definition) != {'destinations', 'sources', 'common', 'machine', 'os_metadata', 'machines'}:
            raise ValueError('unsupported logical file fields')
        destinations = definition['destinations']
        if not isinstance(destinations, dict) or not destinations or set(destinations) - SYSTEMS:
            raise ValueError('unsupported OS mapping')
        if not isinstance(definition['sources'], dict) or set(definition['sources']) != set(destinations):
            raise ValueError('missing exact OS source mapping')
        layer(definition['common'])
        for field, supported in (('machine', data['machines']), ('os_metadata', destinations)):
            if not isinstance(definition[field], dict) or set(definition[field]) - set(supported):
                raise ValueError('unknown machine/OS metadata override')
            for value in definition[field].values():
                layer(value)
        members = definition['machines']
        if not isinstance(members, list) or any(m not in data['machines'] for m in members) or len(set(members)) != len(members):
            raise ValueError('unknown machine applicability')
        compiled = compile_entries(logical, definition)
        for key, entry in compiled.items():
            if data['entries'].get(key) != entry or key in owned:
                raise ValueError('logical/physical source metadata conflict')
            owned.add(key)
        # Validate every override, not only the executing machine's layer.
        for machine in data['machines']:
            compile_entries(logical, definition, machine)


def entries(data, home):
    if data['version'] == 1:
        return data['entries']
    machine = identity(home, data)
    result = dict(data['entries'])
    for logical, definition in data['tracking'].items():
        result.update(compile_entries(logical, definition, machine['id']))
    return result


def applicable(data, home, key):
    if data['version'] == 1:
        return True
    machine = identity(home, data)
    for logical, definition in data['tracking'].items():
        if key in compile_entries(logical, definition):
            return not definition['machines'] or machine['id'] in definition['machines']
    return True


def check_scope(data, home, state):
    if state is None:
        return
    if state['version'] == 3:
        if data['version'] != 2:
            raise ValueError('machine-bound scope requires manifest v2')
        machine = identity(home, data)
        if state['machine'] != {'id': machine['id'], 'os': machine['os']}:
            raise ValueError('approved machine identity changed; fresh adoption required')
    elif data['version'] == 2 and any('@' in key and key.split('@')[0] in data['tracking'] for key in state.get('entries', {})):
        raise ValueError('logical mappings require fresh machine-bound adoption approval')


def approved(data, home, profile, selected):
    if data['version'] == 1:
        return {'version': 2, 'profile': profile, 'entries': selected}
    machine = identity(home, data)
    return {'version': 3, 'profile': profile, 'entries': selected, 'machine': {'id': machine['id'], 'os': machine['os']}}


def inventory(data, home, state):
    check_scope(data, home, state)
    current = current_os()
    resolved = entries(data, home)
    rows = []
    for key, entry in sorted(resolved.items()):
        if current not in entry['os'] or not applicable(data, home, key):
            continue
        row = {'id': key, 'metadata': entry,
               'approved': bool(state and state.get('version') in (2, 3) and state['entries'].get(key) == entry)}
        for logical, definition in data.get('tracking', {}).items():
            if key == logical + '@' + current:
                row.update(logical_id=logical, tracking=definition)
        rows.append(row)
    return {'version': 1, 'os': current, 'machine': identity(home, data) if data['version'] == 2 else None, 'entries': rows}


def track(root, home, data, selection, selected, inputs, actions):
    import intent
    if data['version'] != 2:
        raise ValueError('logical tracking requires reviewed manifest v2 provisioning')
    allowed = {'path', 'logical', 'destinations', 'common', 'machine', 'os_metadata', 'machines', 'template'}
    if not isinstance(selection, dict) or set(selection) - allowed:
        raise ValueError('unsupported logical tracking selection')
    logical = identifier(selection.get('logical'))
    machine = identity(home, data)
    old = data['tracking'].get(logical)
    destination_map = selection.get('destinations')
    if not isinstance(destination_map, dict) or machine['os'] not in destination_map or set(destination_map) - SYSTEMS:
        raise ValueError('unsupported/missing current OS destination mapping')
    live = intent.bounded(home, selection['path'])
    inputs[selection['path']] = intent.file_state(live)
    if inputs[selection['path']] is None:
        raise ValueError('selected file does not exist')
    template = 'template' in selection
    try:
        raw = intent.read_text(live)
    except ValueError as error:
        if not template or not str(error).startswith('credential content detected'):
            raise
        raw = None
    text = intent.template_text(selection['template']) if template else raw
    common = layer(selection.get('common', old['common'] if old else {}))
    if common.get('portability', 'managed-shared') == 'managed-templated' and not template:
        raise ValueError('templated imports require an explicitly reviewed secret-free template')
    mode = inputs[selection['path']]['mode']
    definition = {'destinations': destination_map,
                  'sources': {system: intent.source_name(dest, template, bool(mode & 0o111), template or not bool(mode & 0o044)) for system, dest in destination_map.items()},
                  'common': common, 'machine': selection.get('machine', old['machine'] if old else {}),
                  'os_metadata': selection.get('os_metadata', old['os_metadata'] if old else {}),
                  'machines': selection.get('machines', old['machines'] if old else [])}
    existing = set()
    if old:
        if old['machines'] and machine['id'] not in old['machines']:
            raise ValueError('entry is not applicable to this explicit machine')
        if (not set(old['destinations']) < set(destination_map) or
                any(destination_map.get(system) != dest for system, dest in old['destinations'].items()) or
                any(definition[field] != old[field] for field in ('common', 'machine', 'machines')) or
                any(definition['os_metadata'].get(system) != old['os_metadata'].get(system) for system in old['destinations'])):
            raise ValueError('existing logical tracking requires a metadata plan, not a raw re-add')
        definition['sources'].update(old['sources'])
        existing = set(compile_entries(logical, old))
    data['tracking'][logical] = definition
    compiled = compile_entries(logical, definition)
    for key, entry in compiled.items():
        if key in existing:
            if data['entries'][key] != entry:
                raise ValueError('existing logical metadata/source must remain unchanged during OS enrollment')
            continue
        if key in data['entries'] or intent.bounded(root, entry['source']).exists():
            raise ValueError('logical tracking source collision')
        data['entries'][key] = entry
        actions.append({'path': entry['source'], 'before': None, 'text': text, 'remove': False})
    validate(data)
    selected.update(compile_entries(logical, definition, machine['id']))


def review(data, home, selected):
    if data['version'] != 2:
        return
    machine = identity(home, data)
    print('Explicit machine: ' + machine['label'] + ' [' + machine['id'] + ']')
    print('Metadata precedence: common → explicit machine → OS (whole-field override)')
    for logical, definition in data['tracking'].items():
        if not set(compile_entries(logical, definition)) & set(selected):
            continue
        print('Logical file: ' + logical)
        for system, destination in sorted(definition['destinations'].items()):
            print('OS destination: ' + system + ' → ' + destination)
        for field in sorted(FIELDS):
            origin = ('OS' if field in definition['os_metadata'].get(machine['os'], {}) else
                      'explicit machine' if field in definition['machine'].get(machine['id'], {}) else 'common')
            current = compile_entries(logical, definition, machine['id']).get(logical + '@' + machine['os'], {})
            if field in current:
                print('Resolved ' + field + ' [' + origin + ']: ' + json.dumps(current[field]))


def edit(root, home, data, changes, state, actions, selected, inputs):
    """Review declarative metadata and physical source moves, never target moves."""
    import intent
    import scope
    if data['version'] != 2 or not isinstance(changes, dict) or not changes:
        raise ValueError('metadata edits require a private v2 logical file catalog')
    if state and state['version'] not in (2, 3):
        raise ValueError('metadata edits require exact local scope')
    if intent.git(root, 'status', '--porcelain'):
        raise ValueError('metadata edits require a clean source checkout')
    machine = identity(home, data)
    before_entries = entries(data, home)
    affected = set()
    for logical, change in changes.items():
        if logical not in data['tracking'] or not isinstance(change, dict) or set(change) - {'destinations', 'common', 'machine', 'os_metadata', 'machines'}:
            raise ValueError('unsupported logical metadata edit')
        old = data['tracking'][logical]
        if old['machines'] and machine['id'] not in old['machines']:
            raise ValueError('entry is not applicable to this explicit machine')
        old_entries = compile_entries(logical, old)
        updated = dict(old, **change)
        updated['sources'] = {}
        if not isinstance(updated['destinations'], dict) or not updated['destinations'] or set(updated['destinations']) - SYSTEMS:
            raise ValueError('unsupported OS mapping')
        for system, destination in updated['destinations'].items():
            previous = old_entries.get(logical + '@' + system)
            if previous is None:
                raise ValueError('new OS mappings require explicit Track selection')
            source = intent.bounded(root, previous['source'])
            attrs = scope.attributes(source.name)[0]
            updated['sources'][system] = intent.source_name(destination, scope.templated(previous['source']), 'executable' in attrs, 'private' in attrs)
        after_entries = compile_entries(logical, updated)
        affected.update(old_entries)
        for key, previous in old_entries.items():
            data['entries'].pop(key)
            inputs[previous['destination']] = intent.file_state(intent.bounded(home, previous['destination']))
            following = after_entries.get(key)
            if following is None or following['source'] != previous['source']:
                source = intent.bounded(root, previous['source'])
                text = intent.read_text(source)
                if scope.templated(previous['source']):
                    intent.template_text(text)
                actions.append({'path': previous['source'], 'repository': True, 'before': intent.file_state(source), 'text': None, 'remove': True})
                if following:
                    target = intent.bounded(root, following['source'])
                    if target.exists():
                        raise ValueError('metadata source mapping collision')
                    actions.append({'path': following['source'], 'repository': True, 'before': None, 'text': text, 'remove': False, 'mode': source.stat().st_mode & 0o777})
            if following:
                inputs[following['destination']] = intent.file_state(intent.bounded(home, following['destination']))
        data['tracking'][logical] = updated
        data['entries'].update(after_entries)
    for name, members in data['profiles'].items():
        data['profiles'][name] = [i for i in members if i in data['entries']]
    intent.validate_manifest_data(data)
    after_entries = entries(data, home)
    selected.update({k: e for k, e in after_entries.items() if k in affected})
    # Removing applicability also requires reviewing the former capabilities.
    selected.update({k: before_entries[k] for k in affected if k not in selected})
    after_scope = json.loads(json.dumps(state)) if state else None
    if after_scope:
        for key in set(after_scope['entries']) & affected:
            if key in after_entries and applicable(data, home, key):
                after_scope['entries'][key] = after_entries[key]
            else:
                after_scope['entries'].pop(key)
    actions.append({'path': intent.MANIFEST_RELATIVE, 'repository': True,
                    'before': intent.file_state(intent.bounded(root, intent.MANIFEST_RELATIVE)),
                    'text': json.dumps(data, indent=2, sort_keys=True) + '\n', 'remove': False})
    return after_scope
