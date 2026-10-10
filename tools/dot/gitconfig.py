#!/usr/bin/env python3
"""Bounded Git key edits. Never resolve includes or execute configured commands."""
import base64
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile

import scope

WRAPPER = '''#!/bin/sh
exec python3 -B {{ joinPath .chezmoi.sourceDir ".." "tools/dot/gitconfig.py" | shellQuote }} modify {{ .chezmoi.sourceDir | shellQuote }} {{ .chezmoi.homeDir | shellQuote }} {{ .chezmoi.configFile | shellQuote }} {{ .chezmoi.os | shellQuote }} {{ .chezmoi.args | toJson | b64enc }}
'''
APPROVED_KEYS = {'user.name', 'user.email'}


def validate(entry):
    from intent import relative
    keys = entry.get('git_keys')
    if (entry['destination'] != '.gitconfig' or entry['impact'] != 'Unsafe' or
            not entry['source'].startswith('home/modify_') or
            not isinstance(keys, list) or not keys or len(set(keys)) != len(keys) or
            any(not isinstance(k, str) or not re.fullmatch(r'[a-z][a-z0-9-]*\.[a-z][a-z0-9-]*', k) for k in keys)):
        raise ValueError('Git preservation requires explicit unambiguous keys and Unsafe .gitconfig modify source')
    if set(keys) - APPROVED_KEYS:
        raise ValueError('only reviewed Git keys user.name/user.email are supported; extensions require code review')
    relative(entry.get('git_source'))
    if not entry['git_source'].startswith('home/.chezmoitemplates/'):
        raise ValueError('Git preferences must be an explicit non-managed template source')


def command(path, *args):
    # --file and --no-includes read only this exact file, never global includes.
    result = subprocess.run(['git', 'config', '--no-includes', '--file', str(path), *args],
                            capture_output=True)
    if result.returncode:
        raise ValueError('unsupported/ambiguous Git configuration; no edits applied')
    return result.stdout


def values(path):
    result = {}
    for record in command(path, '--null', '--list').decode('utf-8').split('\0'):
        if not record:
            continue
        key, _, value = record.partition('\n')
        result.setdefault(key, []).append(value)
    return result


def raw(path):
    return path.read_bytes().decode('utf-8')


def native_operation(args):
    """Normalize documented boolean dry-run spellings at one boundary."""
    # Values such as a config file named "-n" or "apply" are not flags or
    # commands. Unknown option grammar fails closed rather than weakening the
    # permission/disclosure/backup boundary after a chezmoi CLI upgrade.
    value_options = {'--age-recipient', '--age-recipient-file', '--cache', '--config', '--config-format',
                     '--destination', '--mode', '--output', '--override-data', '--override-data-file',
                     '--persistent-state', '--source', '--working-tree', '--include', '--exclude',
                     '--path-style', '--file', '--umask', '--color', '--progress', '-c', '-D', '-o', '-S', '-W', '-i', '-x'}
    boolean_options = {'--debug', '--dry-run', '--error-on-conflict', '--force', '--help', '--interactive',
                       '--keep-going', '--less-interactive', '--no-pager', '--no-tty', '--skip-secrets',
                       '--source-path', '--use-builtin-diff', '--verbose', '--version', '--init',
                       '--recursive', '--refresh-externals', '--use-builtin-age', '--reverse',
                       '--use-builtin-git', '-n', '-v', '-h', '-k', '-r', '-R', '-f'}
    flags, command, position, options, umask_override = [], None, 0, True, False
    while position < len(args):
        arg = args[position]
        position += 1
        if position == 1 and Path(arg).name == 'chezmoi':
            continue
        if options and arg == '--':
            options = False
            continue
        if options and arg.startswith('-'):
            option = arg.split('=', 1)[0]
            if option in value_options:
                if option == '--umask':
                    umask_override = True
                if '=' not in arg:
                    if position == len(args):
                        raise ValueError('missing native option value')
                    position += 1
                continue
            if len(arg) > 2 and arg[:2] in value_options and not arg.startswith('--'):
                continue  # Attached short-option value, e.g. -cFILE.
            if option not in boolean_options and not re.fullmatch(r'-[fnvhkrR]+', arg):
                raise ValueError('unsupported native option grammar; no target edits applied')
            if '=' in arg:
                value = arg.split('=', 1)[1]
                if value in ('1', 't', 'T', 'true', 'TRUE', 'True'):
                    arg = option + '=true'
                elif value in ('0', 'f', 'F', 'false', 'FALSE', 'False'):
                    arg = option + '=false'
                else:
                    raise ValueError('unsupported native boolean value; no target edits applied')
            if re.fullmatch(r'-[fnvhkrR]+', arg):
                flags.extend('-' + char for char in arg[1:])
            else:
                flags.append(arg)
        elif command is None:
            command = arg
    dry_run = False
    verbose = False
    debug = False
    for arg in flags:
        if arg in ('-n', '--dry-run', '--dry-run=true', '-n=true') or re.fullmatch(r'-[fnv]*n[fnv]*', arg):
            dry_run = True
        elif arg in ('--dry-run=false', '-n=false'):
            dry_run = False
        elif arg.startswith(('--dry-run=', '-n=')):
            raise ValueError('unsupported native dry-run flag')
        if arg in ('-v', '--verbose', '--verbose=true', '-v=true') or re.fullmatch(r'-[fnv]*v[fnv]*', arg):
            verbose = True
        elif arg in ('--verbose=false', '-v=false'):
            verbose = False
        if arg in ('--debug', '--debug=true'):
            debug = True
        elif arg == '--debug=false':
            debug = False
    return {'preview': command == 'diff' or dry_run or verbose or debug,
            'mutating': command in ('apply', 'update', 'init') and not dry_run,
            'command': command, 'umask_override': umask_override}


