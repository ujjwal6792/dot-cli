"""Terminal adapter: existing dot's task-first style, with native vim motions.

Direction and selection stay visible; metadata is explicit, nothing preselected.
Browsing is never authorization. Preview/approval call the CLI's same engine.
"""
import contextlib
import curses
import io
import json
import sys

import intent


def line(screen, row, text, highlight=False):
    height, width = screen.getmaxyx()
    if 0 <= row < height - 1 and width > 1:
        screen.addnstr(row, 0, text, width - 1, curses.A_REVERSE if highlight else curses.A_NORMAL)


def prompt(screen, title, default=''):
    value = default
    while True:
        screen.erase()
        line(screen, 0, title)
        line(screen, 2, value)
        line(screen, 4, 'Enter accepts · Esc cancels · Backspace edits')
        screen.refresh()
        key = screen.get_wch()
        if key == '\x1b':
            return None
        if key in ('\n', '\r', curses.KEY_ENTER):
            return value
        if key in ('\x7f', '\b', curses.KEY_BACKSPACE):
            value = value[:-1]
        elif isinstance(key, str) and key.isprintable():
            value += key


def choose(screen, title, choices):
    index = 0
    while True:
        screen.erase()
        line(screen, 0, title)
        for row, choice in enumerate(choices, 2):
            line(screen, row, ('> ' if row - 2 == index else '  ') + choice, row - 2 == index)
        line(screen, len(choices) + 3, 'j/k move · Enter selects · q cancels')
        screen.refresh()
        key = screen.get_wch()
        if key in ('q', '\x1b'):
            return None
        if key in ('j', curses.KEY_DOWN):
            index = min(len(choices) - 1, index + 1)
        if key in ('k', curses.KEY_UP):
            index = max(0, index - 1)
        if key in ('\n', '\r', curses.KEY_ENTER):
            return choices[index]


def edit(screen, path):
    destination = prompt(screen, 'Destination relative to HOME (explicit file; directory snapshots retain paths)', path)
    if destination is None:
        return None
    portability = choose(screen, 'Portability — independent of impact', ['managed-shared', 'managed-templated', 'managed-os-only'])
    if portability is None:
        return None
    systems = choose(screen, 'Supported operating systems', ['darwin,linux', 'darwin', 'linux'])
    if systems is None:
        return None
    impact = choose(screen, 'Operational impact — Safe is not loss-free', ['Safe', 'Unsafe'])
    if impact is None:
        return None
    value = {'path': path, 'os': systems.split(','), 'portability': portability, 'impact': impact}
    if destination != path:
        value['destination'] = destination
    if portability == 'managed-templated':
        template = prompt(screen, 'Explicit secret-free template (CLI supports multiline templates)')
        if template is None:
            return None
        value['template'] = template + '\n'
    return value


def review(screen, value):
    output = io.StringIO()
    with contextlib.redirect_stdout(output):
        intent.preview(value)
    lines = output.getvalue().splitlines()
    offset = 0
    while True:
        screen.erase()
        height, _ = screen.getmaxyx()
        line(screen, 0, 'Review exact plan — no changes yet')
        for row, text in enumerate(lines[offset:offset + max(1, height - 5)], 2):
            line(screen, row, text)
        line(screen, height - 2, 'j/k scroll · a approve this plan · q cancel')
        screen.refresh()
        key = screen.get_wch()
        if key in ('q', '\x1b'):
            return False
        if key in ('j', curses.KEY_DOWN):
            offset = min(max(0, len(lines) - 1), offset + 1)
        if key in ('k', curses.KEY_UP):
            offset = max(0, offset - 1)
        if key == 'a':
            return choose(screen, 'Apply the reviewed exact plan?', ['Cancel', 'Apply']) == 'Apply'


def capabilities(screen, current):
    value = dict(current)
    for key, title in [('removals', 'Exact file removals (comma separated; no directories)'),
                       ('tools', 'Advisory tools (comma separated; never installed)')]:
        answer = prompt(screen, title, ','.join(value.get(key, [])))
        if answer is None:
            return None
        value[key] = [item.strip() for item in answer.split(',') if item.strip()]
    warning = choose(screen, 'Compatibility warning — separate desktop acknowledgment',
                     ['None', 'Arch/CachyOS desktop; Ubuntu compatibility not promised'])
    if warning is None:
        return None
    value['warnings'] = [] if warning == 'None' else ['desktop-arch-compatibility']
    if 'git-config' in current.get('integrations', []):
        keys = prompt(screen, 'Explicit Git keys (comma separated; all other settings preserved)', ','.join(current['git_keys']))
        if keys is None:
            return None
        value['git_keys'] = [key.strip() for key in keys.split(',') if key.strip()]
    name = current.get('destination', current.get('path', ''))
    if name.startswith('.agents/skills/'):
        routing = choose(screen, 'Generate Gemini output for this exact selected canonical file?', ['No', 'Yes'])
        if routing is None:
            return None
        value['integrations'] = ['gemini-skills'] if routing == 'Yes' else []
    return value


