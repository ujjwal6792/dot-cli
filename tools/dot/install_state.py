"""Validated access to local public-install and private-connection records."""
import json
import os
from pathlib import Path
import re
import stat
import tempfile
import state_paths


INSTALL_KEYS_V1 = {'record_version', 'owner', 'artifact', 'install_root', 'engine_path', 'browser_sha256'}
INSTALL_KEYS_V2 = INSTALL_KEYS_V1 | {'state_dir'}
ARTIFACT_KEYS = {'repository', 'release_tag', 'platform', 'sha256'}
CONNECTION_KEYS = {'connection_version', 'private_source', 'config_path'}
OWNERS = {'artifact': {'curl'}, 'package-manager': {'brew', 'pacman'}}
PLATFORMS = {'darwin-arm64', 'linux-amd64', 'linux-arm64'}
SHA256 = re.compile(r'[a-f0-9]{64}\Z')


def _record_path(state_dir, name):
    directory = state_paths.state_dir() if state_dir is None else Path(state_dir)
    directory = state_paths.validate_directory(directory)
    if directory.exists() and (not directory.is_dir() or stat.S_IMODE(directory.stat().st_mode) != 0o700):
        raise ValueError('state directory must be a mode-0700 directory')
    path = directory / name
    if path.is_symlink():
        raise ValueError(f'{name} must not be a symlink')
    return path


def _read(path):
    if not path.exists():
        return None
    if not path.is_file():
        raise ValueError(f'{path.name} must be a regular file')
    if stat.S_IMODE(path.stat().st_mode) != 0o600:
        raise ValueError(f'{path.name} must have mode 0600')
    try:
        value = json.loads(path.read_text(encoding='utf-8'))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise ValueError(f'invalid {path.name}: {error}') from None
    if not isinstance(value, dict):
        raise ValueError(f'{path.name} must contain a JSON object')
    return value


def _absolute(value, field):
    if not isinstance(value, str) or not value or not Path(value).is_absolute():
        raise ValueError(f'{field} must be an absolute path')
    if os.path.normpath(value) != value:
        raise ValueError(f'{field} must be normalized')


def read_install_record(state_dir):
    """Read and validate installer-owned artifact identity; never creates files."""
    directory = state_paths.state_dir() if state_dir is None else state_paths.validate_directory(state_dir)
    value = _read(_record_path(directory, 'install.json'))
    if value is None:
        return None
    if (type(value.get('record_version')) is not int or
            (value['record_version'] == 1 and set(value) != INSTALL_KEYS_V1) or
            (value['record_version'] == 2 and set(value) != INSTALL_KEYS_V2) or
            value['record_version'] not in (1, 2)):
        raise ValueError('unsupported or malformed install record schema')
    if value['record_version'] == 2:
        _absolute(value['state_dir'], 'state_dir')
        if Path(value['state_dir']) != directory:
            raise ValueError('install record state_dir does not match its containing directory')
    owner, artifact = value['owner'], value['artifact']
    if (not isinstance(owner, dict) or set(owner) != {'kind', 'manager'} or
            not isinstance(owner['kind'], str) or owner['kind'] not in OWNERS):
        raise ValueError('unknown installation owner')
    if not isinstance(owner['manager'], str) or owner['manager'] not in OWNERS[owner['kind']]:
        raise ValueError('unknown installation manager')
    if not isinstance(artifact, dict) or set(artifact) != ARTIFACT_KEYS:
        raise ValueError('malformed artifact identity')
    if (not isinstance(artifact['repository'], str) or artifact['repository'] != 'ujjwal6792/dots-cli' or
            not isinstance(artifact['release_tag'], str) or not artifact['release_tag'].startswith('v') or
            not isinstance(artifact['platform'], str) or artifact['platform'] not in PLATFORMS or
            not isinstance(artifact['sha256'], str) or
            not SHA256.fullmatch(artifact['sha256'])):
        raise ValueError('invalid artifact identity')
    if not isinstance(value['browser_sha256'], str) or not SHA256.fullmatch(value['browser_sha256']):
        raise ValueError('invalid browser checksum')
    _absolute(value['install_root'], 'install_root')
    _absolute(value['engine_path'], 'engine_path')
    return value


def is_split_install(state_dir):
    """Safely detect artifact records that require the independent engine."""
    value = _read(_record_path(state_dir, 'install.json'))
    return bool(value and (value.get('version') == 2 or value.get('record_version') == 2))


def read_connection(state_dir):
    """Read optional private source/tracker connection; never creates state."""
    value = _read(_record_path(state_dir, 'connection.json'))
    if value is None:
        return None
    if (set(value) != CONNECTION_KEYS or type(value['connection_version']) is not int or
            value['connection_version'] != 1):
        raise ValueError('unsupported or malformed connection record schema')
    _absolute(value['private_source'], 'private_source')
    _absolute(value['config_path'], 'config_path')
    return value


def write_connection(state_dir, private_source, config_path):
    """Atomically write only connection.json, retaining installer-owned bytes."""
    _absolute(private_source, 'private_source')
    _absolute(config_path, 'config_path')
    directory = Path(state_dir)
    directory = state_paths.validate_directory(directory)
    directory.mkdir(mode=0o700, parents=True, exist_ok=True)
    if not directory.is_dir():
        raise ValueError('state path must be a directory')
    if stat.S_IMODE(directory.stat().st_mode) != 0o700:
        raise ValueError('state directory must have mode 0700')
    path = _record_path(directory, 'connection.json')
    content = (json.dumps({'connection_version': 1, 'private_source': private_source,
                           'config_path': config_path}, sort_keys=True) + '\n').encode()
    descriptor, temporary = tempfile.mkstemp(prefix='.connection.', dir=directory)
    try:
        with os.fdopen(descriptor, 'wb') as stream:
            stream.write(content)
            stream.flush()
            os.fsync(stream.fileno())
        os.chmod(temporary, stat.S_IRUSR | stat.S_IWUSR)
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)
