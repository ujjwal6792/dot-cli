#!/usr/bin/env python3
"""Private package data. Never evaluate Brewfile Ruby or invoke a shell."""
import re
import hashlib
import os
from pathlib import Path
import tempfile
import json
import shutil
import subprocess
import sys


SOURCES = {'brew': 'Brewfile', 'pacman': 'pacman.txt', 'yay': 'yay.txt', 'apt': 'apt.txt'}


def digest(value):
    return hashlib.sha256(value.encode()).hexdigest()


def detect_platform():
    if sys.platform == 'darwin':
        return 'darwin'
    if sys.platform.startswith('linux'):
        fields = {}
        try:
            for line in Path('/etc/os-release').read_text().splitlines():
                key, separator, value = line.partition('=')
                if separator:
                    fields[key] = value.strip('"')
        except OSError:
            pass
        if fields.get('ID') in ('ubuntu', 'arch'):
            return fields['ID']
    raise ValueError('packages: unsupported OS; supported: macOS, Arch, Ubuntu')


def run(argv, mutate=False):
    env = dict(os.environ, LC_ALL='C', HOMEBREW_NO_AUTO_UPDATE='1', HOMEBREW_NO_INSTALL_CLEANUP='1',
               HOMEBREW_NO_INSTALLED_DEPENDENTS_CHECK='1', HOMEBREW_NO_INSTALL_UPGRADE='1')
    try:
        if mutate:
            # JSON bridge input is not the interactive manager input. Never let
            # a pipe/EOF implicitly answer manager/build/privilege questions.
            terminal = os.open('/dev/tty', os.O_RDWR)
            try:
                result = subprocess.run(argv, env=env, check=True, text=True, stdin=terminal,
                                        stdout=terminal, stderr=terminal)
            finally:
                os.close(terminal)
        else:
            result = subprocess.run(argv, env=env, check=True, text=True, stdin=subprocess.DEVNULL,
                                    stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=60)
    except (OSError, subprocess.SubprocessError):
        raise ValueError('packages: manager failed; inspect manager availability and permissions') from None
    return result.stdout or ''