def run(screen, root, home, direction=None):
    with contextlib.suppress(curses.error):
        curses.curs_set(0)
    mode = ('Track: HOME → repository' if direction == 'track' else 'Adopt: repository → HOME') if direction else choose(screen, 'Track / Adopt — distinct directions', ['Track: HOME → repository', 'Adopt: repository → HOME'])
    if mode is None:
        return None
    tracking = mode.startswith('Track')
    profile = prompt(screen, 'Named repository profile')
    if profile is None:
        return None
    directory, index, selections = home, 0, {}
    edits = {}
    catalog = intent.manifest(root)
    message = 'Nothing selected. Credentials/runtime/history are permanently excluded.'
    while True:
        if tracking:
            rows = sorted(directory.iterdir(), key=lambda p: (not p.is_dir(), p.name))
            names = [p.relative_to(home).as_posix() for p in rows]
        else:
            system = 'linux' if sys.platform.startswith('linux') else sys.platform
            names = sorted((k for k, e in catalog['entries'].items() if system in e['os']),
                           key=lambda k: (catalog['entries'][k]['impact'], k))
            rows = []
        index = min(index, max(0, len(names) - 1))
        screen.erase()
        height, _ = screen.getmaxyx()
        line(screen, 0, mode + '  |  profile: ' + profile)
        line(screen, 1, ('HOME/' + str(directory.relative_to(home))) if tracking else 'Repository manifest entries — Safe then Unsafe (applicable OS only)')
        start = max(0, index - max(1, height - 9) + 1)
        for row, name in enumerate(names[start:start + max(1, height - 8)], 3):
            entry = selections.get(name)
            label = ('[x] ' if entry is not None else '[ ] ') + name
            if tracking and rows[start + row - 3].is_dir():
                label += '/'
            if entry is not None and tracking:
                label += '  ' + entry['portability'] + ' / ' + entry['impact']
            elif not tracking:
                label += '  ' + catalog['entries'][name]['portability'] + ' / ' + catalog['entries'][name]['impact']
            line(screen, row, label, start + row - 3 == index)
        line(screen, height - 4, message)
        line(screen, height - 3, f'{len(selections)} selections · j/k move · h/l browse · Space select · e metadata · c capabilities')
        line(screen, height - 2, 'p preview · s opt-in repository preset (Adopt) · q cancel (no changes)')
        screen.refresh()
        key = screen.get_wch()
        try:
            if key in ('q', '\x1b'):
                return None
            if key == 's' and not tracking:
                system = 'linux' if sys.platform.startswith('linux') else sys.platform
                available = [p for p, ids in catalog['profiles'].items() if any(system in catalog['entries'][k]['os'] for k in ids)]
                preset = choose(screen, 'Opt-in exact repository preset — Safe / Unsafe; no apply consent', ['Cancel'] + sorted(available))
                if preset is not None and preset != 'Cancel':
                    system = 'linux' if sys.platform.startswith('linux') else sys.platform
                    ids = [k for k in catalog['profiles'][preset] if system in catalog['entries'][k]['os']]
                    selections.update({k: True for k in ids})
                    message = f'Explicit preset snapshot: {len(ids)} applicable exact entries; future files stay opt-in.'
                continue
            if key in ('j', curses.KEY_DOWN):
                index += 1
            elif key in ('k', curses.KEY_UP):
                index = max(0, index - 1)
            elif key in ('h', curses.KEY_LEFT) and tracking and directory != home:
                directory, index = directory.parent, 0
            elif key in ('l', curses.KEY_RIGHT, '\n', '\r') and tracking and rows:
                path = rows[index]
                # Navigation may browse excluded directories, never symlink escapes.
                if path.is_dir() and not path.is_symlink() and path.resolve().is_relative_to(home.resolve()):
                    directory, index = path, 0
            elif key == 'c' and names:
                name = names[index]
                current = selections.get(name) if tracking else edits.get(name, catalog['entries'][name])
                if current is None:
                    message = 'Select and edit file metadata before capabilities.'
                    continue
                if tracking and 'git_keys' in current:
                    message = 'Git Track retains bounded metadata; e edits keys, Adopt reviews capability edits.'
                    continue
                value = capabilities(screen, current)
                if value is not None:
                    if tracking:
                        selections[name] = value
                    else:
                        fields = ('os', 'impact', 'removals', 'tools', 'warnings', 'integrations', 'git_keys')
                        edits[name] = {k: value[k] for k in fields if k in value}
                        selections[name] = True
            elif key in (' ', 'e') and names:
                name = names[index]
                if key == ' ' and name in selections:
                    del selections[name]
                elif tracking:
                    bounded_git = next((e for e in catalog['entries'].values() if e['destination'] == name and
                                        'git-config' in e.get('integrations', [])), None)
                    if bounded_git:
                        keys = prompt(screen, 'Track only explicitly selected Git keys (user.name,user.email)',
                                      ','.join(selections.get(name, {}).get('git_keys', [])))
                        value = None if keys is None else {'path': name, 'git_keys': [k.strip() for k in keys.split(',') if k.strip()],
                                                          'portability': bounded_git['portability'], 'impact': bounded_git['impact'],
                                                          'os': bounded_git['os']}
                        if value is not None:
                            selections[name] = value
                            message = 'Key-only Git import; bounded metadata retained; target unchanged.'
                        continue
                    value = edit(screen, name)
                    if value is not None:
                        previous = selections.get(name, {})
                        for field in ('removals', 'tools', 'warnings', 'integrations', 'requires', 'notices'):
                            if field in previous:
                                value[field] = previous[field]
                        paths = intent.snapshot(home, value)
                        for path in paths:
                            exact = dict(value, path=path)
                            selections[path] = exact
                        message = f'Snapshotted {len(paths)} eligible exact files; future files remain unselected.'
                else:
                    selections[name] = True
                    if key == 'e':
                        systems = choose(screen, 'Supported operating systems', ['darwin,linux', 'darwin', 'linux'])
                        impact = choose(screen, 'Operational impact', ['Safe', 'Unsafe'])
                        if systems is not None and impact is not None:
                            edited = capabilities(screen, dict(edits.get(name, catalog['entries'][name]), os=systems.split(','), impact=impact))
                            if edited is not None:
                                fields = ('os', 'impact', 'removals', 'tools', 'warnings', 'integrations', 'git_keys')
                                edits[name] = {k: edited[k] for k in fields if k in edited}
                    message = 'Repository metadata edits are included in the exact plan; no changes yet.'
            elif key == 'p':
                request = {'version': 1, 'mode': 'track' if tracking else 'adopt', 'profile': profile}
                request['selections' if tracking else 'entries'] = list(selections.values()) if tracking else sorted(selections)
                if not tracking and edits:
                    request['metadata'] = {k: v for k, v in edits.items() if k in selections}
                value = intent.plan(root, home, request)
                if not review(screen, value):
                    message = 'Preview canceled; nothing applied.'
                    continue
                unsafe = intent.unsafe_review(value)
                if unsafe and prompt(screen, 'Unsafe affects broader environment. Type UNSAFE to acknowledge') != 'UNSAFE':
                    message = 'Unsafe acknowledgment canceled; nothing applied.'
                    continue
                desktop = intent.desktop_review(value)
                if desktop and prompt(screen, 'Compatibility review: type DESKTOP (separate from Unsafe)') != 'DESKTOP':
                    message = 'Desktop acknowledgment canceled; nothing applied.'
                    continue
                return value, unsafe, desktop
        except (ValueError, OSError) as error:
            message = str(error)


