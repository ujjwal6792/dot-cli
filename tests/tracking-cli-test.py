#!/usr/bin/env python3
"""Installed CLI acceptance, synthetic private checkout and isolated HOME only."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

REPO = Path(__file__).resolve().parents[1]


class TrackingCliTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='dot-tracking-')
        self.base = Path(self.tmp.name)
        self.home = self.base / 'home'
        self.source = self.base / 'private'
        self.engine = self.base / 'public/tools/dot'
        self.home.mkdir()
        (self.source / 'home').mkdir(parents=True)
        (self.source / 'tools/dot').mkdir(parents=True)
        shutil.copytree(REPO / 'tools/dot', self.engine, ignore=shutil.ignore_patterns('browser', '__pycache__', 'manifest.json', 'groups.json'))
        (self.source / 'dot-contract.json').write_text(json.dumps({
            'protocol': 1, 'engine_api': {'min': 1, 'max': 1},
            'manifest_version': 1, 'approval_version': 2,
            'manifest': 'tools/dot/manifest.json', 'home_source': 'home',
            'required_adapters': ['intent', 'scope', 'gitconfig', 'skills', 'layout']}))
        self.env = dict(os.environ, HOME=str(self.home), DOT_SOURCE=str(self.source), DOT_ENGINE=str(self.engine),
                        GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM='1', PYTHONDONTWRITEBYTECODE='1')
        tracker = self.base / 'tracker.toml'
        tracker.write_text('sourceDir=' + json.dumps(str(self.source / 'home')) +
                           '\n[data]\ndotEngine=' + json.dumps(str(self.engine)) + '\n')
        self.env['DOT_CONFIG'] = str(tracker)
        self.git('init', '-q')
        self.git('config', 'user.name', 'Fixture')
        self.git('config', 'user.email', 'fixture@example.invalid')
        self.entry = dict(destination='prefs.conf', source='home/prefs.conf', os=['darwin', 'linux'],
                          portability='managed-shared', impact='Safe', removals=[], hooks=[], tools=[])
        self.catalog = dict(version=1, entries={'prefs': self.entry}, profiles={'default': ['prefs']})
        (self.source / 'home/prefs.conf').write_text('repository preference\n')
        self.live = self.home / 'prefs.conf'
        self.live.write_text('live preference\n')
        self.live.chmod(0o640)
        self.save_catalog()
        self.commit()
        self.scope = self.home / '.local/state/dot/adoption.json'
        self.scope.parent.mkdir(parents=True)
        self.scope.write_text(json.dumps(dict(version=2, profile='default', entries={'prefs': self.entry})))

    def tearDown(self):
        self.tmp.cleanup()

    def git(self, *args):
        return subprocess.run(['git', '-C', str(self.source), *args], env=self.env, capture_output=True, text=True, check=True).stdout

    def commit(self):
        self.git('add', '.')
        self.git('commit', '-qm', 'synthetic fixture')

    def save_catalog(self):
        (self.source / 'tools/dot/manifest.json').write_text(json.dumps(self.catalog))

    def cli(self, *args, ok=True):
        result = subprocess.run([str(self.engine / 'dot'), *args], env=self.env, text=True, capture_output=True)
        self.assertEqual(result.returncode == 0, ok, result.stdout + result.stderr)
        return result

    def plan(self, request):
        path = self.base / 'request.json'
        path.write_text(json.dumps(request))
        output = self.base / 'plan.json'
        result = self.cli('intent', 'plan', str(path), '--out', str(output))
        return output, json.loads(output.read_text()), result.stdout

    def test_stop_tracking_template_stale_live_or_source_is_rejected_without_mutation(self):
        source = self.source / 'home/private_prefs.conf.tmpl'
        (self.source / 'home/prefs.conf').rename(source)
        source.write_text('theme = {{ .chezmoi.os }}\n')
        self.entry.update(source='home/private_prefs.conf.tmpl', portability='managed-templated')
        self.save_catalog()
        self.commit()
        self.scope.write_text(json.dumps(dict(version=2, profile='default', entries={'prefs': self.entry})))
        path, plan, _ = self.plan(dict(version=1, mode='untrack', profile='default', entries=['prefs']))
        self.live.write_text('changed live\n')
        self.cli('intent', 'apply', str(path), '--approve', plan['id'], ok=False)
        self.assertTrue(source.exists())
        self.assertIn('prefs', json.loads(self.scope.read_text())['entries'])
        path, plan, _ = self.plan(dict(version=1, mode='untrack', profile='default', entries=['prefs']))
        source.write_text('edited template\n')
        self.cli('intent', 'apply', str(path), '--approve', plan['id'], ok=False)
        self.assertEqual(source.read_text(), 'edited template\n')
        self.assertEqual(self.live.read_text(), 'changed live\n')

    def test_stop_tracking_requires_exact_approval_and_keeps_live_bytes_and_mode(self):
        before = self.live.read_bytes(), self.live.stat().st_mode
        path, plan, review = self.plan(dict(version=1, mode='untrack', profile='default', entries=['prefs']))
        self.assertIn('live files unchanged', review)
        self.assertIn('home/prefs.conf', review)
        self.assertIn('scope', review.lower())
        self.cli('intent', 'preview', str(path))
        self.cli('intent', 'apply', str(path), '--approve', 'wrong', ok=False)
        self.assertTrue((self.source / 'home/prefs.conf').exists())
        self.assertIn('prefs', json.loads(self.scope.read_text())['entries'])
        self.cli('intent', 'apply', str(path), '--approve', plan['id'])
        self.assertFalse((self.source / 'home/prefs.conf').exists())
        self.assertEqual(json.loads((self.source / 'tools/dot/manifest.json').read_text())['entries'], {})
        self.assertEqual(json.loads(self.scope.read_text())['entries'], {})
        self.assertEqual((self.live.read_bytes(), self.live.stat().st_mode), before)
        self.assertEqual(self.git('status', '--porcelain'), '')
        recovery = self.home / '.local/state/dot/recovery' / plan['id']
        self.assertTrue((recovery / 'plan.json').is_file())
        self.assertTrue(any(p.read_bytes() == b'repository preference\n' for p in recovery.iterdir() if p.name.isdigit()))

    def test_stop_tracking_other_os_fails_closed(self):
        self.entry['os'] = ['linux' if sys.platform == 'darwin' else 'darwin']
        self.save_catalog()
        self.commit()
        request = self.base / 'request.json'
        request.write_text(json.dumps(dict(version=1, mode='untrack', profile='default', entries=['prefs'])))
        self.cli('intent', 'plan', str(request), ok=False)
        self.assertTrue((self.source / 'home/prefs.conf').exists())
        self.assertEqual(self.live.read_text(), 'live preference\n')

    def metadata_fixture(self):
        current = 'linux' if sys.platform.startswith('linux') else sys.platform
        self.catalog.update(version=2, machines={'fixture-stable-id': {'label': 'Fixture workstation', 'os': current}}, tracking={})
        identity = self.home / '.local/state/dot/machine.json'
        identity.write_text(json.dumps({'version': 1, 'id': 'fixture-stable-id'}))
        identity.chmod(0o600)
        descriptor = self.source / 'dot-contract.json'
        contract = json.loads(descriptor.read_text())
        contract['manifest_version'] = 2
        contract['approval_version'] = 3
        descriptor.write_text(json.dumps(contract))
        self.save_catalog()
        self.commit()

    def test_track_logical_cross_os_mapping_and_metadata_precedence(self):
        self.metadata_fixture()
        request = dict(version=1, mode='track', profile='mapped', selections=[{
            'path': 'prefs.conf', 'logical': 'logical-prefs',
            'destinations': {'darwin': 'Library/Preferences/sample.conf', 'linux': '.config/sample/prefs.conf'},
            'common': {'portability': 'managed-shared', 'impact': 'Safe', 'tools': ['common-tool']},
            'machine': {'fixture-stable-id': {'impact': 'Unsafe', 'tools': ['machine-tool']}},
            'os_metadata': {'darwin': {'tools': ['os-tool']}, 'linux': {'tools': ['os-tool']}}}])
        path, plan, review = self.plan(request)
        self.assertIn('Library/Preferences/sample.conf', review)
        self.assertIn('.config/sample/prefs.conf', review)
        self.assertIn('fixture-stable-id', review)
        self.assertIn('Unsafe', review)
        self.assertIn('os-tool', review)
        before = self.live.read_bytes(), self.live.stat().st_mode
        self.cli('intent', 'apply', str(path), '--approve', plan['id'], '--ack-unsafe')
        self.assertEqual((self.live.read_bytes(), self.live.stat().st_mode), before)
        self.assertEqual((self.source / 'home/Library/Preferences/sample.conf').read_text(), 'live preference\n')
        self.assertEqual((self.source / 'home/dot_config/sample/prefs.conf').read_text(), 'live preference\n')
        self.assertFalse((self.home / '.config/sample/prefs.conf').exists())
        self.assertNotIn('logical-prefs@darwin', json.loads(self.scope.read_text())['entries'])
        # Separate adoption gives exact current-OS scope, never future mappings.
        current = 'linux' if sys.platform.startswith('linux') else sys.platform
        path, plan, review = self.plan(dict(version=1, mode='adopt', profile='mapped'))
        self.cli('intent', 'apply', str(path), '--approve', plan['id'], '--ack-unsafe')
        destination = request['selections'][0]['destinations'][current]
        self.assertEqual((self.home / destination).read_text(), 'live preference\n')
        self.assertEqual(set(json.loads(self.scope.read_text())['entries']), {'logical-prefs@' + current})
        recorded = json.loads((self.source / 'tools/dot/manifest.json').read_text())
        self.assertEqual(recorded['profiles']['mapped'], ['logical-prefs@darwin', 'logical-prefs@linux'])

    def test_installed_contract_negotiates_v2_without_mutation(self):
        self.metadata_fixture()
        before = self.git('rev-parse', 'HEAD'), self.scope.read_bytes(), self.live.read_bytes()
        result = self.cli('contract', 'check', '--engine', str(self.engine), '--source', str(self.source))
        self.assertEqual(json.loads(result.stdout)['manifest_version'], 2)
        self.assertEqual((self.git('rev-parse', 'HEAD'), self.scope.read_bytes(), self.live.read_bytes()), before)

    def test_intent_cannot_bypass_incompatible_installed_contract(self):
        self.metadata_fixture()
        descriptor = self.engine / 'engine.json'
        public = json.loads(descriptor.read_text())
        public.pop('manifest_versions')
        descriptor.write_text(json.dumps(public))
        request = self.base / 'request.json'
        request.write_text(json.dumps(dict(version=1, mode='untrack', profile='default', entries=['prefs'])))
        self.cli('intent', 'plan', str(request), ok=False)
        self.assertEqual(self.git('status', '--porcelain'), '')

    def test_edit_mapped_metadata_updates_scope_together_without_touching_live_file(self):
        self.metadata_fixture()
        current = 'linux' if sys.platform.startswith('linux') else sys.platform
        request = dict(version=1, mode='track', profile='default', selections=[{
            'path': 'prefs.conf', 'logical': 'mapped', 'destinations': {current: 'mapped.conf'},
            'common': {'impact': 'Safe', 'portability': 'managed-shared'}}])
        path, plan, _ = self.plan(request)
        self.cli('intent', 'apply', str(path), '--approve', plan['id'])
        path, plan, _ = self.plan(dict(version=1, mode='adopt', profile='default', entries=['mapped@' + current]))
        self.cli('intent', 'apply', str(path), '--approve', plan['id'])
        live = self.home / 'mapped.conf'
        before = live.read_bytes(), live.stat().st_mode
        path, plan, review = self.plan(dict(version=1, mode='metadata', profile='default', metadata={
            'mapped': {'common': {'impact': 'Unsafe', 'portability': 'managed-shared', 'tools': ['reviewed-tool']},
                       'destinations': {current: 'renamed.conf'}}}))
        self.assertIn('renamed.conf', review)
        self.assertIn('scope', review.lower())
        self.cli('intent', 'apply', str(path), '--approve', plan['id'], ok=False)
        self.cli('intent', 'apply', str(path), '--approve', plan['id'], '--ack-unsafe')
        self.assertEqual((live.read_bytes(), live.stat().st_mode), before)
        self.assertFalse((self.home / 'renamed.conf').exists())
        approved = json.loads(self.scope.read_text())['entries']['mapped@' + current]
        self.assertEqual(approved['destination'], 'renamed.conf')
        self.assertEqual(approved['impact'], 'Unsafe')
        self.assertFalse((self.source / 'home/mapped.conf').exists())
        self.assertEqual((self.source / 'home/renamed.conf').read_text(), 'live preference\n')
        path, plan, _ = self.plan(dict(version=1, mode='untrack', profile='default', entries=['mapped@' + current]))
        self.cli('intent', 'apply', str(path), '--approve', plan['id'], '--ack-unsafe')
        self.assertEqual(json.loads(self.scope.read_text())['entries'], {})
        self.assertEqual((live.read_bytes(), live.stat().st_mode), before)

    def test_pending_source_scope_transaction_blocks_new_plan_and_cli_recovers(self):
        path, plan, _ = self.plan(dict(version=1, mode='untrack', profile='default', entries=['prefs']))
        backup = self.home / '.local/state/dot/recovery' / plan['id']
        backup.mkdir(parents=True)
        (backup / 'plan.json').write_text(json.dumps(plan))
        (backup / 'transaction.json').write_text(json.dumps({'status': 'pending', 'live_files': 'unchanged'}))
        (backup / 'scope.before').write_bytes(self.scope.read_bytes())
        index = self.source / '.git/index'
        (backup / 'index.before').write_bytes(index.read_bytes())
        for i, action in enumerate(plan['actions']):
            source = self.source / action['path']
            if source.exists():
                (backup / str(i)).write_bytes(source.read_bytes())
        # Simulate a crash after source removal, before scope removal/commit.
        (self.source / 'home/prefs.conf').unlink()
        request = self.base / 'request.json'
        self.cli('intent', 'plan', str(request), ok=False)
        before = self.live.read_bytes(), self.live.stat().st_mode
        self.cli('intent', 'recover', str(path), '--approve', 'wrong', ok=False)
        self.cli('intent', 'recover', str(path), '--approve', plan['id'])
        self.assertEqual((self.live.read_bytes(), self.live.stat().st_mode), before)
        self.assertEqual((self.source / 'home/prefs.conf').read_text(), 'repository preference\n')
        self.assertEqual(self.git('status', '--porcelain'), '')
        self.assertEqual(json.loads((backup / 'transaction.json').read_text())['status'], 'rolled-back')

    def test_legacy_sidecar_cannot_change_adoption_owned_machine_identity(self):
        self.metadata_fixture()
        current = 'linux' if sys.platform.startswith('linux') else sys.platform
        self.catalog['machines']['second-fixture-id'] = {'label': 'Second fixture', 'os': current}
        self.save_catalog()
        self.commit()
        request = dict(version=1, mode='track', profile='default', selections=[{
            'path': 'prefs.conf', 'logical': 'mapped', 'destinations': {current: 'mapped.conf'},
            'common': {'impact': 'Safe', 'portability': 'managed-shared'}}])
        path, plan, _ = self.plan(request)
        self.cli('intent', 'apply', str(path), '--approve', plan['id'])
        path, plan, _ = self.plan(dict(version=1, mode='adopt', profile='default', entries=['mapped@' + current]))
        self.cli('intent', 'apply', str(path), '--approve', plan['id'])
        identity = self.home / '.local/state/dot/machine.json'
        identity.write_text(json.dumps({'version': 1, 'id': 'second-fixture-id'}))
        before = self.scope.read_bytes()
        self.cli('intent', 'update', '--yes')
        self.assertEqual(self.scope.read_bytes(), before)
        self.assertEqual(json.loads(before)['machine']['id'], 'fixture-stable-id')
        self.assertEqual((self.home / 'mapped.conf').read_text(), 'live preference\n')

    def test_legacy_remove_cannot_bypass_review_on_unapproved_contract_source(self):
        self.scope.unlink()
        bin_dir = self.base / 'bin'
        bin_dir.mkdir()
        marker = self.base / 'native-called'
        fake = bin_dir / 'chezmoi'
        fake.write_text('#!/bin/sh\ntouch "' + str(marker) + '"\nexit 0\n')
        fake.chmod(0o755)
        self.env['PATH'] = str(bin_dir) + os.pathsep + self.env['PATH']
        self.cli('remove', str(self.live), ok=False)
        self.assertFalse(marker.exists(), 'raw chezmoi called without an exact plan')
        self.assertTrue((self.source / 'home/prefs.conf').exists())
        self.assertEqual(self.live.read_text(), 'live preference\n')

    def test_legacy_remove_cannot_bypass_machine_bound_scope(self):
        self.metadata_fixture()
        path, plan, _ = self.plan(dict(version=1, mode='adopt', profile='default', entries=['prefs']))
        self.cli('intent', 'apply', str(path), '--approve', plan['id'])
        bin_dir = self.base / 'bin'
        bin_dir.mkdir()
        marker = self.base / 'native-called'
        fake = bin_dir / 'chezmoi'
        fake.write_text('#!/bin/sh\ntouch "' + str(marker) + '"\nexit 0\n')
        fake.chmod(0o755)
        self.env['PATH'] = str(bin_dir) + os.pathsep + self.env['PATH']
        self.cli('remove', str(self.live), ok=False)
        self.assertFalse(marker.exists(), 'raw chezmoi called without an exact plan')

    def test_native_status_uses_adoption_owned_machine_scope_without_sidecar(self):
        if shutil.which('chezmoi') is None:
            self.skipTest('chezmoi required for isolated native parity')
        self.metadata_fixture()
        current = 'linux' if sys.platform.startswith('linux') else sys.platform
        path, plan, _ = self.plan(dict(version=1, mode='track', profile='default', selections=[{
            'path': 'prefs.conf', 'logical': 'mapped', 'destinations': {current: 'mapped.conf'},
            'common': {'impact': 'Safe', 'portability': 'managed-shared'},
            'machine': {'fixture-stable-id': {'tools': ['fixture-machine-tool']}}}]))
        self.cli('intent', 'apply', str(path), '--approve', plan['id'])
        path, plan, _ = self.plan(dict(version=1, mode='adopt', profile='default', entries=['mapped@' + current]))
        self.cli('intent', 'apply', str(path), '--approve', plan['id'])
        (self.source / 'home/future.conf').write_text('future opt-in\n')
        (self.source / 'home/.chezmoiignore').write_text('{{ output "python3" "-B" (joinPath (env "DOT_ENGINE") "scope.py") "ignore" .chezmoi.sourceDir .chezmoi.homeDir .chezmoi.configFile .chezmoi.os "-1" "check" (.chezmoi.args | toJson) }}\n')
        self.commit()
        config = self.base / 'chezmoi.toml'
        config.write_text('sourceDir=' + json.dumps(str(self.source / 'home')) +
                          '\n[data]\nfixture = true\ndotEngine=' + json.dumps(str(self.engine)) + '\n')
        self.env['DOT_CONFIG'] = str(config)
        result = self.cli('status')
        self.assertNotIn('future.conf', result.stdout)
        self.assertFalse((self.home / 'future.conf').exists())
        identity = self.home / '.local/state/dot/machine.json'
        identity.unlink()
        before = self.scope.read_bytes()
        self.cli('status')
        self.assertEqual(self.scope.read_bytes(), before)
        self.assertFalse(identity.exists(), 'detection must not create identity')
        self.assertEqual((self.home / 'mapped.conf').read_text(), 'live preference\n')

    def test_machine_override_removal_collision_is_rejected_before_source_mutation(self):
        self.metadata_fixture()
        current = 'linux' if sys.platform.startswith('linux') else sys.platform
        request = dict(version=1, mode='track', profile='default', selections=[{
            'path': 'prefs.conf', 'logical': 'mapped', 'destinations': {current: 'mapped.conf'},
            'common': {'impact': 'Safe', 'portability': 'managed-shared'},
            'machine': {'fixture-stable-id': {'removals': ['prefs.conf']}}}])
        path = self.base / 'request.json'
        path.write_text(json.dumps(request))
        self.cli('intent', 'plan', str(path), ok=False)
        self.assertFalse((self.source / 'home/mapped.conf').exists())
        self.assertEqual(self.git('status', '--porcelain'), '')

    def test_stop_tracking_special_permission_change_invalidates_approval(self):
        path, plan, _ = self.plan(dict(version=1, mode='untrack', profile='default', entries=['prefs']))
        self.live.chmod(0o1640)
        self.cli('intent', 'apply', str(path), '--approve', plan['id'], ok=False)
        self.assertTrue((self.source / 'home/prefs.conf').exists())
        self.assertEqual(self.live.stat().st_mode & 0o7777, 0o1640)

    def test_invalid_identity_mapping_and_exclusions_fail_closed_with_no_enrollment(self):
        self.metadata_fixture()
        current = 'linux' if sys.platform.startswith('linux') else sys.platform
        other = 'linux' if current == 'darwin' else 'darwin'
        identity = self.home / '.local/state/dot/machine.json'
        identity_text = identity.read_text()
        good = {'path': 'prefs.conf', 'logical': 'mapped', 'destinations': {current: 'mapped.conf'},
                'common': {'impact': 'Safe', 'portability': 'managed-shared'}}
        request_file = self.base / 'request.json'
        for bad_identity in ('{}', '{"version":1,"id":"unknown-id"}', '{"version":1,"id":"fixture-stable-id","id":"unknown-id"}'):
            with self.subTest(identity=bad_identity):
                identity.write_text(bad_identity)
                request_file.write_text(json.dumps(dict(version=1, mode='track', profile='default', selections=[good])))
                self.cli('intent', 'plan', str(request_file), ok=False)
                self.assertEqual(self.git('status', '--porcelain'), '')
        identity.write_text(identity_text)
        for destinations in ({current: 'mapped.conf', 'windows': 'win.conf'},
                             {other: 'other.conf'}, {current: 'mapped.conf', other: 'prefs.conf'},
                             {current: 'prefs.conf/child'}, {current: '.ssh/config'}):
            with self.subTest(destinations=destinations):
                request_file.write_text(json.dumps(dict(version=1, mode='track', profile='default', selections=[dict(good, destinations=destinations)])))
                self.cli('intent', 'plan', str(request_file), ok=False)
                self.assertFalse((self.source / 'home/mapped.conf').exists())
                self.assertEqual(self.git('status', '--porcelain'), '')
        # Identity rename/ambient hostname is not a source of authority.
        self.env['HOSTNAME'] = 'changed-hostname'
        path, plan, _ = self.plan(dict(version=1, mode='track', profile='default', selections=[good]))
        identity.write_text(identity_text.replace('fixture-stable-id', 'unknown-id'))
        self.cli('intent', 'apply', str(path), '--approve', plan['id'], ok=False)
        self.assertFalse((self.source / 'home/mapped.conf').exists())

    def test_directory_snapshot_never_enrolls_future_files_and_new_scope_stays_opt_in(self):
        directory = self.home / 'configs'
        directory.mkdir()
        (directory / 'first.conf').write_text('eligible fixture\n')
        (directory / 'auth.json').write_text('excluded fixture\n')
        path, plan, _ = self.plan(dict(version=1, mode='track', profile='default', selections=[{
            'path': 'configs', 'os': ['darwin', 'linux'], 'portability': 'managed-shared', 'impact': 'Safe'}]))
        (directory / 'later.conf').write_text('future opt-in\n')
        self.cli('intent', 'apply', str(path), '--approve', plan['id'], ok=False)
        self.assertFalse((self.source / 'home/configs').exists())
        # Exact snapshots can be applied without importing later siblings.
        path, plan, _ = self.plan(dict(version=1, mode='track', profile='default', selections=[{
            'path': 'configs/first.conf', 'os': ['darwin', 'linux'], 'portability': 'managed-shared', 'impact': 'Safe'}]))
        old_scope = self.scope.read_bytes()
        self.cli('intent', 'apply', str(path), '--approve', plan['id'])
        self.assertTrue((self.source / 'home/configs/first.conf').exists())
        self.assertFalse((self.source / 'home/configs/later.conf').exists())
        self.assertFalse((self.source / 'home/configs/auth.json').exists())
        self.assertEqual(self.scope.read_bytes(), old_scope)

    def test_installed_current_machine_tracking_view_is_read_only(self):
        before = self.git('rev-parse', 'HEAD'), self.scope.read_bytes(), self.live.read_bytes()
        result = self.cli('intent', 'tracked')
        rows = json.loads(result.stdout)['entries']
        self.assertEqual([(e['id'], e['metadata']['destination'], e['approved']) for e in rows], [('prefs', 'prefs.conf', True)])
        self.assertEqual((self.git('rev-parse', 'HEAD'), self.scope.read_bytes(), self.live.read_bytes()), before)
        self.assertEqual(self.git('status', '--porcelain'), '')

    def test_source_tracking_commit_never_executes_repository_hooks(self):
        marker = self.home / 'hook-ran'
        hook = self.source / '.git/hooks/pre-commit'
        hook.write_text('#!/bin/sh\ntouch "' + str(marker) + '"\nexit 1\n')
        hook.chmod(0o755)
        path, plan, _ = self.plan(dict(version=1, mode='untrack', profile='default', entries=['prefs']))
        self.cli('intent', 'apply', str(path), '--approve', plan['id'])
        self.assertFalse(marker.exists())
        self.assertFalse((self.source / 'home/prefs.conf').exists())

    def test_failed_source_commit_rolls_back_source_manifest_scope_and_index_together(self):
        self.git('config', 'user.name', '')
        self.git('config', 'user.email', '')
        old_scope = self.scope.read_bytes(), self.scope.stat().st_mode
        old_catalog = (self.source / 'tools/dot/manifest.json').read_bytes()
        old_head = self.git('rev-parse', 'HEAD')
        path, plan, _ = self.plan(dict(version=1, mode='untrack', profile='default', entries=['prefs']))
        self.cli('intent', 'apply', str(path), '--approve', plan['id'], ok=False)
        self.assertEqual((self.source / 'home/prefs.conf').read_text(), 'repository preference\n')
        self.assertEqual((self.source / 'tools/dot/manifest.json').read_bytes(), old_catalog)
        self.assertEqual((self.scope.read_bytes(), self.scope.stat().st_mode), old_scope)
        self.assertEqual(self.git('rev-parse', 'HEAD'), old_head)
        self.assertEqual(self.git('status', '--porcelain'), '')
        self.assertEqual(self.live.read_text(), 'live preference\n')

    def test_unapproved_machine_metadata_downgrade_still_requires_former_unsafe_acknowledgment(self):
        self.metadata_fixture()
        current = 'linux' if sys.platform.startswith('linux') else sys.platform
        path, plan, _ = self.plan(dict(version=1, mode='track', profile='default', selections=[{
            'path': 'prefs.conf', 'logical': 'mapped', 'destinations': {current: 'mapped.conf'},
            'common': {'impact': 'Safe', 'portability': 'managed-shared'},
            'machine': {'fixture-stable-id': {'impact': 'Unsafe'}}}]))
        self.cli('intent', 'apply', str(path), '--approve', plan['id'], '--ack-unsafe')
        path, plan, _ = self.plan(dict(version=1, mode='metadata', profile='default', metadata={'mapped': {'machine': {}}}))
        self.cli('intent', 'apply', str(path), '--approve', plan['id'], ok=False)
        self.cli('intent', 'apply', str(path), '--approve', plan['id'], '--ack-unsafe')
        self.assertFalse((self.home / 'mapped.conf').exists())

    def test_native_unmanifested_source_attribute_collision_blocks_logical_add(self):
        self.metadata_fixture()
        (self.source / 'home/private_mapped.conf').write_text('unselected private source\n')
        self.commit()
        current = 'linux' if sys.platform.startswith('linux') else sys.platform
        path = self.base / 'request.json'
        path.write_text(json.dumps(dict(version=1, mode='track', profile='default', selections=[{
            'path': 'prefs.conf', 'logical': 'mapped', 'destinations': {current: 'mapped.conf'},
            'common': {'impact': 'Safe', 'portability': 'managed-shared'}}])))
        self.cli('intent', 'plan', str(path), ok=False)
        self.assertEqual((self.source / 'home/private_mapped.conf').read_text(), 'unselected private source\n')
        self.assertFalse((self.source / 'home/mapped.conf').exists())

    def test_logical_template_cannot_bypass_binary_input_but_secret_free_replacement_is_explicit(self):
        self.metadata_fixture()
        current = 'linux' if sys.platform.startswith('linux') else sys.platform
        selection = {'path': 'prefs.conf', 'logical': 'mapped', 'destinations': {current: 'mapped.conf'},
                     'common': {'impact': 'Safe', 'portability': 'managed-templated'},
                     'template': 'theme = {{ .chezmoi.os }}\n'}
        request_file = self.base / 'request.json'
        request_file.write_text(json.dumps(dict(version=1, mode='track', profile='default', selections=[selection])))
        self.live.write_bytes(b'\x00binary')
        self.cli('intent', 'plan', str(request_file), ok=False)
        self.assertFalse((self.source / 'home/private_mapped.conf.tmpl').exists())
        self.live.write_text('password = fixture-secret\n')
        path, plan, review = self.plan(dict(version=1, mode='track', profile='default', selections=[selection]))
        self.assertNotIn('fixture-secret', review)
        self.cli('intent', 'apply', str(path), '--approve', plan['id'])
        self.assertEqual((self.source / 'home/private_mapped.conf.tmpl').read_text(), 'theme = {{ .chezmoi.os }}\n')
        self.assertEqual(self.live.read_text(), 'password = fixture-secret\n')

    def test_recovery_can_finalize_exact_committed_source_scope_transaction_after_crash(self):
        path, plan, _ = self.plan(dict(version=1, mode='untrack', profile='default', entries=['prefs']))
        self.cli('intent', 'apply', str(path), '--approve', plan['id'])
        backup = self.home / '.local/state/dot/recovery' / plan['id']
        (backup / 'transaction.json').write_text(json.dumps({'status': 'pending', 'live_files': 'unchanged'}))
        self.live.write_text('later local work\n')
        before = self.git('rev-parse', 'HEAD'), self.scope.read_bytes(), self.live.read_bytes(), self.live.stat().st_mode
        self.cli('intent', 'recover', str(path), '--approve', plan['id'])
        self.assertEqual((self.git('rev-parse', 'HEAD'), self.scope.read_bytes(), self.live.read_bytes(), self.live.stat().st_mode), before)
        self.assertEqual(json.loads((backup / 'transaction.json').read_text())['status'], 'complete')

    def test_missing_scope_recovery_backup_never_discards_existing_approval(self):
        path, plan, _ = self.plan(dict(version=1, mode='untrack', profile='default', entries=['prefs']))
        backup = self.home / '.local/state/dot/recovery' / plan['id']
        backup.mkdir(parents=True)
        (backup / 'plan.json').write_text(json.dumps(plan))
        (backup / 'transaction.json').write_text(json.dumps({'status': 'pending', 'live_files': 'unchanged'}))
        (backup / 'index.before').write_bytes((self.source / '.git/index').read_bytes())
        for i, action in enumerate(plan['actions']):
            if action['before']:
                (backup / str(i)).write_bytes((self.source / action['path']).read_bytes())
        before = self.scope.read_bytes()
        self.cli('intent', 'recover', str(path), '--approve', plan['id'], ok=False)
        self.assertEqual(self.scope.read_bytes(), before)

    def test_stop_tracking_bounded_git_reviews_and_removes_private_preferences_not_live_git_config(self):
        entry = dict(destination='.gitconfig', source='home/modify_dot_gitconfig.tmpl', os=['darwin', 'linux'],
                     portability='managed-templated', impact='Unsafe', removals=[], hooks=[], tools=[],
                     integrations=['git-config'], git_keys=['user.name'], git_source='home/.chezmoitemplates/gitprefs.tmpl')
        self.catalog['entries'] = {'git': entry}
        self.catalog['profiles'] = {'default': ['git']}
        wrapper = self.source / entry['source']
        wrapper.write_text('bounded fixture wrapper\n')
        prefs = self.source / entry['git_source']
        prefs.parent.mkdir(parents=True)
        prefs.write_text('[user]\n name = Fixture\n')
        live = self.home / '.gitconfig'
        live.write_text('[user]\n name = Local\n[credential]\n helper = keep-local\n')
        live.chmod(0o600)
        before = live.read_bytes(), live.stat().st_mode
        self.save_catalog()
        self.commit()
        self.scope.write_text(json.dumps(dict(version=2, profile='default', entries={'git': entry})))
        path, plan, review = self.plan(dict(version=1, mode='untrack', profile='default', entries=['git']))
        self.assertIn(entry['git_source'], review)
        self.cli('intent', 'apply', str(path), '--approve', plan['id'], '--ack-unsafe')
        self.assertFalse(wrapper.exists())
        self.assertFalse(prefs.exists())
        self.assertEqual((live.read_bytes(), live.stat().st_mode), before)

    def test_concurrent_unrelated_staged_work_is_never_committed_or_discarded(self):
        path, plan, _ = self.plan(dict(version=1, mode='untrack', profile='default', entries=['prefs']))
        real_git = shutil.which('git')
        bin_dir = self.base / 'bin'
        bin_dir.mkdir()
        wrapper = bin_dir / 'git'
        wrapper.write_text('#!/bin/sh\n' + '"' + real_git + '" "$@"\ncode=$?\n' +
                           'for arg do\n if [ "$arg" = add ]; then\n' +
                           'printf "user work\\n" > "$DOT_SOURCE/user-work"\n' +
                           '"' + real_git + '" -C "$DOT_SOURCE" add user-work\n fi\ndone\nexit "$code"\n')
        wrapper.chmod(0o755)
        self.env['PATH'] = str(bin_dir) + os.pathsep + self.env['PATH']
        self.cli('intent', 'apply', str(path), '--approve', plan['id'], ok=False)
        self.assertEqual(self.git('rev-parse', 'HEAD').strip(), plan['head'])
        self.assertEqual((self.source / 'user-work').read_text(), 'user work\n')
        self.assertIn('user-work', self.git('diff', '--cached', '--name-only'))

    def test_ambiguous_same_layer_metadata_is_rejected_instead_of_last_field_winning(self):
        self.metadata_fixture()
        current = 'linux' if sys.platform.startswith('linux') else sys.platform
        request = self.base / 'request.json'
        request.write_text('{"version":1,"mode":"track","profile":"default","selections":[{"path":"prefs.conf","logical":"mapped","destinations":{"' + current + '":"mapped.conf"},"common":{"impact":"Unsafe","impact":"Safe","portability":"managed-shared"}}]}')
        self.cli('intent', 'plan', str(request), ok=False)
        self.assertFalse((self.source / 'home/mapped.conf').exists())

    def test_advertised_tracking_capability_cannot_omit_runtime_module_even_for_legacy_source(self):
        (self.engine / 'tracking.py').unlink()
        self.cli('contract', 'check', '--engine', str(self.engine), '--source', str(self.source), ok=False)
        self.assertTrue((self.source / 'home/prefs.conf').exists())

    def test_boolean_request_and_local_approval_versions_are_not_integer_schema_revisions(self):
        request = self.base / 'request.json'
        request.write_text(json.dumps(dict(version=True, mode='untrack', profile='default', entries=['prefs'])))
        self.cli('intent', 'plan', str(request), ok=False)
        self.scope.write_text(json.dumps({'version': True, 'groups': {}}))
        request.write_text(json.dumps(dict(version=1, mode='adopt', profile='default', entries=['prefs'])))
        self.cli('intent', 'plan', str(request), ok=False)
        self.assertEqual(self.live.read_text(), 'live preference\n')

    def test_removal_collision_with_unselected_generated_output_fails_before_mutation(self):
        self.metadata_fixture()
        self.catalog['entries']['skill'] = dict(destination='.agents/skills/fixture/SKILL.md', source='home/dot_agents/skills/fixture/SKILL.md',
                                              os=['darwin', 'linux'], portability='managed-shared', impact='Safe',
                                              removals=[], hooks=[], tools=[], integrations=['gemini-skills'])
        source = self.source / 'home/dot_agents/skills/fixture/SKILL.md'
        source.parent.mkdir(parents=True)
        source.write_text('synthetic skill\n')
        self.save_catalog()
        self.commit()
        current = 'linux' if sys.platform.startswith('linux') else sys.platform
        request = self.base / 'request.json'
        request.write_text(json.dumps(dict(version=1, mode='track', profile='default', selections=[{
            'path': 'prefs.conf', 'logical': 'mapped', 'destinations': {current: 'mapped.conf'},
            'common': {'impact': 'Safe', 'portability': 'managed-shared', 'removals': ['.gemini/skills/fixture/SKILL.md']}}])))
        self.cli('intent', 'plan', str(request), ok=False)
        self.assertFalse((self.source / 'home/mapped.conf').exists())

    def test_new_os_mapping_requires_explicit_track_and_never_enrolls_target_or_overwrites_existing_source(self):
        self.metadata_fixture()
        current = 'linux' if sys.platform.startswith('linux') else sys.platform
        other = 'linux' if current == 'darwin' else 'darwin'
        selection = {'path': 'prefs.conf', 'logical': 'mapped', 'destinations': {current: 'mapped.conf'},
                     'common': {'impact': 'Safe', 'portability': 'managed-shared'}}
        path, plan, _ = self.plan(dict(version=1, mode='track', profile='default', selections=[selection]))
        self.cli('intent', 'apply', str(path), '--approve', plan['id'])
        original = (self.source / 'home/mapped.conf').read_bytes()
        self.live.write_text('explicit new OS content\n')
        scope_before = self.scope.read_bytes()
        selection['destinations'][other] = 'other-os.conf'
        path, plan, _ = self.plan(dict(version=1, mode='track', profile='default', selections=[selection]))
        self.cli('intent', 'apply', str(path), '--approve', plan['id'])
        self.assertEqual((self.source / 'home/mapped.conf').read_bytes(), original)
        self.assertEqual((self.source / 'home/other-os.conf').read_text(), 'explicit new OS content\n')
        self.assertFalse((self.home / 'other-os.conf').exists())
        self.assertEqual(self.scope.read_bytes(), scope_before)

    def test_commit_is_path_bounded_even_when_unrelated_work_is_staged_at_commit_boundary(self):
        path, plan, _ = self.plan(dict(version=1, mode='untrack', profile='default', entries=['prefs']))
        real_git = shutil.which('git')
        bin_dir = self.base / 'bin'
        bin_dir.mkdir()
        wrapper = bin_dir / 'git'
        wrapper.write_text('#!/bin/sh\nfor arg do\n if [ "$arg" = commit ]; then\n' +
                           'printf "user work\\n" > "$DOT_SOURCE/user-work"\n' +
                           '"' + real_git + '" -C "$DOT_SOURCE" add user-work\n fi\ndone\n' +
                           'exec "' + real_git + '" "$@"\n')
        wrapper.chmod(0o755)
        self.env['PATH'] = str(bin_dir) + os.pathsep + self.env['PATH']
        self.cli('intent', 'apply', str(path), '--approve', plan['id'])
        self.assertNotIn('user-work', self.git('diff', '--name-only', plan['head'], 'HEAD'))
        self.assertIn('user-work', self.git('diff', '--cached', '--name-only'))
        self.assertEqual((self.source / 'user-work').read_text(), 'user work\n')


if __name__ == '__main__':
    unittest.main()
