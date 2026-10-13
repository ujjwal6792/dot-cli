#!/usr/bin/env python3
"""Non-interactive first-run setup; detection never evaluates private configuration.

Interactive prompts were removed: `dot init` either performs an explicitly
requested step or stops with guidance. The only interactive path left is the
prebuilt Go browser (`--rounds`), which owns its own terminal.
"""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import sys
import tempfile
import tomllib
import subprocess
import urllib.parse

import packages

import contract
import install_state
import state_paths


def local_state_dir():
    return state_paths.state_dir()


def safe_path(path):
    if not path.is_absolute() or '..' in path.parts:
        raise ValueError('explicit absolute paths without traversal required')
    for item in (path, *path.parents):
        if item.is_symlink():
            raise ValueError('symlink path conflict: ' + str(item))
        if item == Path(os.environ['HOME']):
            break


def detect(source, engine, config, enforce_connection=True):
    home = Path(os.environ['HOME'])
    print('Private source: ' + str(source / 'home'))
    print('Actual config: ' + str(config))
    state_dir = local_state_dir()
    connection = install_state.read_connection(state_dir)
    for path in (home, source, config, state_dir):
        safe_path(path)
    if enforce_connection and connection and (connection['private_source'] != str(source) or connection['config_path'] != str(config)):
        raise ValueError('private connection record conflicts with requested paths; rerun with explicit --source/--config to review a connection update')
    if not source.exists():
        print('Private checkout pending; no private repository read or tracker change.')
        return connection, None
    for suffix in ('json', 'jsonc', 'yaml', 'yml'):
        alternate = config.with_suffix('.' + suffix)
        if alternate.exists() or alternate.is_symlink():
            raise ValueError('alternate tracker config exists: ' + str(alternate) + '; review explicitly; no tracker created')
    root = Path(subprocess.check_output(['git', '-C', str(source), 'rev-parse', '--show-toplevel'], text=True).strip()).resolve()
    if root != source.resolve():
        raise ValueError('private source path is not the Git checkout root; no source was switched')
    subprocess.check_output(['git', '-C', str(source), 'remote', 'get-url', 'origin'], text=True)
    contract.check(engine, source)
    if config.exists():
        data = tomllib.loads(config.read_text())
        print('Configured source: ' + str(data.get('sourceDir', '(missing)')))
        if (data.get('sourceDir') != str(source / 'home')
                or data.get('data', {}).get('dotEngine') != str(engine)
                or data.get('destDir', str(home)) != str(home)):
            raise ValueError('tracker identity conflict; explicit migration needed: review dot migrate; left untouched')
        print('Compatible tracker detected; machine data preserved.')
        return connection, data
    candidates = {home / '.local/share/chezmoi',
                  Path(os.environ.get('XDG_DATA_HOME', str(home / '.local/share'))) / 'chezmoi'}
    for directory in {home / '.config/chezmoi',
                      Path(os.environ.get('XDG_CONFIG_HOME', str(home / '.config'))) / 'chezmoi'}:
        candidates.update(directory / ('chezmoi.' + suffix) for suffix in ('toml', 'json', 'jsonc', 'yaml', 'yml'))
    for candidate in sorted(candidates):
        if candidate != config and (candidate.exists() or candidate.is_symlink()):
            raise ValueError('existing unrecorded tracker: ' + str(candidate) + '; explicit migration/recovery needed; data left untouched')
    print('Tracker pending; no configuration applied.')
    return connection, None


def validate_source_url(url):
    """Reject credential-bearing, query, or fragment URLs before any network use."""
    if not url:
        raise ValueError('source URL is empty; rerun with --url <your-repo-url>')
    split = urllib.parse.urlsplit(url)
    if split.username or split.password or re.search(r'://[^/]*:[^/@]*@', url):
        raise ValueError('source URL must not contain credentials; use a credential-free HTTPS or SSH URL')
    if split.query or split.fragment or '?' in url or '#' in url:
        raise ValueError('source URL must not contain query parameters or fragments')
    if split.scheme in ('https', 'ssh', 'file') or url.startswith('git@') or os.path.isabs(url):
        return url
    raise ValueError('source URL must be HTTPS, SSH (git@host:...), file://, or an absolute local path')