def merge(text, patch):
    # Git's own parser/writer retains unrelated lines. Restrict unsupported
    # continuation forms rather than attempting a second Git grammar.
    if re.search(r'[\x00-\x08\x0b-\x1f\x7f]', text) or re.search(r'\\\n', text):
        raise ValueError('unsupported/ambiguous Git continuation or binary content')
    with tempfile.TemporaryDirectory(prefix='dot-git-config-') as temporary:
        path = Path(temporary) / 'config'
        path.write_text(text)
        before = values(path)
        for key, value in patch.items():
            if len(before.get(key, [])) > 1:
                raise ValueError('ambiguous repeated selected Git key: ' + key)
            if before.get(key) != [value]:
                command(path, '--replace-all', key, value)
        after = values(path)
        if any(after.get(k) != v for k, v in before.items() if k not in patch):
            raise ValueError('unselected Git settings changed; no edits applied')
        return raw(path), {k: before.get(k, []) for k in patch}


SPLIT_WRAPPER = '''#!/bin/sh
{{ $engine := env "DOT_ENGINE" -}}
{{ if hasKey . "dotEngine" -}}
{{ $engine = .dotEngine -}}
{{ if not $engine -}}{{ fail "explicit public dotEngine is required; run compatible dot init before applying" }}{{ end -}}
{{ end -}}
{{ if and (not $engine) (lstat (joinPath .chezmoi.homeDir ".local/state/dot/install.json")) -}}
{{ if eq (output "python3" "-B" "-c" "import json,sys; print(json.load(open(sys.argv[1])).get('version') == 2)" (joinPath .chezmoi.homeDir ".local/state/dot/install.json") | trim) "True" -}}
{{ fail "split installation requires explicit public dotEngine; run compatible dot init before applying" }}
{{ end -}}
{{ end -}}
{{ if $engine -}}
python3 -B {{ joinPath $engine "contract.py" | shellQuote }} check --engine {{ $engine | shellQuote }} --source {{ joinPath .chezmoi.sourceDir ".." | shellQuote }} >/dev/null || exit 1
exec python3 -B {{ joinPath $engine "gitconfig.py" | shellQuote }} modify {{ .chezmoi.sourceDir | shellQuote }} {{ .chezmoi.homeDir | shellQuote }} {{ .chezmoi.configFile | shellQuote }} {{ .chezmoi.os | shellQuote }} {{ .chezmoi.args | toJson | b64enc }}
{{ else -}}
exec python3 -B {{ joinPath .chezmoi.sourceDir ".." "tools/dot/gitconfig.py" | shellQuote }} modify {{ .chezmoi.sourceDir | shellQuote }} {{ .chezmoi.homeDir | shellQuote }} {{ .chezmoi.configFile | shellQuote }} {{ .chezmoi.os | shellQuote }} {{ .chezmoi.args | toJson | b64enc }}
{{ end -}}
'''


def preferences(root, home, entry, config=None, os_name=None):
    from intent import bounded, read_text, template_text, scan
    validate(entry)
    wrapper = bounded(root, entry['source'])
    if wrapper.read_text() not in (WRAPPER, SPLIT_WRAPPER):
        raise ValueError('unsupported native Git adapter; only the bounded wrapper is authorized')
    source = bounded(root, entry['git_source'])
    text = read_text(source)
    template_text(text)
    rendered = scope.render(root, str(home), source, config, os_name)
    scan(rendered)
    with tempfile.TemporaryDirectory(prefix='dot-git-preferences-') as temporary:
        path = Path(temporary) / 'config'
        path.write_text(rendered)
        parsed = values(path)
    patch = {}
    for key in entry['git_keys']:
        if len(parsed.get(key, [])) != 1:
            raise ValueError('missing or ambiguous selected Git preference: ' + key)
        scan(parsed[key][0])
        patch[key] = parsed[key][0]
    return patch


