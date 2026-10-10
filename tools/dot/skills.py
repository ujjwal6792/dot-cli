#!/usr/bin/env python3
"""Bounded skills fan-out: exact approved files, never a directory mirror."""
import json
import os
from pathlib import Path
import sys

CANONICAL = '.agents/skills/'
MIRROR = '.gemini/skills/'
BACKUP = '.agents/skills.bak'
# Native apply may execute only this known adapter, not arbitrary repo hooks.
HOOK = '''#!/bin/sh
# Bounded native adapter; legacy scopes retain their transitional router.
{{ $repo := .chezmoi.sourceDir -}}
{{ if lstat (joinPath $repo ".." "tools/dot/scope.py") -}}
{{ $repo = joinPath $repo ".." -}}
{{ end -}}
{{ $exact := false -}}
{{ if lstat (joinPath .chezmoi.homeDir ".local/state/dot/adoption.json") -}}
{{ $exact = eq (output "python3" "-B" (joinPath $repo "tools/dot/scope.py") "version" $repo .chezmoi.homeDir | trim) "2" -}}
{{ end -}}
{{ if $exact -}}
python3 -B "{{ $repo }}/tools/dot/skills.py" "{{ $repo }}" "{{ .chezmoi.homeDir }}"
{{ else -}}
"{{ $repo }}/scripts/sync-skills.sh"
{{ end -}}
'''


SPLIT_HOOK = '''#!/bin/sh
# Bounded native adapter; legacy scopes retain their transitional router.
{{ $repo := .chezmoi.sourceDir -}}
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
{{ $repo = joinPath .chezmoi.sourceDir ".." -}}
{{ $contract := output "python3" "-B" (joinPath $engine "contract.py") "check" "--engine" $engine "--source" $repo -}}
python3 -B {{ joinPath $engine "contract.py" | shellQuote }} check --engine {{ $engine | shellQuote }} --source {{ $repo | shellQuote }} >/dev/null || exit 1
{{ if eq (output "python3" "-B" (joinPath $engine "scope.py") "skills" $repo .chezmoi.homeDir | trim) "yes" -}}
python3 -B {{ joinPath $engine "skills.py" | shellQuote }} {{ $repo | shellQuote }} {{ .chezmoi.homeDir | shellQuote }}
{{ end -}}
{{ else -}}
{{ if lstat (joinPath $repo ".." "tools/dot/scope.py") -}}
{{ $repo = joinPath $repo ".." -}}
{{ end -}}
{{ $exact := false -}}
{{ if lstat (joinPath .chezmoi.homeDir ".local/state/dot/adoption.json") -}}
{{ $exact = eq (output "python3" "-B" (joinPath $repo "tools/dot/scope.py") "version" $repo .chezmoi.homeDir | trim) "2" -}}
{{ end -}}
{{ if $exact -}}
python3 -B "{{ $repo }}/tools/dot/skills.py" "{{ $repo }}" "{{ .chezmoi.homeDir }}"
{{ else -}}
"{{ $repo }}/scripts/sync-skills.sh"
{{ end -}}
{{ end -}}
'''


def output(entry):
    return MIRROR + entry['destination'][len(CANONICAL):] if 'gemini-skills' in entry.get('integrations', []) else None


def skill_file(name):
    return any(name.startswith(prefix) for prefix in
               (CANONICAL, MIRROR, '.config/opencode/skills/', '.codex/skills/'))


def backup_path(home, suffix=''):
    # Recovery is permanently local-unmanaged; not an eligible manifest path.
    path = home / BACKUP / suffix
    if path.resolve() != home.resolve() / BACKUP / suffix:
        raise ValueError('skills backup must not traverse symlinks')
    return path


def plan_backup(home, actions, mode):
    from intent import file_state, read_text
    files = {}
    for action in actions:
        if action['path'].startswith(CANONICAL) and not action.get('repository') and action['before']:
            read_text(home / action['path'])  # Secrets/runtime never become a cleanup exception.
            if mode != 'update':
                suffix = action['path'][len(CANONICAL):]
                path = backup_path(home, suffix)
                previous = file_state(path)
                if previous:
                    read_text(path)
                files[suffix] = previous
    if not files:
        return None
    directory = backup_path(home)
    if directory.exists() and not directory.is_dir():
        raise ValueError('singleton skills backup is not a directory')
    # Exact slots only: conflict consent cannot delete unrelated backup contents.
    return {'path': BACKUP, 'exists': directory.exists(), 'files': files}


def save_backup(home, value, replace):
    from intent import bounded, write
    record = value.get('skills_backup')
    if not record:
        return
    if record['exists'] and not replace:
        raise ValueError('existing singleton skills backup conflict; explicit --replace-skills-backup required')
    directory = backup_path(home)
    directory.mkdir(parents=True, exist_ok=True, mode=0o700)
    for suffix in record['files']:
        source = bounded(home, CANONICAL + suffix)
        target = backup_path(home, suffix)
        # Create private recovery parents without chmod of existing directories.
        parents = []
        parent = target.parent
        while not parent.exists():
            parents.append(parent)
            parent = parent.parent
        for parent in reversed(parents):
            parent.mkdir(mode=0o700)
        write(target, source.read_text(encoding='utf-8'))
        os.chmod(target, 0o600)


def route(root, home):
    """Approved native content update; rendering cannot enroll future files."""
    import intent
    import scope
    state = scope.load(str(home))
    if state is None or state['version'] not in (2, 3):
        raise ValueError('exact skills routing requires an approved exact scope')
    if not scope.skills_allowed(state):
        raise ValueError('skills routing is outside adoption scope')
    value = intent.plan(root, home, {'version': 1, 'mode': 'update', 'profile': state['profile']})
    actions = [a for a in value['actions'] if a.get('generated')]
    for action in actions:
        path = intent.bounded(home, action['path'])
        if intent.file_state(path) != action['before']:
            raise ValueError('skills output changed during routing; review before retry')
        intent.write(path, action['text'])
        os.chmod(path, action['mode'])
    intent.missing_tools(home, value['selected'])
    print(f'Skills routing: {len(actions)} approved exact files; unapproved/runtime files preserved.')


if __name__ == '__main__':
    try:
        route(Path(sys.argv[1]), Path(sys.argv[2]))
    except (ValueError, KeyError, TypeError, OSError) as error:
        print('skills routing: ' + str(error), file=sys.stderr)
        sys.exit(1)
