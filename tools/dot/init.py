#!/usr/bin/env python3
"""Consented first-run setup; detection never evaluates private configuration."""
import argparse
import json
import os
from pathlib import Path
import sys
import tomllib
import subprocess
import tempfile
import shutil
import shlex
import hashlib
import re

import packages

import contract


def safe_path(path):
    if not path.is_absolute() or '..' in path.parts:
        raise ValueError('explicit absolute paths without traversal required')
    for item in (path, *path.parents):
        if item.is_symlink():
            raise ValueError('symlink path conflict: ' + str(item))
        if item == Path(os.environ['HOME']):
            break


def detect(source, engine, config):
    home = Path(os.environ['HOME'])
    print('Private source: ' + str(source / 'home'))
    print('Actual config: ' + str(config))
    record_path = home / '.local/state/dot/install.json'
    for path in (home, source, engine, config, record_path):
        safe_path(path)
    for suffix in ('json', 'jsonc', 'yaml', 'yml'):
        alternate = config.with_suffix('.' + suffix)
        if alternate.exists() or alternate.is_symlink():
            raise ValueError('alternate tracker config exists: ' + str(alternate) + '; review explicitly; no tracker created')
    record = contract.read_json(record_path)
    if not isinstance(record, dict):
        raise ValueError('invalid installation record; review recorded paths and recovery; left untouched')
    if (record.get('version') != 2 or record.get('directory') != str(source)
            or record.get('engine') != str(engine.parent.parent)
            or record.get('config') != str(config)):
        raise ValueError('installation record conflicts; review dot migrate; left untouched')
    actual = subprocess.check_output(
        ['git', '-C', str(source), 'remote', 'get-url', 'origin'], text=True).strip()
    if actual != record.get('source'):
        raise ValueError('private remote identity conflicts; no source switched or created')
    actual_engine = subprocess.check_output(
        ['git', '-C', str(engine.parent.parent), 'remote', 'get-url', 'origin'], text=True).strip()
    if actual_engine != record.get('engine_source'):
        raise ValueError('public remote identity conflicts; review installation record; left untouched')
    contract.check(engine, source)
    if config.exists():
        data = tomllib.loads(config.read_text())
        print('Configured source: ' + str(data.get('sourceDir', '(missing)')))
        if (data.get('sourceDir') != str(source / 'home')
                or data.get('data', {}).get('dotEngine') != str(engine)
                or data.get('destDir', str(home)) != str(home)):
            raise ValueError('tracker identity conflict; explicit migration needed: review dot migrate; left untouched')
        print('Compatible tracker detected; machine data preserved.')
        return record, data
    candidates = {home / '.local/share/chezmoi',
                  Path(os.environ.get('XDG_DATA_HOME', str(home / '.local/share'))) / 'chezmoi'}
    for directory in {home / '.config/chezmoi',
                      Path(os.environ.get('XDG_CONFIG_HOME', str(home / '.config'))) / 'chezmoi'}:
        candidates.update(directory / ('chezmoi.' + suffix) for suffix in ('toml', 'json', 'jsonc', 'yaml', 'yml'))
    for candidate in sorted(candidates):
        if candidate != config and (candidate.exists() or candidate.is_symlink()):
            raise ValueError('existing unrecorded tracker: ' + str(candidate) + '; explicit migration/recovery needed; data left untouched')
    print('Tracker pending; no configuration applied.')
    return record, None


def ask(prompt):
    try:
        with open('/dev/tty', 'w') as terminal, open('/dev/tty') as input_stream:
            terminal.write(prompt + ' ')
            terminal.flush()
            return input_stream.readline().strip()
    except OSError:
        return ''


def consent(prompt):
    return ask(prompt + ' [y/N]') in ('y', 'yes')