class Planner:
    """Reusable init/browser boundary; sources are always private checkout data."""
    def __init__(self, source, platform=None, runner=None, which=None, uid=None):
        self.source = Path(source).resolve(strict=True)
        self.platform = platform
        self.runner = runner or run
        self.which = which or shutil.which
        self.uid = os.geteuid() if uid is None else uid

    def executable(self, name):
        value = self.which(name)
        if not value:
            raise ValueError(f'packages: required manager {name} is missing; install it explicitly before retrying')
        return str(Path(value).absolute())

    def identity(self, binary):
        checksum = None
        path = Path(binary)
        # Injected runners may use fictional paths. Real executable detection
        # always supplies a file; bind its bytes as well as its reported version.
        if path.is_file():
            hasher = hashlib.sha256()
            with path.open('rb') as stream:
                for chunk in iter(lambda: stream.read(65536), b''):
                    hasher.update(chunk)
            checksum = hasher.hexdigest()
        return {'path': binary, 'version': self.runner([binary, '--version']), 'sha256': checksum}

    def managers(self):
        platform = self.platform or detect_platform()
        if platform not in ('darwin', 'arch', 'ubuntu'):
            raise ValueError('packages: unsupported OS; supported: macOS, Arch, Ubuntu')
        return platform, {'darwin': ['brew'], 'arch': ['pacman', 'yay'], 'ubuntu': ['apt']}[platform]

    def status(self):
        platform, managers = self.managers()
        sources, installed, identities = {}, {}, {}
        for manager in managers:
            sources[manager] = self.load(manager)
            # AUR is a separate desired source but installed through pacman's database.
            if manager == 'yay':
                installed[manager] = installed['pacman']
                continue
            binary = self.executable({'apt': 'dpkg-query'}.get(manager, manager))
            identities[manager] = self.identity(binary)
            if manager == 'brew':
                values = []
                for kind, flag in [('brew', '--formula'), ('cask', '--cask')]:
                    values.extend((kind, name) for name in self.runner([binary, 'list', flag, '--full-name']).splitlines())
            elif manager == 'pacman':
                values = [(manager, name) for name in self.runner([binary, '-Qq']).splitlines()]
            else:
                dpkg = self.executable('dpkg')
                identities['dpkg'] = self.identity(dpkg)
                architecture = self.runner([dpkg, '--print-architecture']).strip()
                if not NAME.fullmatch(architecture):
                    raise ValueError('packages: invalid architecture response; retry manager query')
                identities['dpkg']['architecture'] = architecture
                output = self.runner([binary, '-W', '-f=${binary:Package}\t${db:Status-Status}\n'])
                values = []
                for line in output.splitlines():
                    name, separator, state = line.partition('\t')
                    if not separator:
                        raise ValueError('packages: invalid installed status response; retry manager query')
                    if state == 'installed':
                        if name.endswith(':' + architecture):
                            name = name.rsplit(':', 1)[0]
                        values.append((manager, name))
            installed[manager] = sorted(set(values))
        rows = []
        for manager in managers:
            actual = set(installed[manager])
            desired = sources[manager]['records']
            wanted = set()
            for record in desired:
                kind = 'pacman' if manager == 'yay' else record['kind']
                name = record['name']
                normalized = name
                if name.startswith(('homebrew/core/', 'homebrew/cask/')):
                    normalized = name.rsplit('/', 1)[-1]
                matches = {(kind, normalized)} & actual
                if '/' not in name and not matches:
                    candidates = {(k, n) for k, n in actual if k == kind and n.rsplit('/', 1)[-1] == name}
                    if len(candidates) == 1:
                        matches = candidates
                wanted |= matches
                present = bool(matches)
                rows.append(dict(record, desired=True, installed=present, missing=not present))
            if manager != 'yay':
                if manager == 'pacman':
                    wanted |= {('pacman', r['name']) for r in sources.get('yay', {}).get('records', [])}
                for kind, name in sorted(actual - wanted):
                    rows.append(dict(manager=manager, kind=kind, name=name, desired=False, installed=True, missing=False))
        return {'platform': platform, 'sources': sources, 'installed': installed,
                'identities': identities, 'rows': sorted(rows, key=lambda r: (r['manager'], r['kind'], r['name']))}

    def overview(self):
        """Editing stays available when status is unavailable, never guess missing."""
        platform, managers = self.managers()
        sources = {manager: self.load(manager) for manager in managers}
        try:
            return self.status()
        except ValueError as error:
            rows = [dict(record, desired=True, installed=False, missing=False, status='unavailable')
                    for source in sources.values() for record in source['records']]
            return {'platform': platform, 'sources': sources, 'rows': rows, 'notice': str(error)}

    def plan(self, entrypoint):
        if entrypoint not in ('browse', 'init'):
            raise ValueError('packages: only browse and init can review installation')
        state = self.status()
        if any(s['diagnostics'] for s in state['sources'].values()):
            raise ValueError('packages: unsupported source syntax; edit indicated lines before installing')
        if state['platform'] == 'arch' and (set(state['sources']['pacman']['names']) & set(state['sources']['yay']['names'])):
            raise ValueError('packages: conflicting pacman/AUR sources; assign each name to one manager')
        missing = [r for r in state['rows'] if r['missing']]
        commands, resolution, executables = [], {}, {}
        for manager in state['sources']:
            targets = [r for r in missing if r['manager'] == manager]
            if not targets:
                continue
            binary = self.executable({'apt': 'apt-get'}.get(manager, manager))
            executables[manager] = self.identity(binary)
            if manager == 'brew':
                if self.uid == 0:
                    raise ValueError('packages: Homebrew must run as an unprivileged user')
                for kind, flag in [('brew', '--formula'), ('cask', '--cask')]:
                    names = [r['name'] for r in targets if r['kind'] == kind]
                    if names:
                        resolution[manager + '/' + kind] = self.runner([binary, 'install', '--dry-run', flag, '--', *names])
                        commands.append([binary, 'install', flag, '--', *names])
            elif manager == 'yay':
                if self.uid == 0:
                    raise ValueError('packages: AUR builds must run as an unprivileged user')
                # Explicit AUR-only targets; preserve helper's build-script review prompts.
                executables['sudo'] = self.identity(self.executable('sudo'))
                resolution['yay'] = self.runner([binary, '-Si', '--aur', '--', *[r['name'] for r in targets]])
                commands.append([binary, '-S', '--aur', '--needed', '--', *[r['name'] for r in targets]])
            else:
                names = [r['name'] for r in targets]
                prefix = [] if self.uid == 0 else [self.executable('sudo')]
                if prefix:
                    executables['sudo'] = self.identity(prefix[0])
                argv = [binary, 'install', '--no-remove', '--', *names] if manager == 'apt' else [binary, '-S', '--needed', '--', *names]
                if manager == 'apt':
                    simulation = self.runner([binary, '-s', 'install', '--no-remove', '--', *names])
                    if any(line.startswith('Remv ') or re.match(r'Inst \S+ \[', line) for line in simulation.splitlines()):
                        raise ValueError('packages: simulation proposes removals/upgrades; review separately outside Install missing')
                    resolution[manager] = simulation
                else:
                    resolution[manager] = self.runner([binary, '-Sp', '--needed', '--print-format', '%n %v', '--', *names])
                commands.append(prefix + argv)
        result = {'version': 1, 'entrypoint': entrypoint, 'platform': state['platform'],
                  'uid': self.uid, 'source': str(self.source),
                  'environment': digest(json.dumps(dict(os.environ), sort_keys=True)),
                  'sources': {m: {'digest': s['digest'], 'exists': s['exists']} for m, s in state['sources'].items()},
                  'installed': state['installed'], 'identities': state['identities'],
                  'executables': executables, 'missing': missing, 'commands': commands,
                  'resolution': resolution}
        # JSON roundtrip makes the public plan stable across transport and immutable comparison.
        result = json.loads(json.dumps(result, sort_keys=True))
        result['id'] = digest(json.dumps(result, sort_keys=True))
        return result

    def apply(self, plan, approve, entrypoint, ack_aur=False):
        if not isinstance(plan, dict) or not approve or approve != plan.get('id'):
            raise ValueError('packages: approval of the exact plan ID required')
        if entrypoint != plan.get('entrypoint'):
            raise ValueError('packages: approval belongs to a different entrypoint')
        original = {key: value for key, value in plan.items() if key != 'id'}
        if digest(json.dumps(original, sort_keys=True)) != plan['id']:
            raise ValueError('packages: changed plan; review a new plan')
        if self.plan(entrypoint) != plan:
            raise ValueError('packages: stale plan; review a new plan')
        if any(r['manager'] == 'yay' for r in plan['missing']) and ack_aur is not True:
            raise ValueError('packages: separate AUR build-script risk acknowledgment required')
        for argv in plan['commands']:
            self.runner(argv, mutate=True)
        return {'applied': True, 'plan_id': plan['id']}

    def review(self, plan):
        import shlex
        lines = ['INSTALL MISSING · ' + plan['entrypoint'], 'Plan ID: ' + plan['id'],
                 'Private lists are not changed by installation. Deselection never uninstalls.',
                 'Only the targets below are requested. Managers may add dependencies.',
                 'Manager prompts remain enabled; inspect their transaction before confirming.',
                 'Freshness is rechecked immediately before execution, not a repository lock.']
        if not plan['commands']:
            lines.append('Nothing missing. No installation will run.')
        for record in plan['missing']:
            lines.append(f"{record['manager']}/{record['kind']}: {record['name']}")
        for argv in plan['commands']:
            lines.append('Command: ' + shlex.join(argv))
        for manager, output in plan['resolution'].items():
            label = 'AUR metadata (not a locked build transaction):' if manager == 'yay' else manager + ' transaction simulation:'
            lines += [label, output]
        if any(r['kind'] == 'cask' for r in plan['missing']):
            lines.append('Casks may run installer scripts and request administrator authorization; inspect Homebrew prompts.')
        if any(r['manager'] == 'yay' for r in plan['missing']):
            lines += ['AUR: user-contributed build scripts execute as your unprivileged user.',
                      'Inspect PKGBUILDs and dependency scripts in yay prompts; approval is not a security audit.',
                      'sudo may authorize only the helper/package-manager installation transaction.']
        if plan['platform'] == 'arch':
            lines.append('Arch: no -Syu/system upgrade is requested. Keep repositories/system synchronized separately; partial upgrades are unsupported.')
        if plan['platform'] in ('arch', 'ubuntu') and plan['commands']:
            lines.append('Privilege: the displayed sudo command or manager authorization may request root installation permission.')
        return '\n'.join(lines)

    def path(self, manager):
        if manager not in SOURCES:
            raise ValueError('packages: unsupported manager')
        folder = self.source / 'packages'
        path = folder / SOURCES[manager]
        if folder.is_symlink() or path.is_symlink() or path.resolve() != path:
            raise ValueError('packages: private source must not be symlinked')
        return path

    def load(self, manager):
        path = self.path(manager)
        if path.exists() and not path.is_file():
            raise ValueError('packages: private source must be a regular file')
        if path.exists() and path.stat().st_size > 1024 * 1024:
            raise ValueError('packages: source is too large or binary')
        try:
            text = path.read_bytes().decode('utf-8') if path.exists() else ''
        except UnicodeError:
            raise ValueError('packages: source must be UTF-8 text') from None
        if len(text.encode()) > 1024 * 1024 or '\x00' in text:
            raise ValueError('packages: source is too large or binary')
        # No raw errors or source lines may escape to logs/public diagnostics.
        result = parse(manager, text)
        return dict(result, manager=manager, digest=digest(text), exists=path.exists())

    def import_file(self, manager, expected, supplied):
        if (not isinstance(supplied, str) or not supplied or Path(supplied).is_absolute()
                or any(part in ('', '.', '..', '.git') for part in supplied.split('/'))):
            raise ValueError('packages: import must be a regular file inside the private checkout')
        path = self.source / supplied
        if path.resolve() != path or not path.is_file() or path.stat().st_size > 1024 * 1024:
            raise ValueError('packages: import must be a regular file inside the private checkout')
        try:
            text = path.read_bytes().decode('utf-8')
        except UnicodeError:
            raise ValueError('packages: source must be UTF-8 text') from None
        return self.save(manager, expected, text)

    def save(self, manager, expected, text):
        if not isinstance(text, str) or len(text.encode()) > 1024 * 1024 or '\x00' in text:
            raise ValueError('packages: source is too large or binary')
        old = self.load(manager)
        if expected != old['digest']:
            raise ValueError('packages: stale source; reload before saving')
        path = self.path(manager)
        path.parent.mkdir(mode=0o700, exist_ok=True)
        # Preserve unsupported syntax verbatim. It remains install-blocking.
        # All edits are explicit whole-document saves, not destructive reserialization.
        backup = path.parent / '.backups'
        if backup.is_symlink():
            raise ValueError('packages: unsafe backup directory')
        backup.mkdir(mode=0o700, exist_ok=True)
        if old['exists']:
            fd, name = tempfile.mkstemp(prefix=path.name + '.', dir=backup)
            with os.fdopen(fd, 'wb') as stream:
                stream.write(old['text'].encode())
                stream.flush()
                os.fsync(stream.fileno())
        fd, name = tempfile.mkstemp(prefix='.package-edit-', dir=path.parent)
        try:
            with os.fdopen(fd, 'wb') as stream:
                stream.write(text.encode())
                stream.flush()
                os.fsync(stream.fileno())
            if self.load(manager)['digest'] != expected:
                raise ValueError('packages: stale source; reload before saving')
            os.replace(name, path)
        finally:
            if os.path.exists(name):
                os.unlink(name)
        return self.load(manager)


