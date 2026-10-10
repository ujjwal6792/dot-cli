import json
import os
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'tools/dot'))
import install_state


def artifact_record():
    return {
        'record_version': 1,
        'owner': {'kind': 'artifact', 'manager': 'curl'},
        'artifact': {'repository': 'ujjwal6792/dots-cli', 'release_tag': 'v1.2.3',
                     'platform': 'linux-amd64', 'sha256': 'a' * 64},
        'install_root': '/tmp/public tool',
        'engine_path': '/tmp/public tool/current/tools/dot',
        'browser_sha256': 'b' * 64,
    }


class InstallStateTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.state = Path(self.temp.name).resolve() / 'local state' / 'dot'

    def tearDown(self):
        self.temp.cleanup()

    def test_missing_records_are_read_only_and_normal(self):
        self.assertIsNone(install_state.read_install_record(self.state))
        self.assertIsNone(install_state.read_connection(self.state))
        self.assertFalse(self.state.exists())

    def test_artifact_schema_is_independent_of_private_connection(self):
        self.state.mkdir(parents=True)
        self.state.chmod(0o700)
        path = self.state / 'install.json'
        original = json.dumps(artifact_record(), sort_keys=True).encode() + b'\n'
        path.write_bytes(original)
        path.chmod(0o600)
        before_mode = path.stat().st_mode
        self.assertEqual(install_state.read_install_record(self.state), artifact_record())
        connection = install_state.write_connection(self.state, '/tmp/private config', '/tmp/chezmoi.toml')
        self.assertIsNone(connection)
        self.assertEqual(path.read_bytes(), original)
        self.assertEqual(path.stat().st_mode, before_mode)
        self.assertEqual(install_state.read_install_record(self.state), artifact_record())
        self.assertEqual(install_state.read_connection(self.state), {
            'connection_version': 1,
            'private_source': '/tmp/private config',
            'config_path': '/tmp/chezmoi.toml',
        })
        self.assertEqual((self.state / 'connection.json').stat().st_mode & 0o777, 0o600)

    def test_package_manager_owner_is_validated_not_claimed_by_artifact(self):
        self.state.mkdir(parents=True)
        self.state.chmod(0o700)
        value = artifact_record()
        value['owner'] = {'kind': 'package-manager', 'manager': 'pacman'}
        install_path = self.state / 'install.json'
        install_path.write_text(json.dumps(value))
        install_path.chmod(0o600)
        self.assertEqual(install_state.read_install_record(self.state)['owner'], value['owner'])

    def test_unknown_fields_and_redirected_records_fail_closed(self):
        self.state.mkdir(parents=True)
        self.state.chmod(0o700)
        value = artifact_record()
        value['private_source'] = '/private'
        install_path = self.state / 'install.json'
        install_path.write_text(json.dumps(value))
        install_path.chmod(0o600)
        with self.assertRaisesRegex(ValueError, 'schema'):
            install_state.read_install_record(self.state)
        (self.state / 'install.json').unlink()
        target = self.state / 'outside'
        target.write_text(json.dumps(artifact_record()))
        (self.state / 'install.json').symlink_to(target)
        with self.assertRaisesRegex(ValueError, 'symlink'):
            install_state.read_install_record(self.state)

    def test_connection_requires_absolute_paths_and_known_schema(self):
        with self.assertRaisesRegex(ValueError, 'absolute'):
            install_state.write_connection(self.state, 'relative', '/tmp/config')
        self.state.mkdir(parents=True)
        self.state.chmod(0o700)
        connection_path = self.state / 'connection.json'
        connection_path.write_text('{"connection_version": 2}')
        connection_path.chmod(0o600)
        with self.assertRaisesRegex(ValueError, 'schema'):
            install_state.read_connection(self.state)

    def test_state_directory_symlink_is_refused(self):
        actual = Path(self.temp.name).resolve() / 'actual'
        actual.mkdir()
        redirected = Path(self.temp.name).resolve() / 'redirected'
        redirected.symlink_to(actual, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, 'symlink'):
            install_state.read_install_record(redirected)


if __name__ == '__main__':
    unittest.main()