def ensure_source(source, engine, url):
    """Clone only an explicitly provided URL; otherwise stop with guidance."""
    if source.exists() or source.is_symlink():
        return True
    if not url:
        print('No private checkout at ' + str(source) + '; nothing was assumed, created, or cloned.')
        print('Existing repository: rerun `dot init --url <your-repo-url>` with a credential-free HTTPS or SSH URL.')
        print('New repository: create the empty repository on your hosting platform, then rerun with --url once it exists.')
        return False
    validated = validate_source_url(url)
    safe_path(source)
    if not source.is_absolute():
        raise ValueError('private checkout destination must be absolute')
    source.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    result = subprocess.run(['git', 'clone', validated, str(source)], capture_output=True, text=True)
    if result.returncode:
        raise ValueError('private repository clone failed: ' + result.stderr.strip())
    contract.check(engine, source)
    print('Private checkout cloned and contract-checked. Tracker creation remains a separate step.', flush=True)
    return True


def compatible(tool):
    executable = os.environ.get('DOT_PYTHON') if tool == 'python3' else None
    executable = executable or shutil.which(tool)
    if not executable:
        return False
    result = subprocess.run([executable, '--version'], capture_output=True, text=True)
    if result.returncode != 0:
        return False
    if tool == 'python3':
        return subprocess.run([executable, '-c', 'import sys,tomllib; sys.exit(sys.version_info < (3,11))'],
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0
    return True


def dependencies():
    required = ('bash', 'git', 'curl', 'python3', 'chezmoi')
    missing = [tool for tool in required if not compatible(tool)]
    if missing:
        print('Missing/incompatible dependencies: ' + ', '.join(missing))
        print('Install them with your platform package manager, then rerun dot init. No dependency was installed.')
        return False
    return True


def platform():
    system = subprocess.check_output(['uname', '-s'], text=True).strip()
    if system == 'Darwin':
        if subprocess.check_output(['uname', '-m'], text=True).strip() != 'arm64':
            raise ValueError('native Apple Silicon required')
        version = subprocess.check_output(['sw_vers', '-productVersion'], text=True).strip().split('.')[0]
        if not version.isdigit() or int(version) < 15:
            raise ValueError('Apple Silicon macOS 15+ required')
        return 'darwin'
    if system == 'Linux':
        fields = dict(line.split('=', 1) for line in Path('/etc/os-release').read_text().splitlines() if '=' in line)
        distro = fields.get('ID', '').strip('"')
        architecture = subprocess.check_output(['uname', '-m'], text=True).strip()
        if distro == 'arch':
            if architecture != 'x86_64':
                raise ValueError('official Arch x86_64 required; no ARM or derivative fallback')
            return distro
        if distro == 'ubuntu':
            version = fields.get('VERSION_ID', '').strip('"')
            if not re.fullmatch(r'\d{2}\.\d{2}', version) or (version != '22.04' and tuple(map(int, version.split('.'))) < (24, 4)):
                raise ValueError('Ubuntu 24.04+ required; 22.04 requires existing Python 3.11+')
            native = subprocess.check_output(['dpkg', '--print-architecture'], text=True).strip()
            if (native, architecture) not in (('amd64', 'x86_64'), ('arm64', 'aarch64'), ('arm64', 'arm64')):
                raise ValueError('native Ubuntu amd64 or arm64 required')
            return distro
    raise ValueError('unsupported platform; supported: Apple Silicon macOS, Ubuntu, official Arch')


def create_tracker(source, engine, config, record, name=None, email=None):
    home = Path(os.environ['HOME'])
    state = state_paths.state_dir(home)
    identity = {}
    for field, override in (('name', name), ('email', email)):
        result = subprocess.run(['git', '-C', str(source), 'config', 'user.' + field], capture_output=True, text=True)
        if result.returncode not in (0, 1):
            raise ValueError('cannot read Git identity; review private source configuration')
        identity[field] = result.stdout.strip() or (override or '')
        if not identity[field]:
            print('Tracker pending: missing ' + field + '; set Git user.' + field
                  + ' in the private source or rerun with --' + field + ' VALUE. No Git settings changed.')
            return False
    text = ''.join(key + ' = ' + json.dumps(str(value)) + '\n' for key, value in (
        ('sourceDir', source / 'home'), ('destDir', home),
        ('cacheDir', state / 'chezmoi-cache'), ('persistentState', state / 'chezmoi-state.boltdb')))
    text += '[data]\n' + ''.join(key + ' = ' + json.dumps(value) + '\n' for key, value in (
        ('name', identity['name']), ('email', identity['email']), ('dotEngine', str(engine))))
    print('Local-unmanaged tracker config (mode 0600):\n' + text, flush=True)
    print('No private templates/hooks execute. No live files, approvals, cache or database are created. Existing machine data is never removed.', flush=True)
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
    if (not browser.is_absolute() or browser.is_symlink() or
            browser.resolve().parent != (engine.parent.parent / 'bin').resolve() or
            not browser.is_file() or not os.access(browser, os.X_OK)):
        print('Browser unavailable; install a reviewed compatible prebuilt artifact. No Go build or fallback application. Finish remains available.')
        return
    print('Optional Track and Adopt rounds: nothing selected; Finish default. Each mutation needs separate exact-plan consent.', flush=True)
    packages.run([str(browser), '--engine', str(engine), '--source', str(source), '--home', os.environ['HOME'], '--rounds'], mutate=True)
    print('Browser returned; no additional configuration approval inferred.', flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true', help='read-only identity detection')
    parser.add_argument('--rounds', action='store_true', help='offer prebuilt Track/Adopt rounds after setup; Finish default')
    parser.add_argument('--url', help='explicit private repository URL (HTTPS/SSH, no credentials); only used to clone a missing checkout')
    parser.add_argument('--name', help='tracker full name when Git user.name is unset; never changes Git settings')
    parser.add_argument('--email', help='tracker email when Git user.email is unset; never changes Git settings')
    parser.add_argument('--source', help='explicit private checkout path; never inferred from the public engine')
    parser.add_argument('--config', help='explicit local chezmoi config path')
    args = parser.parse_args()
    home = Path(os.environ['HOME'])
    state_dir = local_state_dir()
    connection = install_state.read_connection(state_dir)
    source_value = args.source or os.environ.get('DOT_SOURCE')
    config_value = args.config or os.environ.get('DOT_CONFIG')
    if connection:
        source_value = source_value or connection['private_source']
        config_value = config_value or connection['config_path']
    source_value = source_value or str(home / '.local/share/dot-config')
    config_value = config_value or str(Path(os.environ.get('XDG_CONFIG_HOME', str(home / '.config'))) / 'chezmoi/chezmoi.toml')
    source = Path(source_value)
    engine = Path(os.environ['DOT_ENGINE'])
    config = Path(config_value)
    if args.check:
        detect(source, engine, config)
        if not source.exists() and not source.is_symlink():
            print('init verdict: new-user; no private checkout; nothing was created or modified.')
        else:
            print('init verdict: old-user; valid Git checkout with a passing contract check; nothing was created or modified.')
        return 0
    platform()
    new_connection = connection is None or connection['private_source'] != str(source) or connection['config_path'] != str(config)
    if not ensure_source(source, engine, args.url):
        return 0
    record, data = detect(source, engine, config, enforce_connection=not new_connection)
    if new_connection:
        install_state.write_connection(state_dir, str(source), str(config))
        record, data = detect(source, engine, config)

    def validate():
        if detect(source, engine, config) != (record, data):
            raise ValueError('stale installation/tracker identity; review again')
    if not dependencies():
        return 0
    validate()
    if data is None and not create_tracker(source, engine, config, record, args.name, args.email):
        return 0
    if args.rounds:
        browser_rounds(source, engine, config)
        return 0
    print('Finish; adoption scope unchanged. Empty scope is not whole-machine parity.')
    print('Optional Track/Adopt rounds: rerun `dot init --rounds` for the prebuilt browser, or use `dot browse`.')
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