def compatible(tool):
    if not shutil.which(tool):
        return False
    result = subprocess.run([tool, '--version'], capture_output=True, text=True)
    if result.returncode != 0:
        return False
    if tool == 'gh':
        version = re.match(r'gh version (\d+)\.(\d+)\.(\d+)', result.stdout)
        if not version or version.groups()[:2] in (('2', '45'), ('2', '46')):
            return False
        return all(subprocess.run(['gh', 'auth', verb, '--help'], stdout=subprocess.DEVNULL,
                                  stderr=subprocess.DEVNULL).returncode == 0 for verb in ('setup-git', 'switch'))
    if tool == 'python3':
        return subprocess.run(['python3', '-c', 'import sys,tomllib; sys.exit(sys.version_info < (3,11))'],
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0
    return True


def dependencies(validate):
    required = ('bash', 'git', 'curl', 'gh', 'python3', 'chezmoi', 'gum')
    def remaining():
        print('Remaining dependencies: ' + ', '.join(tool for tool in required if not compatible(tool)))
    for tool in required:
        if compatible(tool):
            continue
        print('Missing/incompatible dependency: ' + tool, flush=True)
        if platform() == 'darwin' and not compatible('brew'):
            if not provision_homebrew(validate):
                print('Remaining dependency: ' + tool + '; Homebrew provisioning declined; tracker pending.')
                remaining()
                return False
        planner = dependency_plan(tool)
        print('Dependency operation ID: ' + planner['id'], flush=True)
        print(planner['warning'], flush=True)
        print('Command: ' + shlex.join(planner['command']), flush=True)
        print('Read-only resolution:\n' + planner['resolution'], flush=True)
        if not consent('Install dependency ' + tool + ' with this exact operation?'):
            print('Remaining dependency: ' + tool + '; tracker setup pending.')
            remaining()
            return False
        if dependency_plan(tool) != planner:
            raise ValueError('stale dependency operation; review again')
        validate()
        apply_dependency(planner, validate)
        if not compatible(tool):
            raise ValueError('dependency verification failed: ' + tool + '; inspect manager output and PATH before retrying')
    return True


def provision_homebrew(validate):
    url = 'https://raw.githubusercontent.com/Homebrew/install/700c9a145d37a3f0f3bd3b7c208d7adab31bd278/install.sh'
    checksum = 'ece8385d14080da35eb3c1f72cf43b71be5d1a3838e554e4bb748b9888b15cec'
    print('Homebrew provisioning: download ' + url + '\nSHA-256: ' + checksum, flush=True)
    print('Reviewed installer executes shell code as your user and may request administrator permission to provision /opt/homebrew. No application lists or version pins will be changed. Execution has separate consent after verification.', flush=True)
    if not consent('Provision Homebrew: download this reviewed installer?'):
        return False
    validate()
    with tempfile.TemporaryDirectory(prefix='dot-init-brew-') as directory:
        path = Path(directory) / 'homebrew.sh'
        packages.run(['curl', '--fail', '--silent', '--show-error', '--location', '--proto', '=https', '--proto-redir', '=https', '--tlsv1.2', url, '-o', str(path)])
        if hashlib.sha256(path.read_bytes()).hexdigest() != checksum:
            raise ValueError('Homebrew installer checksum mismatch; no code executed')
        print('Command: bash ' + str(path) + ' (verified SHA-256 ' + checksum + ')', flush=True)
        if not consent('Execute this exact verified Homebrew installer?'):
            return False
        if hashlib.sha256(path.read_bytes()).hexdigest() != checksum:
            raise ValueError('stale Homebrew installer')
        validate()
        packages.run(['bash', str(path)], mutate=True)
    if not shutil.which('brew') and Path('/opt/homebrew/bin/brew').is_file():
        os.environ['PATH'] = '/opt/homebrew/bin:' + os.environ['PATH']
    if not compatible('brew'):
        raise ValueError('Homebrew verification failed; inspect installation and PATH before retrying')
    return True


def platform():
    system = packages.run(['uname', '-s']).strip()
    if system == 'Darwin':
        if packages.run(['uname', '-m']).strip() != 'arm64':
            raise ValueError('native Apple Silicon required')
        version = packages.run(['sw_vers', '-productVersion']).strip().split('.')[0]
        if not version.isdigit() or int(version) < 15:
            raise ValueError('Apple Silicon macOS 15+ required')
        return 'darwin'
    if system == 'Linux':
        fields = dict(line.split('=', 1) for line in packages.run(['cat', '/etc/os-release']).splitlines() if '=' in line)
        distro = fields.get('ID', '').strip('"')
        architecture = packages.run(['uname', '-m']).strip()
        if distro == 'arch':
            if architecture != 'x86_64':
                raise ValueError('official Arch x86_64 required; no ARM or derivative fallback')
            return distro
        if distro == 'ubuntu':
            version = fields.get('VERSION_ID', '').strip('"')
            if not re.fullmatch(r'\d{2}\.\d{2}', version) or (version != '22.04' and tuple(map(int, version.split('.'))) < (24, 4)):
                raise ValueError('Ubuntu 24.04+ required; 22.04 requires existing Python 3.11+')
            native = packages.run(['dpkg', '--print-architecture']).strip()
            if (native, architecture) not in (('amd64', 'x86_64'), ('arm64', 'aarch64'), ('arm64', 'arm64')):
                raise ValueError('native Ubuntu amd64 or arm64 required')
            return distro
    raise ValueError('unsupported platform; supported: Apple Silicon macOS, Ubuntu, official Arch')


def dependency_plan(tool):
    os_name = platform()
    manager = {'darwin': 'brew', 'ubuntu': 'apt-get', 'arch': 'pacman'}[os_name]
    binary = shutil.which(manager)
    if not binary:
        raise ValueError(manager + ' unavailable; provision the manager explicitly, then retry dot init; no fallback executed')
    if os_name == 'ubuntu' and tool in ('chezmoi', 'gum'):
        architecture = packages.run(['dpkg', '--print-architecture']).strip()
        pins = {
            ('gum', 'amd64'): '4c59b09c7248ea03c1544a11506b1152b1d8bd20e602fb2c2e9d158204d1f490',
            ('gum', 'arm64'): '9ae145bdd738f65e843f904957496d5aea1c2c182f3f1999b609ed9627f6a68e',
            ('chezmoi', 'amd64'): '24df0be680b422e87fd8b4930fba583650920d68af81d53162a30867f8a8e0c8',
            ('chezmoi', 'arm64'): '99372f1e46c0532d377c0a368d3d3ba60eb252a9dbba3ca43e1767f571a73362',
        }
        if (tool, architecture) not in pins:
            raise ValueError('unsupported Ubuntu architecture')
        filename = f'gum_0.17.0_{architecture}.deb' if tool == 'gum' else f'chezmoi_2.73.0_linux_{architecture}.deb'
        base = 'charmbracelet/gum/releases/download/v0.17.0' if tool == 'gum' else 'twpayne/chezmoi/releases/download/v2.73.0'
        sudo = shutil.which('sudo')
        if not sudo:
            raise ValueError('sudo unavailable; authorize permissions separately')
        result = {'tool': tool, 'platform': os_name, 'command': [sudo, binary, 'install', '--no-remove', '--no-install-recommends', 'VERIFIED_LOCAL_DEB'],
                  'artifact': {'url': 'https://github.com/' + base + '/' + filename, 'filename': filename, 'sha256': pins[tool, architecture]},
                  'warning': 'Download reviewed upstream Debian artifact into an owned temporary directory and verify embedded SHA-256. Then separately review/consent to resolved apt installation; dependencies/maintainer scripts and sudo permission may be required. No repository edits or application version-pin changes.',
                  'resolution': 'Artifact: ' + filename + '\nDownload: https://github.com/' + base + '/' + filename + '\nSHA-256: ' + pins[tool, architecture],
                  'environment': packages.digest(json.dumps(dict(os.environ), sort_keys=True))}
        result['id'] = packages.digest(json.dumps(result, sort_keys=True))
        return result
    package = {'python3': 'python@3.11'}.get(tool, tool) if os_name == 'darwin' else tool
    if os_name == 'darwin':
        command = [binary, 'install', '--force-bottle', '--', package]
        query = [binary, 'install', '--dry-run', '--force-bottle', '--', package]
        warning = 'Homebrew bottles; dependencies may be added. No automatic cleanup, dependent repair or update. Inspect the final manager transaction.'
    elif os_name == 'arch':
        package = {'python3': 'core/python', 'gh': 'extra/github-cli', 'bash': 'core/bash', 'curl': 'core/curl'}.get(tool, 'extra/' + tool)
        command = [binary, '-Syu', '--', package]
        query = [binary, '-Sp', '--print-format', '%n %v', '--', package]
        warning = 'FULL SYSTEM UPGRADE: refreshes repositories and upgrades unrelated packages, dependencies, replacements, scripts/hooks. Review archlinux.org/news, pacman configuration, reboot/.pacnew needs. No AUR or partial-upgrade fallback. Pacman retains final transaction approval; printed targets do not resolve the whole upgrade.'
    else:
        command = [binary, 'install', '--no-remove', '--no-install-recommends', '--', package]
        query = [binary, '-s', 'install', '--no-remove', '--no-install-recommends', '--', package]
        warning = 'Ubuntu configured apt repositories only; dependencies and maintainer scripts may run. No repository edits, metadata refresh, full upgrade, Python PPA or removal. Inspect final transaction. Missing repository candidates require separately reviewed provisioning.'
    if os_name != 'darwin':
        sudo = shutil.which('sudo')
        if not sudo:
            raise ValueError('sudo unavailable; provision permissions separately and retry')
        command.insert(0, sudo)
        warning += ' Displayed sudo transaction may request administrator permission.'
    result = {'tool': tool, 'platform': os_name, 'command': command,
              'warning': warning, 'resolution': packages.run(query),
              'environment': packages.digest(json.dumps(dict(os.environ), sort_keys=True)),
              'executables': []}
    for executable in command[:2] if os_name != 'darwin' else command[:1]:
        path = Path(executable)
        result['executables'].append({'path': str(path), 'sha256': hashlib.sha256(path.read_bytes()).hexdigest()})
    result['id'] = packages.digest(json.dumps(result, sort_keys=True))
    return result


def apply_dependency(plan, validate):
    if 'artifact' not in plan:
        packages.run(plan['command'], mutate=True)
        return
    artifact = plan['artifact']
    with tempfile.TemporaryDirectory(prefix='dot-init-') as directory:
        path = Path(directory) / artifact['filename']
        packages.run(['curl', '--fail', '--silent', '--show-error', '--location', '--proto', '=https', '--proto-redir', '=https', '--tlsv1.2', artifact['url'], '-o', str(path)])
        if hashlib.sha256(path.read_bytes()).hexdigest() != artifact['sha256']:
            raise ValueError('dependency checksum mismatch; artifact not installed')
        command = [*plan['command'][:-1], str(path)]
        query = [command[1], '-s', *command[2:]]
        resolution = packages.run(query)
        print('Verified artifact apt transaction:\n' + resolution + '\nCommand: ' + shlex.join(command), flush=True)
        if not consent('Install this exact verified artifact transaction?'):
            raise ValueError('artifact installation declined; dependency remains pending')
        if packages.run(query) != resolution or hashlib.sha256(path.read_bytes()).hexdigest() != artifact['sha256']:
            raise ValueError('stale artifact transaction; review again')
        validate()
        packages.run(command, mutate=True)


def create_tracker(source, engine, config, record, profile):
    home = Path(os.environ['HOME'])
    state = home / '.local/state/dot'
    identity = {}
    for field, prompt in (('name', 'Tracker full name (local machine data only):'),
                          ('email', 'Tracker email (local machine data only):')):
        result = subprocess.run(['git', '-C', str(source), 'config', 'user.' + field], capture_output=True, text=True)
        if result.returncode not in (0, 1):
            raise ValueError('cannot read Git identity; review private source configuration')
        identity[field] = result.stdout.strip() or ask(prompt)
        if not identity[field]:
            print('Tracker pending: missing ' + field + '; no Git settings changed.')
            return False
    text = ''.join(key + ' = ' + json.dumps(str(value)) + '\n' for key, value in (
        ('sourceDir', source / 'home'), ('destDir', home),
        ('cacheDir', state / 'chezmoi-cache'), ('persistentState', state / 'chezmoi-state.boltdb')))
    text += '[data]\n' + ''.join(key + ' = ' + json.dumps(value) + '\n' for key, value in (
        ('name', identity['name']), ('email', identity['email']), ('profile', profile), ('dotEngine', str(engine))))
    print('Create local-unmanaged tracker config (mode 0600):\n' + text, flush=True)
    print('No private templates/hooks execute. No live files, approvals, cache or database are created. Existing machine data is never removed.', flush=True)
    if not consent('Create this tracker at ' + str(config) + '?'):
        print('Tracker pending: creation declined; no changes made.')
        return False
    current, data = detect(source, engine, config)
    if current != record or data is not None:
        raise ValueError('stale tracker creation; review again')
    safe_path(config)
    config.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix='.dot-init-', dir=config.parent)
    try:
        with os.fdopen(fd, 'w') as stream:
            stream.write(text)
            stream.flush()
            os.fsync(stream.fileno())
        # Hard-link publication refuses concurrent creation rather than replacing it.
        os.link(temporary, config)
    finally:
        os.unlink(temporary)
    print('Tracker created; no configuration applied.')
    return True