def track(root, home, entry, selection):
    """Import reviewed scalar keys only, retaining the bounded entry metadata."""
    from intent import bounded, read_text, template_text, scan
    validate(entry)
    keys = selection.get('git_keys')
    if (not isinstance(keys, list) or not keys or any(not isinstance(k, str) for k in keys) or
            len(set(keys)) != len(keys) or set(keys) - set(entry['git_keys'])):
        raise ValueError('Track Git requires explicitly selected approved git_keys')
    if 'template' in selection:
        raise ValueError('Track Git imports selected keys, not a replacement template')
    for key, value in selection.items():
        if key not in ('path', 'destination', 'git_keys') and value != entry.get(key):
            raise ValueError('Track cannot change or remove bounded Git metadata: ' + key)
    wrapper = bounded(root, entry['source'])
    if wrapper.read_text() not in (WRAPPER, SPLIT_WRAPPER):
        raise ValueError('unsupported native Git adapter; only the bounded wrapper is authorized')
    live = bounded(home, selection['path'])
    text, _ = merge(raw(live), {})  # Validate without following includes.
    with tempfile.TemporaryDirectory(prefix='dot-git-track-') as temporary:
        path = Path(temporary) / 'config'
        path.write_text(text)
        parsed = values(path)
    patch = {}
    for key in keys:
        if len(parsed.get(key, [])) != 1:
            raise ValueError('missing or ambiguous selected Git preference: ' + key)
        value = parsed[key][0]
        scan(value)
        if '{{' in value or '}}' in value:
            raise ValueError('unsupported template delimiters in imported Git value')
        patch[key] = value
    source = bounded(root, entry['git_source'])
    original = read_text(source)
    template_text(original)
    imported, _ = merge(original, patch)
    template_text(imported)
    return imported


def track_entry(data, destination):
    """Apply the same retention guard to exact selections and snapshot members."""
    collision = data['entries'].get(destination)
    if collision and collision['destination'] != destination and 'git-config' in collision.get('integrations', []):
        raise ValueError('Track cannot overwrite a bounded Git manifest ID; select its declared destination and approved keys')
    return next(((key, entry) for key, entry in data['entries'].items() if entry['destination'] == destination and
                 'git-config' in entry.get('integrations', [])), (None, None))


def approved(root, home):
    from intent import manifest
    state = scope.load(str(home))
    if state is None:
        raise ValueError('Git preservation requires exact local approval')
    catalog = manifest(root)
    if state['version'] == 1:
        if not scope.allows(state, '.gitconfig'):
            raise ValueError('Git preservation is outside legacy adoption scope')
        entries = [e for e in catalog['entries'].values() if e['destination'] == '.gitconfig' and
                   'git-config' in e.get('integrations', [])]
        if len(entries) != 1:
            raise ValueError('legacy Git requires the bounded preferences adapter')
        entry = dict(entries[0], git_keys=sorted(APPROVED_KEYS))
        validate(entry)
        if entry['removals'] or entry.get('requires') or entry.get('warnings'):
            raise ValueError('legacy Git cannot acquire new capabilities; review exact adoption first')
        return entry
    entries = [e for k, e in state['entries'].items() if 'git-config' in e.get('integrations', [])
               and catalog['entries'].get(k) == e]
    if len(entries) != 1:
        raise ValueError('Git preservation requires unchanged exact approved metadata')
    return entries[0]


def enabled(root, home):
    from intent import manifest
    state = scope.load(str(home))
    if not state:
        return False
    if state['version'] in (2, 3):
        return any('git-config' in e.get('integrations', []) for e in state['entries'].values())
    return scope.allows(state, '.gitconfig') and any('git-config' in e.get('integrations', [])
                                                   for e in manifest(root)['entries'].values())


def preview(root, home, config):
    from intent import bounded, file_state, scan
    entry = approved(root, home)
    os_name = 'linux' if sys.platform.startswith('linux') else sys.platform
    patch = preferences(root, home, entry, config or None, os_name)
    target = bounded(home, entry['destination'])
    _, old = merge(raw(target) if file_state(target) else '', patch)
    print('Legacy Git key-only preview; stored scope unchanged.')
    for key, value in patch.items():
        try:
            scan('\n'.join(old[key]))
            previous = json.dumps(old[key])
        except ValueError:
            previous = '[existing value withheld]'
        print('Git key ' + key + ': ' + previous + ' → ' + json.dumps(value))


