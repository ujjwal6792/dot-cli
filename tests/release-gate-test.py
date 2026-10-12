#!/usr/bin/env python3
"""Offline safety tests for the GitHub release publication gate."""
import importlib.util
import hashlib
import json
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
DRAFT_SPEC = importlib.util.spec_from_file_location('draft_release', ROOT / 'scripts/draft-release.py')
DRAFT = importlib.util.module_from_spec(DRAFT_SPEC)
DRAFT_SPEC.loader.exec_module(DRAFT)


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

    def test_release_workflow_serializes_same_version_without_canceling_active_run(self):
        workflow = (ROOT / '.github/workflows/release.yml').read_text()
        self.assertRegex(
            workflow,
            r'(?m)^concurrency:\n  group: release-\$\{\{ inputs\.approved_version \}\}\n  cancel-in-progress: false$',
        )

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
                {'name': path.name, 'size': path.stat().st_size,
                 'digest': 'sha256:' + hashlib.sha256(path.read_bytes()).hexdigest()}
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
            with self.assertRaisesRegex(ValueError, 'size mismatch'):
                ASSET_CHECKER.verify(first, [dict(item, size=item['size'] + 1) for item in uploaded])

    def _draft_fixture(self, directory, source='a' * 40):
        asset = directory / 'package.tar.gz'
        asset.write_bytes(b'verified package')
        record = {
            'id': 41, 'tag_name': 'v0.1.0-rc.2', 'target_commitish': source,
            'draft': True, 'prerelease': True, 'created_at': '2026-10-10T23:05:34Z',
            'assets': [{'name': asset.name, 'size': asset.stat().st_size,
                        'digest': 'sha256:' + hashlib.sha256(asset.read_bytes()).hexdigest()}],
        }
        return asset, record

    def _install_draft_api(self, record, fail_after_retarget=False, published_tag_sha=None):
        calls = []

        def fake_gh(*args):
            calls.append(args)
            if args == ('--paginate', 'repos/owner/repo/releases'):
                return json.dumps([record])
            if args == ('repos/owner/repo/releases/41',):
                if fail_after_retarget and any('target_commitish=' in str(arg) for call in calls for arg in call):
                    raise RuntimeError('synthetic read failure after retarget')
                return json.dumps(record)
            if args == ('repos/owner/repo/git/matching-refs/tags/v0.1.0-rc.2',):
                return '[]'
            if args[:2] == ('--method', 'PATCH'):
                if args[2] != 'repos/owner/repo/releases/41':
                    self.fail('PATCH did not use release ID')
                if 'draft=false' in args:
                    record['draft'] = False
                for arg in args:
                    if arg.startswith('target_commitish='):
                        record['target_commitish'] = arg.split('=', 1)[1]
                return '{}'
            if args == ('repos/owner/repo/commits/v0.1.0-rc.2',):
                return json.dumps({'sha': published_tag_sha or record['target_commitish']})
            return '{}'

        return calls, fake_gh

    def test_stale_ancestor_draft_is_verified_retargeted_then_published_by_id(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            _, record = self._draft_fixture(directory)
            calls, fake_gh = self._install_draft_api(record)
            originals = (DRAFT.gh, DRAFT.is_ancestor, DRAFT.verified_origin_run, DRAFT.tag_ref_is_absent)
            DRAFT.gh = fake_gh
            DRAFT.is_ancestor = lambda source, target: source == 'a' * 40 and target == 'b' * 40
            DRAFT.verified_origin_run = lambda *args: {'id': 99}
            DRAFT.tag_ref_is_absent = lambda *args: True
            try:
                release_id = DRAFT.prepare('owner/repo', 'v0.1.0-rc.2', 'b' * 40, directory, inspect_only=True)
                self.assertEqual(release_id, 41)
                self.assertFalse(any(call[:2] == ('--method', 'PATCH') for call in calls))
                release_id = DRAFT.prepare('owner/repo', 'v0.1.0-rc.2', 'b' * 40, directory)
                DRAFT.publish('owner/repo', release_id, 'v0.1.0-rc.2', 'b' * 40, directory)
            finally:
                DRAFT.gh, DRAFT.is_ancestor, DRAFT.verified_origin_run, DRAFT.tag_ref_is_absent = originals
            mutations = [call for call in calls if call[:2] == ('--method', 'PATCH')]
            self.assertEqual(len(mutations), 2)
            self.assertIn('target_commitish=' + 'b' * 40, mutations[0])
            self.assertIn('draft=true', mutations[0])
            self.assertIn('draft=false', mutations[1])
            self.assertTrue(all(call[2] == 'repos/owner/repo/releases/41' for call in mutations))
            self.assertFalse(any('DELETE' in call for call in calls))

    def test_stale_mismatch_nonancestor_duplicate_or_unknown_source_never_mutates(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            asset, record = self._draft_fixture(directory)
            calls, fake_gh = self._install_draft_api(record)
            originals = (DRAFT.gh, DRAFT.is_ancestor, DRAFT.verified_origin_run, DRAFT.tag_ref_is_absent)
            DRAFT.gh = fake_gh
            DRAFT.is_ancestor = lambda *args: False
            DRAFT.verified_origin_run = lambda *args: {'id': 99}
            DRAFT.tag_ref_is_absent = lambda *args: True
            try:
                with self.assertRaisesRegex(ValueError, 'ancestor'):
                    DRAFT.prepare('owner/repo', 'v0.1.0-rc.2', 'b' * 40, directory)
                DRAFT.is_ancestor = lambda *args: True
                DRAFT.verified_origin_run = lambda *args: (_ for _ in ()).throw(ValueError('unknown source'))
                with self.assertRaisesRegex(ValueError, 'unknown source'):
                    DRAFT.prepare('owner/repo', 'v0.1.0-rc.2', 'b' * 40, directory)
                DRAFT.verified_origin_run = lambda *args: {'id': 99}
                record['assets'][0]['size'] += 1
                with self.assertRaisesRegex(ValueError, 'size mismatch'):
                    DRAFT.prepare('owner/repo', 'v0.1.0-rc.2', 'b' * 40, directory)
                record['assets'][0]['size'] -= 1
                record['assets'][0]['digest'] = 'sha256:' + '0' * 64
                with self.assertRaisesRegex(ValueError, 'digest mismatch'):
                    DRAFT.prepare('owner/repo', 'v0.1.0-rc.2', 'b' * 40, directory)
                record['assets'][0]['digest'] = 'sha256:' + hashlib.sha256(asset.read_bytes()).hexdigest()
                duplicate = json.dumps([record, record])
                DRAFT.gh = lambda *args: duplicate
                with self.assertRaisesRegex(ValueError, 'multiple'):
                    DRAFT.prepare('owner/repo', 'v0.1.0-rc.2', 'b' * 40, directory)
            finally:
                DRAFT.gh, DRAFT.is_ancestor, DRAFT.verified_origin_run, DRAFT.tag_ref_is_absent = originals
            self.assertFalse(any(call[:2] == ('--method', 'PATCH') for call in calls))

    def test_failure_after_retarget_keeps_draft_and_never_publishes(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            _, record = self._draft_fixture(directory)
            calls, fake_gh = self._install_draft_api(record, fail_after_retarget=True)
            originals = (DRAFT.gh, DRAFT.is_ancestor, DRAFT.verified_origin_run, DRAFT.tag_ref_is_absent)
            DRAFT.gh = fake_gh
            DRAFT.is_ancestor = lambda *args: True
            DRAFT.verified_origin_run = lambda *args: {'id': 99}
            DRAFT.tag_ref_is_absent = lambda *args: True
            try:
                with self.assertRaisesRegex(RuntimeError, 'after retarget'):
                    DRAFT.prepare('owner/repo', 'v0.1.0-rc.2', 'b' * 40, directory)
            finally:
                DRAFT.gh, DRAFT.is_ancestor, DRAFT.verified_origin_run, DRAFT.tag_ref_is_absent = originals
            mutations = [call for call in calls if call[:2] == ('--method', 'PATCH')]
            self.assertEqual(len(mutations), 1)
            self.assertIn('draft=true', mutations[0])
            self.assertTrue(record['draft'])

    def test_existing_tag_ref_blocks_stale_retarget(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            _, record = self._draft_fixture(directory)
            calls, fake_gh = self._install_draft_api(record)
            originals = (DRAFT.gh, DRAFT.is_ancestor, DRAFT.verified_origin_run, DRAFT.tag_ref_is_absent)
            DRAFT.gh = fake_gh
            DRAFT.is_ancestor = lambda *args: True
            DRAFT.verified_origin_run = lambda *args: {'id': 99}
            DRAFT.tag_ref_is_absent = lambda *args: False
            try:
                with self.assertRaisesRegex(ValueError, 'tag ref already exists'):
                    DRAFT.prepare('owner/repo', 'v0.1.0-rc.2', 'b' * 40, directory)
            finally:
                DRAFT.gh, DRAFT.is_ancestor, DRAFT.verified_origin_run, DRAFT.tag_ref_is_absent = originals
            self.assertFalse(any(call[:2] == ('--method', 'PATCH') for call in calls))

    def test_published_tag_must_resolve_to_approved_commit(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            _, record = self._draft_fixture(directory, source='b' * 40)
            _, fake_gh = self._install_draft_api(record, published_tag_sha='a' * 40)
            original = DRAFT.gh
            DRAFT.gh = fake_gh
            try:
                with self.assertRaisesRegex(ValueError, 'tag does not resolve'):
                    DRAFT.publish('owner/repo', 41, 'v0.1.0-rc.2', 'b' * 40, directory)
            finally:
                DRAFT.gh = original

    def test_final_recheck_refuses_draft_changed_after_initial_verification(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            _, record = self._draft_fixture(directory, source='b' * 40)
            calls, fake_gh = self._install_draft_api(record)
            original = DRAFT.gh
            DRAFT.gh = fake_gh
            try:
                release_id = DRAFT.prepare('owner/repo', 'v0.1.0-rc.2', 'b' * 40, directory)
                record['assets'][0]['digest'] = 'sha256:' + '0' * 64
                with self.assertRaisesRegex(ValueError, 'digest mismatch'):
                    DRAFT.publish('owner/repo', release_id, 'v0.1.0-rc.2', 'b' * 40, directory)
            finally:
                DRAFT.gh = original
            self.assertFalse(any(call[:2] == ('--method', 'PATCH') and 'draft=false' in call for call in calls))
            self.assertTrue(record['draft'])

    def test_stale_source_requires_matching_prior_run_with_successful_build_and_smoke(self):
        release = {'created_at': '2026-10-10T23:05:34Z'}
        run = {'id': 99, 'head_sha': 'a' * 40, 'event': 'workflow_dispatch',
               'head_branch': 'main', 'status': 'completed', 'conclusion': 'failure',
               'created_at': '2026-10-10T22:48:31Z', 'updated_at': '2026-10-10T23:05:39Z'}
        jobs = {'jobs': [
            {'name': 'verify', 'conclusion': 'success'},
            {'name': 'build', 'conclusion': 'success'},
            {'name': 'installed-smoke', 'conclusion': 'success'},
            {'name': 'publish', 'conclusion': 'failure'},
        ]}
        original = DRAFT.gh

        def fake_gh(*args):
            if args == ('--paginate', 'repos/owner/repo/actions/workflows/release.yml/runs?event=workflow_dispatch&per_page=100'):
                return json.dumps({'workflow_runs': [run]})
            if args == ('--paginate', 'repos/owner/repo/actions/runs/99/jobs'):
                return json.dumps(jobs)
            self.fail(f'unexpected fake GitHub API call: {args}')

        DRAFT.gh = fake_gh
        try:
            self.assertEqual(DRAFT.verified_origin_run('owner/repo', release, 'a' * 40), run)
            jobs['jobs'][1]['conclusion'] = 'failure'
            with self.assertRaisesRegex(ValueError, 'exactly one'):
                DRAFT.verified_origin_run('owner/repo', release, 'a' * 40)
        finally:
            DRAFT.gh = original


if __name__ == '__main__':
    unittest.main()
