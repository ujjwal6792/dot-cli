#!/usr/bin/env python3
"""External JSON bridge contract tests; all paths and HOME are fixtures."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


REPO = Path(__file__).resolve().parents[1]


class BrowserApiTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.base = Path(self.tmp.name)
        self.home = self.base / 'home'
        self.engine = self.base / 'public' / 'tools' / 'dot'
        self.source = self.base / 'private'
        self.home.mkdir(parents=True)
        self.engine.mkdir(parents=True)
        (self.source / 'home').mkdir(parents=True)
        (self.source / 'tools' / 'dot').mkdir(parents=True)
        subprocess.run(['git', 'init', '-q', str(self.source)], check=True)
        subprocess.run(['git', '-C', str(self.source), 'config', 'user.name', 'Fixture'], check=True)
        subprocess.run(['git', '-C', str(self.source), 'config', 'user.email', 'fixture@example.invalid'], check=True)
        shutil.copy(REPO / 'tools' / 'dot' / 'scope.py', self.source / 'tools' / 'dot' / 'scope.py')
        for name in ('contract.py', 'intent.py', 'scope.py', 'gitconfig.py', 'skills.py', 'layout.py', 'browser_api.py', 'tracking.py', 'file_metadata.py', 'packages.py'):
            shutil.copy(REPO / 'tools' / 'dot' / name, self.engine / name)
        shutil.copy(REPO / 'tools' / 'dot' / 'engine.json', self.engine / 'engine.json')
        shutil.copy(REPO / 'tools' / 'dot' / 'dot', self.engine / 'dot')
        self.manifest = {'version': 1, 'entries': {}, 'profiles': {}}
        self.write_manifest()
        (self.source / 'dot-contract.json').write_text(json.dumps({
            'protocol': 1, 'engine_api': {'min': 1, 'max': 1},
            'manifest_version': 1, 'approval_version': 2,
            'manifest': 'tools/dot/manifest.json', 'home_source': 'home',
            'required_adapters': ['intent', 'scope', 'gitconfig', 'skills', 'layout']}))
        subprocess.run(['git', '-C', str(self.source), 'add', '.'], check=True)
        subprocess.run(['git', '-C', str(self.source), 'commit', '-qm', 'fixture'], check=True)
        self.env = dict(os.environ, HOME=str(self.home), DOT_ENGINE='/not-a-default',
                        DOT_SOURCE='/not-a-default', PYTHONPATH=str(self.engine))
        tracker = self.base / 'tracker.toml'
        tracker.write_text('sourceDir=' + json.dumps(str(self.source / 'home')) +
                           '\n[data]\ndotEngine=' + json.dumps(str(self.engine)) + '\n')
        self.env['DOT_CONFIG'] = str(tracker)

    def tearDown(self):
        self.tmp.cleanup()

    def write_manifest(self):
        path = self.source / 'tools' / 'dot' / 'manifest.json'
        path.write_text(json.dumps(self.manifest))

    def call(self, payload):
        proc = subprocess.run([sys.executable, str(self.engine / 'browser_api.py')],
                              input=json.dumps(payload) + '\n', text=True,
                              capture_output=True, env=self.env)
        self.assertNotIn('Traceback', proc.stderr)
        self.assertTrue(proc.stdout.strip(), proc.stderr)
        response = json.loads(proc.stdout)
        if proc.returncode:
            self.fail(f"bridge rejected fixture: {response.get('error')}")
        return proc, response

    def payload(self, command, **kw):
        return dict(version=1, command=command, engine=str(self.engine),
                    source=str(self.source), **kw)

    def test_packages_private_editor_and_invalid_approval_are_inert(self):
        (self.source / 'packages').mkdir()
        path = self.source / 'packages/Brewfile'
        path.write_text('# fixture\nbrew "fixture-cli"\n')
        import hashlib
        original = hashlib.sha256(path.read_bytes()).hexdigest()
        _, response = self.call(self.payload('packages', request={'action': 'load', 'manager': 'brew', 'entrypoint': 'browse'}))
        self.assertEqual(response['result']['digest'], original)
        _, response = self.call(self.payload('packages', request={'action': 'save', 'manager': 'brew', 'digest': original,
                                    'text': '# fixture\nbrew "fixture-tmux"\n', 'entrypoint': 'browse'}))
        self.assertIn('fixture-tmux', path.read_text())
        self.assertEqual(len(list((path.parent / '.backups').iterdir())), 1)
        payload = self.payload('packages', request={'action': 'apply', 'entrypoint': 'browse'}, plan={'id': 'invented'}, approve='')
        proc = subprocess.run([sys.executable, str(self.engine / 'browser_api.py')], input=json.dumps(payload)+'\n', text=True, capture_output=True, env=self.env)
        self.assertNotEqual(proc.returncode, 0)
        self.assertIn('approval', json.loads(proc.stdout)['error'])

    def test_packages_editor_transport_preserves_unknown_source_bytes(self):
        (self.source / 'packages').mkdir()
        text = '# fixture\r\nbrew "fixture-cli"\r\nsystem("fixture\u200dvalue")\r\n'
        (self.source / 'packages/Brewfile').write_bytes(text.encode())
        _, response = self.call(self.payload('packages', request={'action':'load', 'manager':'brew', 'entrypoint':'browse'}))
        self.assertEqual(response['result']['text'], text)
        self.assertEqual(response['result']['diagnostics'], ['line 3: unsupported syntax; install blocked'])

    def test_packages_capability_cannot_import_private_code_or_use_missing_module(self):
        sentinel = self.home / 'private-module-executed'
        private_code = self.source / 'private-code.py'
        private_code.write_text('from pathlib import Path\nPath(' + repr(str(sentinel)) + ').write_text("executed")\n')
        module = self.engine / 'packages.py'
        module.unlink()
        for symlink in (True, False):
            if symlink:
                module.symlink_to(private_code)
            payload = self.payload('packages', request={'action':'load', 'manager':'brew', 'entrypoint':'browse'})
            proc = subprocess.run([sys.executable, str(self.engine / 'browser_api.py')], input=json.dumps(payload)+'\n', text=True, capture_output=True, env=self.env)
            self.assertFalse(sentinel.exists(), 'private module executed through public capability')
            self.assertNotEqual(proc.returncode, 0)
            self.assertIn('update compatible public engine', json.loads(proc.stdout)['error'])
            if symlink:
                module.unlink()

    def test_inventory_tracks_hierarchy_exclusions_symlinks_and_adopt_metadata(self):
        parent = self.home / '.config' / 'sample'
        parent.mkdir(parents=True)
        (parent / 'prefs.conf').write_text('theme = light\n')
        (parent / 'auth.json').write_text('never disclose secret\n')
        outside = self.base / 'outside'
        outside.write_text('outside\n')
        (parent / 'escape').symlink_to(outside)
        dest = '.config/sample/prefs.conf'
        entry = {'destination': dest, 'source': 'home/dot_config/sample/prefs.conf',
                 'os': ['darwin', 'linux'], 'portability': 'managed-shared',
                 'impact': 'Safe', 'removals': [], 'hooks': [], 'tools': []}
        self.manifest['entries'][dest] = entry
        self.write_manifest()
        proc, response = self.call(self.payload('inventory'))
        self.assertEqual(proc.returncode, 0, proc.stderr)
        rows = response['result']['entries']
        self.assertTrue(any(row['path'] == '.config' and row['kind'] == 'directory' for row in rows))
        self.assertTrue(any(row['path'] == '.config/sample/auth.json' and row['status'] == 'excluded' for row in rows))
        link = next(row for row in rows if row['path'] == '.config/sample/escape')
        self.assertEqual(link['status'], 'symlink')
        adopted = next(row for row in rows if row['path'] == dest)
        self.assertEqual(adopted['destination'], dest)
        self.assertEqual(adopted['metadata']['impact'], 'Safe')
        self.assertNotIn('never disclose', proc.stdout)
        self.assertTrue(any(row['path'] == '.config/sample/auth.json' and row['status'] == 'excluded'
                            for row in rows))

    def test_track_plan_returns_safe_directional_diff_and_exact_id(self):
        (self.home / '.config').mkdir()
        (self.home / '.config' / 'prefs.conf').write_text('theme = light\n')
        request = {'version': 1, 'mode': 'track', 'profile': 'fixture',
                   'selections': [{'path': '.config/prefs.conf', 'os': ['darwin', 'linux'],
                                   'portability': 'managed-shared', 'impact': 'Safe'}]}
        proc, response = self.call(self.payload('plan', request=request))
        self.assertEqual(proc.returncode, 0, proc.stderr)
        result = response['result']
        request_path = self.base / 'request.json'
        request_path.write_text(json.dumps(request))
        plan_path = self.base / 'plan.json'
        cli = subprocess.run(['bash', str(self.engine / 'dot'), 'intent', 'plan',
                              str(request_path), '--out', str(plan_path)],
                             env=dict(self.env, DOT_ENGINE=str(self.engine),
                                      DOT_SOURCE=str(self.source)),
                             text=True, capture_output=True)
        self.assertEqual(cli.returncode, 0, cli.stderr)
        self.assertEqual(result['plan']['id'], json.loads(plan_path.read_text())['id'])
        self.assertEqual(result['direction'], 'HOME → repository')
        self.assertIn('theme = light', json.dumps(result['diffs']))
        self.assertIn('\n', result['diffs'][0]['text'])
        self.assertIn('json', result['diffs'][0])
        self.assertFalse((self.source / 'home' / 'dot_config' / 'prefs.conf').exists())

    def test_secret_terminal_and_binary_content_never_appear_in_preview(self):
        (self.home / '.config').mkdir()
        live = self.home / '.config' / 'prefs.conf'
        live.write_text('password = "fixture-secret-value"\n')
        request = {'version': 1, 'mode': 'track', 'profile': 'fixture',
                   'selections': [{'path': '.config/prefs.conf', 'os': ['darwin', 'linux'],
                                   'portability': 'managed-shared', 'impact': 'Safe'}]}
        proc, response = self.call(self.payload('inventory'))
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertNotIn('fixture-secret-value', proc.stdout)
        self.assertTrue(any(row.get('status') == 'restricted' for row in response['result']['entries']))
        live.write_bytes(b'\x00binary\x1b[31m')
        proc, response = self.call(self.payload('inventory'))
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertNotIn('binary', proc.stdout)
        self.assertNotIn('\x1b', proc.stdout)

    def test_contract_must_be_explicit_independent_and_apply_uses_reviewed_id_consents(self):
        (self.home / '.config').mkdir()
        (self.home / '.config' / 'prefs.conf').write_text('theme = light\n')
        request = {'version': 1, 'mode': 'track', 'profile': 'fixture',
                   'selections': [{'path': '.config/prefs.conf', 'os': ['darwin', 'linux'],
                                   'portability': 'managed-shared', 'impact': 'Unsafe'}]}
        _, planned = self.call(self.payload('plan', request=request))
        plan = planned['result']['plan']
        payload = self.payload('apply', plan=plan, approve=plan['id'])
        payload['ack_unsafe'] = False
        raw = subprocess.run([sys.executable, str(self.engine / 'browser_api.py')],
                             input=json.dumps(payload) + '\n', text=True,
                             capture_output=True, env=self.env)
        denied = json.loads(raw.stdout)
        self.assertNotEqual(raw.returncode, 0)
        self.assertFalse((self.source / 'home' / 'dot_config' / 'prefs.conf').exists())
        payload['ack_unsafe'] = True
        proc, applied = self.call(payload)
        self.assertEqual(proc.returncode, 0, applied)
        self.assertTrue((self.source / 'home' / 'dot_config' / 'prefs.conf').exists())
        payload['engine'] = str(self.source / 'tools' / 'dot')
        proc = subprocess.run([sys.executable, str(self.engine / 'browser_api.py')],
                              input=json.dumps(payload) + '\n', text=True,
                              capture_output=True, env=self.env)
        rejected = json.loads(proc.stdout)
        self.assertNotEqual(proc.returncode, 0)
        self.assertFalse(rejected['ok'])

    def test_preview_rechecks_freshness_and_rejects_changed_plan(self):
        (self.home / '.config').mkdir()
        live = self.home / '.config' / 'prefs.conf'
        live.write_text('theme = light\n')
        request = {'version': 1, 'mode': 'track', 'profile': 'fixture',
                   'selections': [{'path': '.config/prefs.conf', 'os': ['darwin', 'linux'],
                                   'portability': 'managed-shared', 'impact': 'Safe'}]}
        _, planned = self.call(self.payload('plan', request=request))
        plan = dict(planned['result']['plan'])
        # API exposes opaque id only; fixture retains full plan from internal test helper.
        _, preview = self.call(self.payload('preview', plan=plan))
        self.assertEqual(preview['result']['plan_id'], plan['id'])
        live.write_text('theme = dark\n')
        proc = subprocess.run([sys.executable, str(self.engine / 'browser_api.py')],
                              input=json.dumps(self.payload('preview', plan=plan)) + '\n',
                              text=True, capture_output=True, env=self.env)
        self.assertNotEqual(proc.returncode, 0)
        self.assertIn('stale/changed plan', json.loads(proc.stdout)['error'])

    def test_preview_rejects_tampered_actions_even_when_id_and_request_match(self):
        (self.home / 'prefs.conf').write_text('safe\n')
        request = {'version': 1, 'mode': 'track', 'profile': 'fixture',
                   'selections': [{'path': 'prefs.conf', 'os': ['darwin', 'linux'],
                                   'portability': 'managed-shared', 'impact': 'Safe'}]}
        _, planned = self.call(self.payload('plan', request=request))
        plan = planned['result']['plan']
        plan['actions'][0]['text'] = 'private unrelated data\n'
        proc = subprocess.run([sys.executable, str(self.engine / 'browser_api.py')],
                              input=json.dumps(self.payload('preview', plan=plan)) + '\n',
                              text=True, capture_output=True, env=self.env)
        self.assertNotEqual(proc.returncode, 0)
        self.assertNotIn('private unrelated data', proc.stdout)

    def test_inventory_adopt_includes_missing_manifest_destinations_and_scope_os(self):
        self.manifest['entries']['.config/missing.conf'] = {
            'destination': '.config/missing.conf', 'source': 'home/dot_config/missing.conf',
            'os': ['darwin'], 'portability': 'managed-shared', 'impact': 'Safe',
            'removals': [], 'hooks': [], 'tools': []}
        self.write_manifest()
        proc, response = self.call(self.payload('inventory', mode='adopt'))
        self.assertEqual(proc.returncode, 0)
        row = next(x for x in response['result']['entries'] if x['path'] == '.config/missing.conf')
        self.assertEqual(row['status'], 'managed')
        self.assertEqual(row['kind'], 'file')
        self.assertEqual(row['id'], '.config/missing.conf')
        self.assertTrue(any(x.get('logical') and x['path'] == '.config' for x in response['result']['entries']))
        expected_os = 'supported' if sys.platform in ('darwin', 'linux') else 'unsupported'
        self.assertEqual(row['os_status'], expected_os)
        self.assertFalse(row['approved'])

    def test_track_inventory_does_not_descend_excluded_or_dot_git_directories(self):
        (self.home / '.ssh').mkdir()
        (self.home / '.ssh' / 'id_ed25519').write_text('private fixture')
        (self.home / '.config').mkdir()
        (self.home / '.config' / '.git').mkdir()
        (self.home / '.config' / '.git' / 'secret').write_text('fixture')
        (self.home / '.config' / 'visible').write_text('ok')
        proc, response = self.call(self.payload('inventory'))
        paths = {x['path'] for x in response['result']['entries']}
        self.assertEqual(proc.returncode, 0)
        self.assertIn('.ssh', paths)
        self.assertNotIn('.ssh/id_ed25519', paths)
        self.assertNotIn('.config/.git/secret', paths)
        self.assertNotIn('.config/.git', paths)

    def test_track_inventory_prunes_excluded_and_unrelated_repository_trees(self):
        (self.home / '.ssh').mkdir()
        (self.home / '.ssh' / 'private-label').write_text('fixture only')
        (self.home / '.cache').mkdir()
        (self.home / '.cache' / 'private-label').write_text('fixture only')
        project = self.home / 'project'
        (project / '.git').mkdir(parents=True)
        (project / 'owner-local-file').write_text('fixture only')
        proc, response = self.call(self.payload('inventory'))
        self.assertEqual(proc.returncode, 0)
        self.assertNotIn('private-label', proc.stdout)
        self.assertNotIn('owner-local-file', proc.stdout)
        self.assertTrue(any(e['status'] == 'excluded' for e in response['result']['entries']))

    def test_snapshot_is_exact_uses_shared_exclusion_and_skips_mixed_unreadable_files(self):
        folder = self.home / '.config' / 'snapshot'
        folder.mkdir(parents=True)
        (folder / 'ok.conf').write_text('theme = light\n')
        (folder / 'auth.json').write_text('secret fixture')
        (folder / 'binary').write_bytes(b'\0\xff')
        repo = folder / 'unrelated'
        (repo / '.git').mkdir(parents=True)
        (repo / 'owner-data').write_text('do not snapshot')
        proc, response = self.call(self.payload('snapshot', path='.config/snapshot'))
        self.assertEqual(proc.returncode, 0, response)
        self.assertEqual(response['result']['paths'], ['.config/snapshot/ok.conf'])
        self.assertEqual(response['result']['skipped'], 2)
        self.assertNotIn('unrelated/owner-data', response['result']['paths'])

    def test_gates_effects_review_labels_and_paths_are_sanitized(self):
        unsafe = {'version': 1, 'mode': 'track', 'profile': 'fixture',
                  'selections': [{'path': 'prefs\x1b[31m.conf', 'os': ['darwin'],
                                  'portability': 'managed-shared', 'impact': 'Unsafe'}]}
        (self.home / 'prefs\x1b[31m.conf').write_text('ok\x1b[31m\n')
        proc = subprocess.run([sys.executable, str(self.engine / 'browser_api.py')],
                              input=json.dumps(self.payload('plan', request=unsafe)) + '\n',
                              text=True, capture_output=True, env=self.env)
        self.assertNotEqual(proc.returncode, 0)
        self.assertNotIn('\x1b', proc.stdout)
        (self.home / 'prefs.conf').write_text('ok\n')
        req = dict(unsafe, selections=[dict(unsafe['selections'][0], path='prefs.conf')])
        proc, result = self.call(self.payload('plan', request=req))
        review = result['result']
        self.assertTrue(review['unsafe'])
        self.assertTrue(review['gates_known'])
        self.assertTrue(review['diffs'][0]['summary'])
        self.assertIn('REPOSITORY', review['diffs'][0]['text'])
        self.assertNotIn('prefs\x1b', proc.stdout)

    def test_highlight_blocked_file_returns_safe_noncontent_summary(self):
        (self.home / 'bad.conf').write_bytes(b'\0secret\x1b')
        proc, result = self.call(self.payload('highlight', path='bad.conf'))
        self.assertEqual(proc.returncode, 0)
        self.assertEqual(result['result']['status'], 'unsupported')
        self.assertNotIn('secret', proc.stdout)
        self.assertNotIn('\x1b', proc.stdout)

    def test_explicit_home_must_exist_before_canonicalization(self):
        proc = subprocess.run([sys.executable, str(self.engine / 'browser_api.py')],
                              input=json.dumps(self.payload('inventory', home=str(self.base / 'missing'))) + '\n',
                              text=True, capture_output=True, env=self.env)
        self.assertNotEqual(proc.returncode, 0)
        self.assertNotIn('missing', proc.stdout)

    def test_stop_tracking_inventory_review_and_apply_are_source_only(self):
        entry = {'destination': 'prefs.conf', 'source': 'home/prefs.conf', 'os': ['darwin', 'linux'],
                 'portability': 'managed-shared', 'impact': 'Safe', 'removals': [], 'hooks': [], 'tools': []}
        self.manifest['entries']['prefs'] = entry
        self.manifest['profiles']['default'] = ['prefs']
        self.write_manifest()
        (self.source / 'home/prefs.conf').write_text('private source\n')
        live = self.home / 'prefs.conf'
        live.write_text('live file\n')
        live.chmod(0o640)
        subprocess.run(['git', '-C', str(self.source), 'add', '.'], check=True)
        subprocess.run(['git', '-C', str(self.source), 'commit', '-qm', 'tracking fixture'], check=True)
        state = self.home / '.local/state/dot/adoption.json'
        state.parent.mkdir(parents=True)
        state.write_text(json.dumps({'version': 2, 'profile': 'default', 'entries': {'prefs': entry}}))
        _, response = self.call(self.payload('inventory', mode='untrack'))
        row = next(e for e in response['result']['entries'] if e['path'] == 'prefs.conf')
        self.assertEqual(row['id'], 'prefs')
        self.assertTrue(row['approved'])
        request = {'version': 1, 'mode': 'untrack', 'profile': 'default', 'entries': ['prefs']}
        _, response = self.call(self.payload('plan', request=request))
        result = response['result']
        self.assertIn('live files unchanged', result['direction'])
        self.assertTrue(all('REPOSITORY/' in d['text'] for d in result['diffs']))
        plan = result['plan']
        self.call(self.payload('apply', plan=plan, approve=plan['id']))
        self.assertEqual(live.read_bytes(), b'live file\n')
        self.assertEqual(live.stat().st_mode & 0o777, 0o640)
        self.assertEqual(json.loads(state.read_text())['entries'], {})


if __name__ == '__main__':
    unittest.main()