def browser_rounds(source, engine, config):
    detect(source, engine, config)
    browser = Path(os.environ.get('DOT_BROWSER', str(engine.parent.parent / 'bin/dot-browser')))
    safe_path(browser)
    if not browser.is_file() or not os.access(browser, os.X_OK):
        print('Browser unavailable; install a reviewed compatible prebuilt artifact. No Go build or fallback application. Finish remains available.')
        return
    print('Optional Track and Adopt rounds: nothing selected; Finish default. Each mutation needs separate exact-plan consent.', flush=True)
    packages.run([str(browser), '--engine', str(engine), '--source', str(source), '--home', os.environ['HOME'], '--rounds'], mutate=True)
    print('Browser returned; no additional configuration approval inferred.', flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true', help='read-only identity detection')
    parser.add_argument('--rounds', action='store_true', help='offer prebuilt Track/Adopt rounds after setup; Finish default')
    parser.add_argument('--profile', choices=['personal', 'work'], default='personal')
    args = parser.parse_args()
    source = Path(os.environ['DOT_SOURCE'])
    engine = Path(os.environ['DOT_ENGINE'])
    config = Path(os.environ['DOT_CONFIG'])
    record, data = detect(source, engine, config)
    if args.check:
        return 0
    platform()
    def validate():
        if detect(source, engine, config) != (record, data):
            raise ValueError('stale installation/tracker identity; review again')
    if not dependencies(validate):
        return 0
    if data is None and not create_tracker(source, engine, config, record, args.profile):
        return 0
    if args.rounds:
        browser_rounds(source, engine, config)
        return 0
    while True:
        action = ask('Setup action [Finish/packages/rounds]:').lower()
        if action in ('', 'finish', 'cancel'):
            print('Finish; adoption scope unchanged. Empty scope is not whole-machine parity.')
            break
        if action == 'rounds':
            browser_rounds(source, engine, config)
        elif action == 'packages':
            planner = packages.Planner(source, platform=platform())
            plan = planner.plan('init')
            print(planner.review(plan), flush=True)
            if not plan['commands']:
                continue
            approval = ask('Approve exact package plan ID (empty cancels):')
            if not approval:
                print('Install missing canceled; no packages installed.')
                continue
            ack = False
            if any(row['manager'] == 'yay' for row in plan['missing']):
                ack = consent('Separately acknowledge unprivileged AUR build-script execution?')
            detect(source, engine, config)
            planner.apply(plan, approval, 'init', ack_aur=ack)
        else:
            print('Unknown action; Finish remains the default.')
    return 0


if __name__ == '__main__':
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        print('dot init canceled; inspect any earlier completed manager operations before retrying; machine data retained', file=sys.stderr)
        sys.exit(130)
    except subprocess.SubprocessError:
        print('dot init: Git identity detection failed; inspect the reported source/config and recorded origin; no source switched or created; do not delete machine data', file=sys.stderr)
        sys.exit(1)
    except (OSError, ValueError, KeyError, TypeError) as error:
        print('dot init: ' + str(error) + '; inspect actual source/config; do not delete machine data', file=sys.stderr)
        sys.exit(1)