def native_validate(root, home, entry, config, os_name, args, mask):
    from intent import bounded, file_state
    if os_name not in entry['os']:
        raise ValueError('approved Git entry is not applicable to this OS')
    patch = preferences(root, home, entry, config, os_name)
    live = bounded(home, entry['destination'])
    before = file_state(live)
    merge(raw(live) if before else '', patch)
    if before and native_operation(args)['mutating']:
        attrs = scope.attributes(Path(entry['source']).name)[0]
        desired = ((0o600 if 'private' in attrs else 0o666) | (0o111 if 'executable' in attrs else 0)) & ~mask
        current = live.stat().st_mode & 0o7777
        if current != desired:
            raise ValueError(f'unapproved Git permission change: {current:04o} → {desired:04o}; '
                             'existing .gitconfig permissions preserved; use reviewed intent adoption/update')


def native_backup(root, home, config, os_name, args):
    from intent import bounded, file_state, write, scan
    state = scope.load(str(home))
    if not state or (state['version'] == 1 and not scope.allows(state, '.gitconfig')) or (
            state['version'] in (2, 3) and not any('git-config' in e.get('integrations', []) for e in state['entries'].values())):
        return
    entry = approved(root, home)
    target = bounded(home, entry['destination'])
    before = file_state(target)
    text = raw(target) if before else ''
    operation = native_operation(args)
    if operation['preview']:
        try:
            scan(text)
        except ValueError:
            raise ValueError('Native Git diff withheld: existing credential content; use dot intent preview for key-only review') from None
    patch = preferences(root, home, entry, config, os_name)
    merged, _ = merge(text, patch)
    if merged == text or not operation['mutating']:
        return
    recovery = home / '.local/state/dot/native-git-recovery'
    if recovery.resolve() != home.resolve() / recovery.relative_to(home):
        raise ValueError('native Git recovery must not traverse symlinks')
    recovery.mkdir(parents=True, exist_ok=True, mode=0o700)
    recovery.chmod(0o700)
    backup = Path(tempfile.mkdtemp(dir=recovery))
    if before:
        snapshot = backup / 'config'
        snapshot.write_bytes(target.read_bytes())
        snapshot.chmod(0o600)
        if file_state(target) != before:
            raise ValueError('Git target changed during backup')
    write(backup / 'plan.json', json.dumps({'path': entry['destination'], 'before': before, 'keys': entry['git_keys']}) + '\n')


def main():
    from intent import bounded, file_state
    if len(sys.argv) == 5 and sys.argv[1] == 'preview':
        preview(Path(sys.argv[2]), Path(sys.argv[3]), sys.argv[4])
        return
    command_name, source, destination, config, os_name, args_encoded = sys.argv[1:]
    root, home = Path(source).parent, Path(destination)
    if command_name != 'modify':
        raise ValueError('unsupported Git adapter command')
    entry = approved(root, home)
    patch = preferences(root, home, entry, config, os_name)
    text = sys.stdin.read()
    target = bounded(home, '.gitconfig')
    before = file_state(target)
    if text != (raw(target) if target.exists() else ''):
        raise ValueError('Git target changed during native rendering')
    merged, _ = merge(text, patch)
    args = json.loads(base64.b64decode(args_encoded, validate=True))
    if merged != text and native_operation(args)['mutating']:
        recovery = home / '.local/state/dot/native-git-recovery'
        if recovery.resolve() != home.resolve() / recovery.relative_to(home):
            raise ValueError('native Git recovery must not traverse symlinks')
        records = list(recovery.glob('*/plan.json'))
        if not any(record.parent.resolve() == recovery.resolve() / record.parent.name and
                   not record.is_symlink() and record.parent.stat().st_mode & 0o777 == 0o700 and
                   json.loads(record.read_text()).get('before') == before and
                   (before is None or (not (record.parent / 'config').is_symlink() and
                    (record.parent / 'config').stat().st_mode & 0o777 == 0o600 and
                    (record.parent / 'config').read_bytes() == text.encode())) for record in records):
            raise ValueError('native Git target changed after backup; no edits applied')
    sys.stdout.write(merged)


if __name__ == '__main__':
    try:
        main()
    except (ValueError, OSError, UnicodeError) as error:
        print('dot Git preservation: ' + str(error), file=sys.stderr)
        sys.exit(1)