def pick(root, home, direction=None):
    if not sys.stdin.isatty() or not sys.stdout.isatty():
        raise ValueError('picker requires a terminal; use intent plan/preview/apply')
    result = curses.wrapper(run, root, home, direction)
    if result is None:
        print('Canceled; nothing applied.')
        return
    value, unsafe, desktop = result
    # Keep output/results visible after leaving the alternate screen.
    intent.preview(value)
    replace = False
    if value.get('skills_backup') and value['skills_backup']['exists']:
        def confirm_backup(screen):
            return prompt(screen, 'Singleton backup conflict: type REPLACE to replace inventoried slots only (unrelated files preserved)') == 'REPLACE'
        replace = curses.wrapper(confirm_backup)
        if not replace:
            print('Backup conflict consent canceled; nothing applied.')
            return
    intent.apply(root, home, value, value['id'], unsafe, desktop, replace)


def rounds(root, home):
    if not sys.stdin.isatty() or not sys.stdout.isatty():
        raise ValueError('optional rounds require a terminal; use intent plan/preview/apply')
    while True:
        selection = curses.wrapper(choose, 'Optional configuration round — installation is complete; nothing preselected',
                                   ['Finish without further adoption', 'Track: HOME → repository', 'Adopt: repository → HOME'])
        if selection is None or selection.startswith('Finish'):
            print('Optional rounds finished; no additional adoption. Prior results remain above.')
            return
        pick(root, home, 'track' if selection.startswith('Track') else 'adopt')
