#!/usr/bin/env python3
"""Snapshot-verified installed split. Retains originals; never applies HOME."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import stat
import subprocess
import sys
import tempfile
import tomllib
sys.dont_write_bytecode = True
import contract
import intent
import layout
import scope


def write(path, text, mode=0o600):
    """Durable atomic references/journal; publish with the final mode already set."""
    fd, temporary = tempfile.mkstemp(dir=path.parent)
    try:
        with os.fdopen(fd, 'w') as stream:
            stream.write(text)
            stream.flush()
            os.fchmod(stream.fileno(), mode)
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        directory = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def inventory(path):
    """No-follow byte/type/mode inventory, including Git metadata and ignored work."""
    result = {}
    def visit(item, name):
        info = item.lstat()
        mode = stat.S_IMODE(info.st_mode)
        identity = [info.st_uid, info.st_gid]
        if stat.S_ISLNK(info.st_mode):
            result[name] = ['link', mode, *identity, os.readlink(item)]
        elif stat.S_ISREG(info.st_mode):
            result[name] = ['file', mode, *identity, hashlib.sha256(item.read_bytes()).hexdigest()]
        elif stat.S_ISDIR(info.st_mode):
            result[name] = ['dir', mode, *identity]
            for child in sorted(item.iterdir()):
                visit(child, name + '/' + child.name)
        else:
            raise ValueError('special file cannot be safely snapshotted: ' + str(item))
    visit(path, '.')
    return result


def git(path):
    if not (path / '.git').is_dir() or (path / '.git').is_symlink():
        raise ValueError('linked worktree/external Git metadata unsupported; preserve common metadata and migrate manually')
    def query(*args):
        return subprocess.check_output(['git', '-c', 'core.hooksPath=/dev/null', '-C', str(path), *args], text=True).strip()
    for option in ('--absolute-git-dir', '--git-common-dir'):
        location = Path(query('rev-parse', option))
        if not location.is_absolute():
            location = path / location
        if location.resolve() != path / '.git':
            raise ValueError('external Git metadata unsupported; preserve common metadata manually')
    if (path / '.git/objects/info/alternates').exists() or (path / '.git/worktrees').exists():
        raise ValueError('shared Git metadata/alternates unsupported; manual migration required')
    if any(item.is_symlink() for item in (path / '.git').rglob('*')):
        raise ValueError('external Git metadata symlink unsupported; manual migration required')
    if subprocess.run(['git', '-C', str(path), 'config', '--local', '--get', 'core.worktree'], stdout=subprocess.DEVNULL).returncode == 0:
        raise ValueError('explicit Git worktree binding unsupported')
    remotes = subprocess.run(['git', '-C', str(path), 'config', '--null', '--get-regexp', r'^remote\..*\.(url|pushurl)$'], capture_output=True, text=True)
    for entry in remotes.stdout.split('\0'):
        if entry:
            url = entry.partition('\n')[2]
            if not (Path(url).is_absolute() or re.match(r'^[A-Za-z][A-Za-z0-9+.-]*://|^[^/]+:', url)):
                raise ValueError('relative Git remote unsupported; manual identity review required')
    origin = subprocess.run(['git', '-C', str(path), 'remote', 'get-url', 'origin'], capture_output=True, text=True)
    return {'head': query('rev-parse', 'HEAD'), 'origin': origin.stdout.strip() if origin.returncode == 0 else None,
            'configuration': hashlib.sha256(query('config', '--null', '--list').encode()).hexdigest()}


def paths(home):
    state = home / '.local/state/dot'
    return {name: layout.private_path(home, path) for name, path in {
        'record': state / 'install.json', 'launcher': home / '.local/bin/dot',
        'journal': state / 'installed-migration.json', 'recovery': state / 'installed-recovery',
        'private': home / '.local/share/dot-config', 'public': home / '.local/share/dot-cli',
        'state': state}.items()}


def launcher(source, config, public=None):
    text = '#!/usr/bin/env bash\nexport DOT_SOURCE=' + shlex.quote(str(source)) + '\n'
    if public:
        text += 'export DOT_ENGINE=' + shlex.quote(str(public / 'tools/dot')) + '\nexport DOT_BROWSER=' + shlex.quote(str(public / 'bin/dot-browser')) + '\n'
    text += 'export DOT_CONFIG=' + shlex.quote(str(config)) + '\nexec ' + shlex.quote(str((public or source) / 'tools/dot/dot')) + ' "$@"\n'
    return text


def proposal(home, source, config, candidate):
    p = paths(home)
    if p['journal'].exists() and json.loads(p['journal'].read_text()).get('stage') != 'complete':
        raise ValueError('incomplete installed migration; inspect ' + str(p['journal']) + '; follow recovery README; no unsafe retry')
    source = layout.private_path(home, source)
    config = layout.private_path(home, config)
    record = json.loads(p['record'].read_text())
    if record.get('version') == 2:
        data = tomllib.loads(config.read_bytes().decode())
        if (source != p['private'] or record.get('directory') != str(source) or
                record.get('engine') != str(p['public']) or record.get('config') != str(config) or
                data.get('sourceDir') != str(source / 'home') or
                data.get('data', {}).get('dotEngine') != str(p['public'] / 'tools/dot') or
                p['launcher'].read_text() != launcher(source, config, p['public'])):
            raise ValueError('split installation reference conflict; explicit recovery required')
        contract.check(p['public'] / 'tools/dot', source)
        return {'version': 1, 'complete': True}
    if record.get('version') != 1 or record.get('directory') != str(source) or record.get('config') != str(config):
        raise ValueError('legacy installed record does not match explicit source/config; development checkout untouched')
    if not source.is_relative_to(home / '.local/share') or source == home / '.local/share':
        raise ValueError('only recorded installed checkout under HOME/.local/share may migrate; development checkout untouched')
    if scope.load(str(home)) is None:
        raise ValueError('explicit existing adoption boundary required')
    if p['launcher'].read_text() != launcher(source, config):
        raise ValueError('unrecognized legacy launcher; review manually without overwriting')
    candidate = Path(os.path.abspath(candidate))
    if candidate.resolve() != candidate:
        raise ValueError('public candidate must not traverse symlinks')
    for a in (source, candidate, p['private'], p['public'], p['state']):
        for b in (source, candidate, p['private'], p['public'], p['state']):
            if a != b and (a in b.parents or b in a.parents):
                raise ValueError('checkout/state paths must be independent and non-nested')
    for destination in (p['private'], p['public']):
        if destination.exists() or destination.is_symlink():
            raise ValueError('destination collision; no overwrite or unsafe retry')
    for marker in ('home', 'dot-contract.json', 'tools/dot/manifest.json', 'tools/dot/groups.json'):
        if (candidate / marker).exists() or (candidate / marker).is_symlink():
            raise ValueError('public candidate contains private-role marker')
    private_git = git(source)
    public_git = git(candidate)
    contract.check(candidate / 'tools/dot', source)
    raw = config.read_bytes().decode()
    data = tomllib.loads(raw)
    if data.get('hooks') or data.get('sourceDir') not in (str(source), str(source / 'home')):
        raise ValueError('source ownership conflict or hooks; left untouched')
    if data.get('data', {}).get('dotEngine') not in (None, str(source / 'tools/dot')):
        raise ValueError('engine reference ownership conflict')
    # Validate supported, bounded TOML edits before any snapshot or mutation.
    changed_config(raw, source, p['private'], p['public'])
    value = {'version': 1, 'home': str(home), 'source': str(source), 'config': str(config),
             'private_git': private_git, 'public_git': public_git,
             'candidate': str(candidate), 'snapshots': {name: inventory(path) for name, path in
             [('source', source), ('candidate', candidate), ('config', config), ('launcher', p['launcher']), ('state', p['state'])]}}
    value['id'] = intent.digest(value)
    return value


def changed_config(raw, source, private, public):
    pattern = r'(?m)^(sourceDir\s*=\s*)("[^"\r\n]*"|\x27[^\x27\r\n]*\x27)([^\r\n]*)(\r?\n|$)'
    if len(re.findall(pattern, raw)) != 1:
        raise ValueError('unsupported sourceDir syntax; manual review required')
    raw = re.sub(pattern, lambda m: m[1] + json.dumps(str(private / 'home')) + m[3] + m[4], raw)
    engine = json.dumps(str(public / 'tools/dot'))
    if 'dotEngine' in tomllib.loads(raw).get('data', {}):
        pattern = r'(?m)^(dotEngine\s*=\s*)("[^"\r\n]*"|\x27[^\x27\r\n]*\x27)([^\r\n]*)(\r?\n|$)'
        if len(re.findall(pattern, raw)) != 1:
            raise ValueError('unsupported dotEngine syntax')
        raw = re.sub(pattern, lambda m: m[1] + engine + m[3] + m[4], raw)
    else:
        pattern = r'(?m)^\[data\]([^\r\n]*)(\r?\n)'
        if len(re.findall(pattern, raw)) != 1:
            raise ValueError('explicit [data] table required; manual review')
        raw = re.sub(pattern, lambda m: m[0] + 'dotEngine = ' + engine + m[2], raw)
    parsed = tomllib.loads(raw)
    if parsed['sourceDir'] != str(private / 'home') or parsed['data']['dotEngine'] != str(public / 'tools/dot'):
        raise ValueError('reference edit verification failed')
    return raw


def copy_verified(source, destination, expected):
    if source.is_dir():
        shutil.copytree(source, destination, symlinks=True)
    else:
        shutil.copy2(source, destination, follow_symlinks=False)
    if inventory(source) != expected or inventory(destination) != expected:
        raise ValueError('snapshot verification failed; references unchanged')


def apply(value, token, home, source, config):
    if not isinstance(value, dict) or token != value.get('id') or proposal(home, source, config, value['candidate']) != value:
        raise ValueError('stale/unapproved installed migration; review fresh plan before mutation')
    p = paths(home)
    backup = p['recovery'] / token
    if backup.exists() or backup.is_symlink():
        raise ValueError('recovery collision; no overwrite or unsafe retry')
    progress = {'version': 1, 'stage': 'snapshotting', 'backup': str(backup), 'plan': value, 'completed': []}
    # Journal precedes all snapshot/destination writes, including directory creation.
    write(p['journal'], json.dumps(progress, indent=2) + '\n')
    backup.mkdir(parents=True, mode=0o700, exist_ok=False)
    backup.chmod(0o700)
    def checkpoint(stage):
        progress['stage'] = stage
        write(p['journal'], json.dumps(progress, indent=2) + '\n')
        if os.environ.get('DOT_MIGRATION_TEST_STOP') == stage:
            raise ValueError('incomplete installed migration; recovery: ' + str(backup) + '; fixture interruption at ' + stage)
    instructions = ('Interrupted migration: do not rerun bootstrap/migrate or delete originals.\n'
                    'Inspect installed-migration.json completed steps and compare snapshots.\n'
                    'Restore config, launcher and install.json only from verified config/launcher/state snapshots, preserving modes.\n'
                    'At snapshotting, originals are unchanged; do not use partial/unverified snapshots.\n'
                    'Retain both new destinations and all recovery data; manually reconcile/quarantine new destinations before a fresh plan.\n'
                    'Only after verifying original references and private bytes, archive the journal and recovery directory outside dot state.\n')
    write(backup / 'README', instructions)
    checkpoint('snapshotting')
    try:
        for name, path in [('source', source), ('candidate', Path(value['candidate'])), ('config', config), ('launcher', p['launcher'])]:
            copy_verified(path, backup / name, value['snapshots'][name])
        # The journal/recovery are our only additions to local state; snapshot original entries individually.
        (backup / 'state').mkdir(mode=0o700)
        for child in p['state'].iterdir():
            if child.name not in ('installed-migration.json', 'installed-recovery'):
                if child.is_dir() and not child.is_symlink():
                    shutil.copytree(child, backup / 'state' / child.name, symlinks=True)
                else:
                    shutil.copy2(child, backup / 'state' / child.name, follow_symlinks=False)
        (backup / 'state').chmod(value['snapshots']['state']['.'][1])
        if inventory(backup / 'state') != value['snapshots']['state']:
            raise ValueError('local state snapshot verification failed')
        checkpoint('snapshots-verified')
        copy_verified(backup / 'source', p['private'], value['snapshots']['source'])
        progress['completed'].append('private-checkout')
        checkpoint('private-copied')
        copy_verified(backup / 'candidate', p['public'], value['snapshots']['candidate'])
        progress['completed'].append('public-checkout')
        checkpoint('public-copied')
        contract.check(p['public'] / 'tools/dot', p['private'])
        if git(p['private']) != value['private_git'] or git(p['public']) != value['public_git']:
            raise ValueError('Git identity/configuration changes at relocated path; references unchanged; manual review required')
        # Verify originals and every local approval/backup entry once more before references switch.
        for name, path in [('source', source), ('candidate', Path(value['candidate'])), ('config', config), ('launcher', p['launcher'])]:
            if inventory(path) != value['snapshots'][name]:
                raise ValueError('original changed after snapshot; references unchanged')
        current = inventory(p['state'])
        current = {k: v for k, v in current.items() if not any(k == './' + n or k.startswith('./' + n + '/') for n in ('installed-migration.json', 'installed-recovery'))}
        if current != value['snapshots']['state']:
            raise ValueError('local state changed after snapshot; references unchanged')
        checkpoint('references-ready')
        raw = changed_config(config.read_bytes().decode(), source, p['private'], p['public'])
        mode = stat.S_IMODE(config.stat().st_mode)
        write(config, raw, mode)
        progress['completed'].append('config')
        checkpoint('config-written')
        write(p['launcher'], launcher(p['private'], config, p['public']), value['snapshots']['launcher']['.'][1])
        progress['completed'].append('launcher')
        checkpoint('launcher-written')
        record = json.loads((backup / 'state/install.json').read_text())
        record.update(version=2, directory=str(p['private']), engine=str(p['public']),
                      engine_source=value['public_git']['origin'] or str(value['candidate']))
        mode = stat.S_IMODE(p['record'].stat().st_mode)
        write(p['record'], json.dumps(record, indent=2) + '\n', mode)
        progress['completed'].append('record')
        checkpoint('record-written')
        if inventory(source) != value['snapshots']['source'] or inventory(p['private']) != value['snapshots']['source']:
            raise ValueError('private preservation verification failed')
        checkpoint('complete')
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        raise ValueError('incomplete installed migration; recovery: ' + str(backup) + '; ' + str(error)) from error
    print('Installed split complete; originals retained; no adoption. Recovery: ' + str(backup))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('command', choices=('plan', 'apply'))
    parser.add_argument('file', nargs='?')
    parser.add_argument('--engine', help='reviewed independent public checkout candidate (no fetch/clone)')
    parser.add_argument('--out')
    parser.add_argument('--approve')
    args = parser.parse_args()
    home = Path(os.path.abspath(os.environ['HOME']))
    source = Path(os.path.abspath(os.environ['DOT_SOURCE']))
    config = Path(os.environ.get('DOT_CONFIG', str(home / '.config/chezmoi/chezmoi.toml')))
    if args.command == 'plan':
        if not args.engine or not args.out:
            raise ValueError('plan requires --engine public checkout and --out new plan file')
        value = proposal(home, source, config, args.engine)
        if value.get('complete'):
            print('Installed layout already split; no changes or new approval.')
            return
        output = Path(os.path.abspath(args.out))
        if output.resolve() != output or output.exists() or output.is_symlink() or any(output.is_relative_to(Path(value[k])) for k in ('source', 'candidate')) or output.is_relative_to(home / '.local/state/dot'):
            raise ValueError('plan output must be new, outside checkouts and local state')
        intent.write(output, json.dumps(value, indent=2) + '\n')
        print('Installed split preview (no adoption/packages/services):\n' + str(source) + ' → ' + str(paths(home)['private']) + '\nPublic candidate: ' + args.engine + ' → ' + str(paths(home)['public']) + '\nReferences: sourceDir, dotEngine, launcher, install.json\nOriginals, exact scope, identity and recovery retained. Plan: ' + value['id'])
    else:
        if not args.file:
            raise ValueError('saved plan required')
        apply(json.loads(Path(args.file).read_text()), args.approve, home, source, config)


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError, subprocess.CalledProcessError) as error:
        print('dot migrate: ' + str(error), file=sys.stderr)
        sys.exit(1)
