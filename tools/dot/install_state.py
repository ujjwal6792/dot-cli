"""Validated access to local public-install and private-connection records."""
import json
import os
from pathlib import Path
import re
import stat
import tempfile


INSTALL_KEYS = {'record_version', 'owner', 'artifact', 'install_root', 'engine_path', 'browser_sha256'}
ARTIFACT_KEYS = {'repository', 'release_tag', 'platform', 'sha256'}
CONNECTION_KEYS = {'connection_version', 'private_source', 'config_path'}
OWNERS = {'artifact': {'curl'}, 'package-manager': {'brew', 'pacman'}}
PLATFORMS = {'darwin-arm64', 'linux-amd64', 'linux-arm64'}
SHA256 = re.compile(r'[a-f0-9]{64}\Z')


def _record_path(state_dir, name):
    directory = Path(state_dir)
    if not directory.is_absolute():
        raise ValueError('state directory must be absolute')
    for component in (directory, *directory.parents):
        if component.is_symlink():
            raise ValueError('state path must not traverse symlinks')
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
    value = _read(_record_path(state_dir, 'install.json'))
    if value is None:
        return None
    if set(value) != INSTALL_KEYS or type(value['record_version']) is not int or value['record_version'] != 1:
        raise ValueError('unsupported or malformed install record schema')
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
    if not directory.is_absolute():
        raise ValueError('state directory must be absolute')
    if directory.is_symlink():
        raise ValueError('state directory must not be a symlink')
    for component in directory.parents:
        if component.is_symlink():
            raise ValueError('state path must not traverse symlinks')
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