NAME = re.compile(r'[a-z0-9][a-z0-9+_.-]*\Z')
BREW_NAME = re.compile(r'[a-z0-9][a-z0-9+_-]*(?:@[0-9][a-z0-9+_.-]*)?\Z')
BREW = re.compile(r'''\s*(brew|cask)\s+(["'])([a-z0-9][a-z0-9+_.@/-]*)\2\s*(?:#.*)?\Z''')


def parse(manager, text):
    records, diagnostics = [], []
    seen = set()
    for number, line in enumerate(text.splitlines(), 1):
        if not line.strip() or line.lstrip().startswith('#'):
            continue
        match = BREW.fullmatch(line) if manager == 'brew' else None
        name = match[3] if match else line.split('#', 1)[0].strip()
        kind = match[1] if match else manager
        valid = bool(match) if manager == 'brew' else bool(NAME.fullmatch(name))
        if valid and manager == 'brew':
            parts = name.split('/')
            valid = (len(parts) in (1, 3) and all(NAME.fullmatch(part) for part in parts[:-1])
                     and bool(BREW_NAME.fullmatch(parts[-1])) and not name.endswith(('.rb', '.json')))
        if not valid or (kind, name) in seen:
            diagnostics.append(f'line {number}: unsupported syntax; install blocked')
            continue
        seen.add((kind, name))
        records.append({'manager': manager, 'kind': kind, 'name': name})
    return {'text': text, 'names': [r['name'] for r in records],
            'records': records, 'diagnostics': diagnostics}
