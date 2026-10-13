#!/usr/bin/env python3
"""Assignment behavior through the public engine bridge, not policy internals."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ENGINE = Path(__file__).resolve().parents[1] / 'tools/dot'


class AssignmentsTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.base = Path(self.tmp.name)
        self.home = self.base / 'target'
        self.home.mkdir()
        self.root = self.base / 'source'
        (self.root / 'home').mkdir(parents=True)
        self.system = 'darwin' if sys.platform == 'darwin' else 'linux'
        self.entry = dict(destination='prefs.conf', source='home/prefs.conf', os=[self.system],
                          portability='managed-shared', impact='Safe', removals=[], hooks=[], tools=[])
        self.catalog = dict(version=1, entries={'prefs': self.entry}, profiles={'base': ['prefs']})
        (self.root / 'dot-contract.json').write_text(json.dumps(dict(protocol=1,
            engine_api=dict(min=1, max=1), manifest_version=1, approval_version=2,
            manifest='manifest.json', home_source='home',
            required_adapters=['intent', 'scope', 'gitconfig', 'skills', 'layout'])))
        (self.root / 'home/prefs.conf').write_text('shared preference\n')
        self.env = dict(os.environ, HOME=str(self.home))
        for key in ('DOT_STATE_DIR', 'XDG_STATE_HOME'):
            self.env.pop(key, None)
        self.persist()
        self.call('init-machine', request=dict(action='save', id='local_machine'))

    def tearDown(self):
        self.tmp.cleanup()

    def persist(self):
        (self.root / 'manifest.json').write_text(json.dumps(self.catalog))
        for args in [('init', '-q'), ('add', '.'), ('-c', 'user.name=Fixture', '-c',
                     'user.email=fixture@example.invalid', 'commit', '--allow-empty', '-qm', 'fixture')]:
            subprocess.run(['git', '-C', str(self.root), *args], check=True, capture_output=True)

    def call(self, command, ok=True, engine=ENGINE, **fields):
        payload = dict(version=1, command=command, engine=str(engine), source=str(self.root),
                       home=str(self.home), **fields)
        result = subprocess.run([sys.executable, '-B', str(engine / 'browser_api.py')],
                                input=json.dumps(payload), env=self.env, text=True, capture_output=True)
        response = json.loads(result.stdout)
        self.assertEqual(response['ok'], ok, response)
        return response.get('result', response.get('error'))

    def plan(self, **fields):
        return self.call('plan', request=dict(version=1, mode='adopt', profile='base', **fields))['plan']

    def apply(self, plan):
        self.call('apply', plan=plan, approve=plan['id'])

    def test_track_defaults_new_entry_to_all(self):
        (self.home / 'custom.conf').write_text('local preference\n')
        selection = dict(path='custom.conf', os=[self.system], portability='managed-shared', impact='Safe')
        plan = self.call('plan', request=dict(version=1, mode='track', profile='local', selections=[selection]))['plan']
        self.assertEqual(plan['selected']['custom.conf']['machines'], dict(include=['all'], exclude=[]))

    def test_plan_rejects_invalid_assignments_without_writes(self):
        for assignment in [dict(include=[], exclude=[]), dict(include=['*'], exclude=[]),
                           dict(include=['local_machine'], exclude=['local_*']),
                           dict(include=['all'], exclude=[], future=True),
                           dict(include=['all']), dict(include='all', exclude=[]),
                           dict(include=['all', 'all'], exclude=[]),
                           dict(include=[False], exclude=[]), dict(include=['all'], exclude=['bad.id'])]:
            with self.subTest(assignment=assignment):
                self.entry['machines'] = assignment
                self.persist()
                before = (self.root / 'manifest.json').read_bytes()
                error = self.call('plan', ok=False, request=dict(version=1, mode='adopt', profile='base'))
                self.assertIn('assignment:', error)
                self.assertEqual((self.root / 'manifest.json').read_bytes(), before)
                self.assertFalse((self.home / 'prefs.conf').exists())

    def test_assignments_filter_profiles_but_explicit_ineligible_selection_fails(self):
        for assignment in [dict(include=['other_machine'], exclude=[]),
                           dict(include=['all'], exclude=['local_machine']),
                           dict(include=['all'], exclude=['all'])]:
            self.entry['machines'] = assignment
            self.persist()
            self.assertEqual(self.plan()['selected'], {})
            self.call('plan', ok=False, request=dict(version=1, mode='adopt', profile='base', entries=['prefs']))
        self.entry['machines'] = dict(include=['local_machine'], exclude=['other_machine'])
        self.persist()
        self.assertEqual(list(self.plan()['selected']), ['prefs'])

    def test_adopt_composes_named_profiles_as_union_without_editing_manifest(self):
        self.catalog['entries']['extra'] = dict(self.entry, destination='extra.conf', source='home/extra.conf')
        self.catalog['profiles']['extra'] = ['prefs', 'extra']
        (self.root / 'home/extra.conf').write_text('extra preference\n')
        self.persist()
        before = (self.root / 'manifest.json').read_bytes()
        plan = self.plan(profiles=['base', 'extra'])
        self.assertEqual(sorted(plan['selected']), ['extra', 'prefs'])
        self.assertFalse(any(a.get('repository') for a in plan['actions']))
        self.apply(plan)
        self.assertEqual((self.root / 'manifest.json').read_bytes(), before)
        self.assertEqual((self.home / 'extra.conf').read_text(), 'extra preference\n')
        state = json.loads((self.home / '.local/state/dot/adoption.json').read_text())
        self.assertEqual(state['profiles'], ['base', 'extra'])
        self.assertEqual(sorted(self.plan()['selected']), ['extra', 'prefs'])

    def test_deselections_and_custom_metadata_are_machine_local_and_survive_update(self):
        self.entry['machines'] = dict(include=['all'], exclude=[])
        self.catalog['entries']['extra'] = dict(self.entry, destination='extra.conf', source='home/extra.conf')
        self.catalog['profiles']['base'].append('extra')
        (self.root / 'home/extra.conf').write_text('extra preference\n')
        self.persist()
        before = (self.root / 'manifest.json').read_bytes()
        plan = self.plan(entries=['prefs'], metadata={'prefs': {'tools': ['local-tool']}})
        self.assertFalse(any(a.get('repository') for a in plan['actions']))
        self.apply(plan)
        self.assertEqual((self.root / 'manifest.json').read_bytes(), before)
        state = json.loads((self.home / '.local/state/dot/adoption.json').read_text())
        self.assertEqual(state['deselected'], ['extra'])
        self.assertEqual(state['customs'], {'prefs': {'tools': ['local-tool']}})
        self.assertEqual(self.plan()['selected']['prefs']['tools'], ['local-tool'])
        self.assertNotIn('extra', self.plan()['selected'])
        update = self.call('plan', request=dict(version=1, mode='update', profile='base'))['plan']
        self.assertEqual(list(update['selected']), ['prefs'])
        self.apply(update)
        self.assertEqual((self.root / 'manifest.json').read_bytes(), before)
        self.home = self.base / 'second'
        self.home.mkdir()
        self.env['HOME'] = str(self.home)
        self.call('init-machine', request=dict(action='save', id='other_machine'))
        other = self.plan()
        self.assertEqual(sorted(other['selected']), ['extra', 'prefs'])
        self.assertEqual(other['selected']['prefs']['tools'], [])

    def test_logical_track_defaults_assignments_and_can_retarget_with_a_track_plan(self):
        self.catalog.update(version=2, machines={'local_machine': dict(label='Local', os=self.system),
                                               'other_machine': dict(label='Other', os=self.system)}, tracking={})
        path = self.root / 'dot-contract.json'
        descriptor = json.loads(path.read_text())
        descriptor.update(manifest_version=2, approval_version=3)
        path.write_text(json.dumps(descriptor))
        self.persist()
        (self.home / 'logical.conf').write_text('local logical preference\n')
        selection = dict(path='logical.conf', logical='logical', destinations={self.system: 'logical.conf'},
                         common={}, machine={}, os_metadata={})
        request = dict(version=1, mode='track', profile='logical', selections=[selection])
        plan = self.call('plan', request=request)['plan']
        self.assertEqual(plan['selected']['logical@' + self.system]['machines'], dict(include=['all'], exclude=[]))
        self.apply(plan)
        selection['machines'] = dict(include=['other_machine'], exclude=[])
        plan = self.call('plan', request=request)['plan']
        self.assertEqual(plan['selected']['logical@' + self.system]['machines']['include'], ['other_machine'])
        self.apply(plan)
        self.assertEqual(self.plan(profiles=['logical'])['selected'], {})

    def test_customs_survive_deselection_and_explicit_reselection(self):
        self.apply(self.plan(entries=['prefs'], metadata={'prefs': {'tools': ['local-tool']}}))
        self.apply(self.plan(entries=[]))
        state = json.loads((self.home / '.local/state/dot/adoption.json').read_text())
        self.assertEqual(state['customs'], {'prefs': {'tools': ['local-tool']}})
        self.assertEqual(self.plan()['selected'], {})
        plan = self.plan(entries=['prefs'])
        self.assertEqual(plan['selected']['prefs']['tools'], ['local-tool'])
        self.apply(plan)
        self.assertEqual(list(self.plan()['selected']), ['prefs'])

    def test_track_retargets_existing_manifest_id_without_adopting(self):
        (self.home / 'prefs.conf').write_text('local preference\n')
        selection = {k: v for k, v in self.entry.items() if k not in ('source', 'hooks')}
        selection.update(path='prefs.conf', machines=dict(include=['other_machine'], exclude=[]))
        state_before = (self.home / '.local/state/dot/adoption.json').read_bytes()
        plan = self.call('plan', request=dict(version=1, mode='track', profile='base', selections=[selection]))['plan']
        self.apply(plan)
        recorded = json.loads((self.root / 'manifest.json').read_text())
        self.assertEqual(list(recorded['entries']), ['prefs'])
        self.assertEqual(recorded['entries']['prefs']['machines'], dict(include=['other_machine'], exclude=[]))
        self.assertEqual((self.home / 'prefs.conf').read_text(), 'local preference\n')
        self.assertEqual((self.home / '.local/state/dot/adoption.json').read_bytes(), state_before)

    def test_pre_assignment_engine_rejects_metadata_instead_of_downgrading(self):
        repo = ENGINE.parents[1]
        revision = 'ebf77f1'
        available = subprocess.run(['git', '-C', str(repo), 'cat-file', '-e', revision], capture_output=True)
        if available.returncode:
            self.skipTest('historical pre-assignment engine unavailable in shallow checkout')
        old = self.base / 'old-engine'
        old.mkdir()
        for path in list(ENGINE.glob('*.py')) + [ENGINE / 'engine.json']:
            result = subprocess.run(['git', '-C', str(repo), 'show', revision + ':tools/dot/' + path.name], capture_output=True)
            if result.returncode == 0:
                (old / path.name).write_bytes(result.stdout)
        self.entry['machines'] = dict(include=['other_machine'], exclude=[])
        self.persist()
        before = (self.root / 'manifest.json').read_bytes()
        self.call('plan', ok=False, engine=old, request=dict(version=1, mode='adopt', profile='base', entries=['prefs']))
        self.assertFalse((self.home / 'prefs.conf').exists())
        self.assertEqual((self.root / 'manifest.json').read_bytes(), before)

    def test_changed_local_overrides_invalidate_exact_approval(self):
        self.apply(self.plan(entries=['prefs'], metadata={'prefs': {'tools': ['local-tool']}}))
        plan = self.plan()
        path = self.home / '.local/state/dot/adoption.json'
        state = json.loads(path.read_text())
        state['customs']['prefs']['tools'] = ['changed-tool']
        path.write_text(json.dumps(state))
        before = (self.home / 'prefs.conf').read_bytes()
        error = self.call('apply', ok=False, plan=plan, approve=plan['id'])
        self.assertIn('stale/changed plan', error)
        self.assertEqual((self.home / 'prefs.conf').read_bytes(), before)

    def test_track_retargets_bounded_git_without_importing_unapproved_keys(self):
        fixtures = ENGINE.parents[1] / 'tests/fixtures'
        (self.root / 'home/.chezmoitemplates').mkdir()
        for name in ('modify_dot_gitconfig.tmpl', '.chezmoitemplates/gitconfig.tmpl'):
            (self.root / 'home' / name).write_bytes((fixtures / name).read_bytes())
        entry = dict(self.entry, destination='.gitconfig', source='home/modify_dot_gitconfig.tmpl',
                     portability='managed-templated', impact='Unsafe', integrations=['git-config'],
                     git_keys=['user.name', 'user.email'], git_source='home/.chezmoitemplates/gitconfig.tmpl')
        self.catalog['entries']['git'] = entry
        self.persist()
        live = '[user]\nname = Local Name\nemail = keep@example.invalid\n[credential]\npassword = private-value\n'
        (self.home / '.gitconfig').write_text(live)
        selection = dict(path='.gitconfig', git_keys=['user.name'], machines=dict(include=['other_machine'], exclude=[]))
        plan = self.call('plan', request=dict(version=1, mode='track', profile='git', selections=[selection]))['plan']
        self.call('apply', plan=plan, approve=plan['id'], ack_unsafe=True)
        recorded = json.loads((self.root / 'manifest.json').read_text())['entries']['git']
        self.assertEqual(recorded['machines'], dict(include=['other_machine'], exclude=[]))
        self.assertEqual(recorded['git_keys'], ['user.name', 'user.email'])
        self.assertEqual((self.home / '.gitconfig').read_text(), live)
        self.assertNotIn('private-value', json.dumps(plan))


if __name__ == '__main__':
    unittest.main()
