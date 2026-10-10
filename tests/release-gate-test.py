#!/usr/bin/env python3
"""Offline safety tests for the GitHub release publication gate."""
import importlib.util
import hashlib
import tempfile
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location('release_gate', ROOT / 'scripts/release-gate.py')
GATE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(GATE)
BUILD_SPEC = importlib.util.spec_from_file_location('release_build', ROOT / 'scripts/build-release.py')
BUILDER = importlib.util.module_from_spec(BUILD_SPEC)
BUILD_SPEC.loader.exec_module(BUILDER)
CHECK_SPEC = importlib.util.spec_from_file_location('release_check', ROOT / 'scripts/check-release.py')
CHECKER = importlib.util.module_from_spec(CHECK_SPEC)
CHECK_SPEC.loader.exec_module(CHECKER)
ASSET_SPEC = importlib.util.spec_from_file_location('verify_release_assets', ROOT / 'scripts/verify-release-assets.py')
ASSET_CHECKER = importlib.util.module_from_spec(ASSET_SPEC)
ASSET_SPEC.loader.exec_module(ASSET_CHECKER)


class ReleaseGateTests(unittest.TestCase):
    def eligible(self, **overrides):
        context = {
            'repository': 'ujjwal6792/dots-cli',
            'event_name': 'workflow_dispatch',
            'ref': 'refs/heads/main',
            'sha': 'a' * 40,
            'current_main': 'a' * 40,
            'version': 'v0.1.0-rc.2',
            'existing_tags': 'v0.1.0-rc.1',
        }
        context.update(overrides)
        return context

    def test_dispatch_on_exact_current_public_main_is_eligible(self):
        self.assertEqual(GATE.validate(self.eligible()), 'v0.1.0-rc.2')
        self.assertEqual(GATE.validate(self.eligible(version='v0.1.0-rc.10', existing_tags='v0.1.0-rc.9')), 'v0.1.0-rc.10')

    def test_feature_branch_dispatch_is_rejected(self):
        with self.assertRaisesRegex(ValueError, 'main'):
            GATE.validate(self.eligible(ref='refs/heads/implement/standalone-38'))

    def test_stale_main_dispatch_is_rejected(self):
        with self.assertRaisesRegex(ValueError, 'current main'):
            GATE.validate(self.eligible(current_main='b' * 40))

    def test_other_repository_or_event_is_rejected(self):
        for overrides in ({'repository': 'attacker/fork'}, {'event_name': 'push'}):
            with self.subTest(overrides=overrides), self.assertRaises(ValueError):
                GATE.validate(self.eligible(**overrides))

    def test_only_next_rc_format_is_allowed_and_rc1_is_never_republished(self):
        for version in ('v0.1.0-rc.1', 'v0.1.0', '0.1.0-rc.2', 'v0.1.0-rc.0', 'v0.1.0-rc.02', 'v0.1.0-rc.2-extra'):
            with self.subTest(version=version), self.assertRaises(ValueError):
                GATE.validate(self.eligible(version=version))

    def test_existing_or_nonmonotonic_rc_is_rejected(self):
        with self.assertRaisesRegex(ValueError, 'already exists'):
            GATE.validate(self.eligible(existing_tags='v0.1.0-rc.1\nv0.1.0-rc.2'))
        with self.assertRaises(ValueError):
            GATE.validate(self.eligible(version='v0.1.0-rc.1', existing_tags=''))
        with self.assertRaisesRegex(ValueError, 'greater'):
            GATE.validate(self.eligible(version='v0.1.0-rc.3', existing_tags='v0.1.0-rc.4'))

    def test_fixture_packages_are_reproducible_and_checksum_checked_without_running_go(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            fixtures = root / 'fixture-binaries'
            fixtures.mkdir()
            for platform in BUILDER.PLATFORMS:
                binary = fixtures / ('dot-browser-' + platform)
                binary.write_bytes(('synthetic browser fixture: ' + platform).encode())
                binary.chmod(0o755)
            first, second = root / 'first', root / 'second'
            BUILDER.build('0.1.0-rc.2', first, fixtures)
            BUILDER.build('0.1.0-rc.2', second, fixtures)
            CHECKER.check(first)
            uploaded = [
                {'name': path.name, 'digest': 'sha256:' + hashlib.sha256(path.read_bytes()).hexdigest()}
                for path in first.iterdir()
            ]
            ASSET_CHECKER.verify(first, uploaded)
            self.assertEqual(
                {path.name: path.read_bytes() for path in first.iterdir()},
                {path.name: path.read_bytes() for path in second.iterdir()},
            )
            checksum = first / 'SHA256SUMS'
            checksum.write_text(checksum.read_text().replace('  dot-cli-', '  altered-'))
            with self.assertRaises(ValueError):
                CHECKER.check(first)
            with self.assertRaisesRegex(ValueError, 'digest mismatch'):
                ASSET_CHECKER.verify(first, [dict(item, digest='sha256:' + '0' * 64) for item in uploaded])
            with self.assertRaisesRegex(ValueError, 'names differ'):
                ASSET_CHECKER.verify(first, uploaded[:-1])


if __name__ == '__main__':
    unittest.main()
