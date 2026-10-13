#!/usr/bin/env python3
"""Machine setup through the public browser bridge, isolated local state only."""
import json
import itertools
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

REPO = Path(__file__).resolve().parents[1]
ENGINE = REPO / 'tools/dot'


class MachineIdentityTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.base = Path(self.tmp.name).resolve()
        self.home = self.base / 'home'
        self.home.mkdir()
        self.source = self.base / 'source'
        (self.source / 'home').mkdir(parents=True)
        (self.source / 'dot-contract.json').write_text(json.dumps(dict(
            protocol=1, engine_api=dict(min=1, max=1), manifest_version=1,
            approval_version=2, manifest='manifest.json', home_source='home',
            required_adapters=['intent', 'scope', 'gitconfig', 'skills', 'layout'])))
        (self.source / 'manifest.json').write_text(json.dumps(dict(version=1, entries={}, profiles={})))
        self.env = dict(os.environ, HOME=str(self.home), USER='Ace.Test', LOGNAME='Ace.Test')
        for key in ('DOT_STATE_DIR', 'XDG_STATE_HOME'):
            self.env.pop(key, None)
        self.state = self.home / '.local/state/dot/adoption.json'
        self.platform_fixture = None

    def tearDown(self):
        self.tmp.cleanup()

    def bridge(self, action, ok=True, command='init-machine', **fields):
        request = dict(version=1, command=command, engine=str(ENGINE),
                       source=str(self.source), home=str(self.home), request=dict(action=action, **fields))
        if command == 'inventory':
            request.pop('request')
            request['mode'] = 'adopt'
        args = [sys.executable, '-B', str(ENGINE / 'browser_api.py')]
        if self.platform_fixture is not None:
            # Fixture the operating-system boundary, not machine policy internals.
            args = [sys.executable, '-B', '-c',
                    'import platform,sys,runpy; sys.platform="linux"; '
                    'platform.freedesktop_os_release=lambda: ' + repr(self.platform_fixture) + '; '
                    'sys.path.insert(0,' + repr(str(ENGINE)) + '); '
                    'runpy.run_path(' + repr(str(ENGINE / 'browser_api.py')) + ',run_name="__main__")']
        result = subprocess.run(args,
                                input=json.dumps(request) + '\n', env=self.env, text=True, capture_output=True, timeout=5)
        response = json.loads(result.stdout)
        self.assertEqual(response['ok'], ok, response)
        return response.get('result', response.get('error'))

    def test_suggestion_is_read_only_and_confirmation_persists_local_v3(self):
        suggestion = self.bridge('suggest')
        self.assertRegex(suggestion['machine']['id'], r'^[a-z0-9_]+_ace_test$')
        if sys.platform == 'darwin':
            self.assertEqual(suggestion['machine'], {'id': 'macos_ace_test', 'os': 'darwin'})
        self.assertFalse(self.state.exists())
        before = (self.source / 'manifest.json').read_bytes()
        self.bridge('save', id='my_workstation')
        recorded = json.loads(self.state.read_text())
        self.assertEqual(recorded, dict(version=3, profile='unapproved', entries={},
                                       machine=dict(id='my_workstation', os='darwin' if sys.platform == 'darwin' else 'linux')))
        self.assertEqual(self.state.stat().st_mode & 0o777, 0o600)
        self.assertEqual((self.source / 'manifest.json').read_bytes(), before)
        self.assertEqual(self.bridge('suggest')['machine']['id'], 'my_workstation')

    def test_adoption_owned_identity_resolves_v2_without_machine_sidecar(self):
        current = 'darwin' if sys.platform == 'darwin' else 'linux'
        path = self.source / 'dot-contract.json'
        descriptor = json.loads(path.read_text())
        descriptor.update(manifest_version=2, approval_version=3)
        path.write_text(json.dumps(descriptor))
        (self.source / 'manifest.json').write_text(json.dumps(dict(
            version=2, entries={}, profiles={}, tracking={},
            machines={'other_machine': dict(label='Other machine', os=current)})))
        self.bridge('save', id='this_machine')
        self.bridge('', command='inventory')
        self.assertFalse((self.state.parent / 'machine.json').exists())

    def test_os_release_id_then_id_like_and_normalized_fallback(self):
        for release, expected in [({'ID': 'CachyOS', 'ID_LIKE': 'arch linux'}, 'cachyos_ace_test'),
                                  ({'ID': '', 'ID_LIKE': 'Arch Linux'}, 'arch_linux_ace_test'),
                                  ({'ID': '---', 'ID_LIKE': 'Arch Linux'}, 'arch_linux_ace_test'),
                                  ({}, 'linux_ace_test')]:
            with self.subTest(release=release):
                self.platform_fixture = release
                self.assertEqual(self.bridge('suggest')['machine'], dict(id=expected, os='linux'))

    def test_second_machine_collision_gets_two_character_suffix_without_touching_first(self):
        first = self.bridge('suggest')['machine']
        self.bridge('save', id=first['id'])
        before = self.state.read_bytes()
        current = first['os']
        descriptor = self.source / 'dot-contract.json'
        data = json.loads(descriptor.read_text())
        data.update(manifest_version=2, approval_version=3)
        descriptor.write_text(json.dumps(data))
        (self.source / 'manifest.json').write_text(json.dumps(dict(
            version=2, entries={}, profiles={}, tracking={},
            machines={first['id']: dict(label='First machine', os=current)})))
        source_before = (self.source / 'manifest.json').read_bytes()
        self.home = self.base / 'second'
        self.home.mkdir()
        self.env['HOME'] = str(self.home)
        suggested = self.bridge('suggest')['machine']['id']
        self.assertRegex(suggested, '^' + first['id'] + r'_[a-z0-9]{2}$')
        error = self.bridge('save', id=first['id'], ok=False)
        self.assertIn('already exists locally', error)
        self.bridge('save', id=suggested)
        self.assertEqual(self.state.read_bytes(), before)
        self.assertEqual((self.source / 'manifest.json').read_bytes(), source_before)
        self.assertEqual(json.loads((self.home / '.local/state/dot/adoption.json').read_text())['machine']['id'], suggested)

    def test_invalid_or_changed_identity_never_overwrites_saved_state(self):
        for key in ('', 'Upper', 'with-hyphen', '../escape', 'with space', 'é', 'a' * 129):
            self.bridge('save', id=key, ok=False)
            self.assertFalse(self.state.exists())
        self.bridge('save', id='first_machine')
        before = self.state.read_bytes()
        self.bridge('save', id='second_machine', ok=False)
        self.assertEqual(self.state.read_bytes(), before)

    def test_leading_underscore_is_a_valid_persistent_identity(self):
        self.bridge('save', id='_my_machine')
        self.assertEqual(self.bridge('suggest')['machine']['id'], '_my_machine')
        self.bridge('', command='inventory')

    def test_existing_v3_identity_is_preserved_without_compatibility_rewrite(self):
        self.state.parent.mkdir(parents=True)
        self.state.write_text(json.dumps(dict(version=3, profile='default', entries={},
            machine=dict(id='Legacy-Machine', os='darwin' if sys.platform == 'darwin' else 'linux'))))
        before = self.state.read_bytes()
        self.assertEqual(self.bridge('suggest')['machine']['id'], 'Legacy-Machine')
        self.bridge('save', id='Legacy-Machine')
        self.assertEqual(self.state.read_bytes(), before)

    def test_state_override_is_used_and_symlink_targets_are_protected(self):
        override = self.base / 'override'
        self.env['DOT_STATE_DIR'] = str(override)
        self.bridge('save', id='override_machine')
        self.assertFalse(self.state.exists())
        self.assertEqual(json.loads((override / 'adoption.json').read_text())['machine']['id'], 'override_machine')
        before = (override / 'adoption.json').read_bytes()
        link = self.base / 'linked'
        link.symlink_to(override, target_is_directory=True)
        self.env['DOT_STATE_DIR'] = str(link)
        self.bridge('suggest', ok=False)
        self.bridge('save', id='bad_machine', ok=False)
        self.assertEqual((override / 'adoption.json').read_bytes(), before)
        self.env['DOT_STATE_DIR'] = str(self.base / 'safe')
        safe = Path(self.env['DOT_STATE_DIR'])
        safe.mkdir()
        (safe / 'adoption.json').symlink_to(override / 'adoption.json')
        self.bridge('save', id='bad_machine', ok=False)
        self.assertEqual((override / 'adoption.json').read_bytes(), before)

    def test_identity_setup_is_offline(self):
        bin_dir = self.base / 'bin'
        bin_dir.mkdir()
        log = self.base / 'network-calls'
        for tool in ('git', 'curl', 'ssh', 'gh', 'wget'):
            script = bin_dir / tool
            script.write_text('#!/bin/sh\necho called >> ' + str(log) + '\nexit 99\n')
            script.chmod(0o700)
        self.env['PATH'] = str(bin_dir)
        self.bridge('suggest')
        self.bridge('save', id='offline_machine')
        self.bridge('', command='inventory')
        self.assertFalse(log.exists())

    def test_adopt_keeps_v3_machine_identity_with_v1_manifest(self):
        entry = dict(destination='prefs.conf', source='home/prefs.conf', os=['darwin', 'linux'],
                     portability='managed-shared', impact='Safe', removals=[], hooks=[], tools=[])
        (self.source / 'home/prefs.conf').write_text('repository preference\n')
        (self.source / 'manifest.json').write_text(json.dumps(dict(version=1, entries={'prefs': entry},
                                                                 profiles={'default': ['prefs']})))
        for args in [('init', '-q'), ('add', '.'), ('-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid', 'commit', '-qm', 'fixture')]:
            subprocess.run(['git', '-C', str(self.source), *args], env=self.env, capture_output=True, check=True)
        self.bridge('save', id='local_machine')
        request = self.base / 'request.json'
        request.write_text(json.dumps(dict(version=1, mode='adopt', profile='default', entries=['prefs'])))
        plan = self.base / 'plan.json'
        tracker = self.base / 'tracker.toml'
        tracker.write_text('sourceDir=' + json.dumps(str(self.source / 'home')) +
                           '\n[data]\ndotEngine=' + json.dumps(str(ENGINE)) + '\n')
        env = dict(self.env, DOT_ENGINE=str(ENGINE), DOT_SOURCE=str(self.source), DOT_CONFIG=str(tracker))
        for args in [('intent', 'plan', str(request), '--out', str(plan)),
                     ('intent', 'apply', str(plan), '--approve', None)]:
            args = list(args)
            if args[-1] is None:
                args[-1] = json.loads(plan.read_text())['id']
            result = subprocess.run([str(ENGINE / 'dot'), *args], env=env, text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        recorded = json.loads(self.state.read_text())
        self.assertEqual(recorded['version'], 3)
        self.assertEqual(recorded['machine']['id'], 'local_machine')
        self.assertEqual(recorded['entries'], {'prefs': entry})
        self.assertEqual((self.home / 'prefs.conf').read_text(), 'repository preference\n')

    def test_legacy_group_scope_and_invalid_v3_are_not_downgraded(self):
        self.state.parent.mkdir(parents=True)
        for data in (dict(version=1, groups={'shell': ['prefs.conf']}),
                     dict(version=3, profile='unapproved', entries={}, machine={'id': 'broken'})):
            self.state.write_text(json.dumps(data))
            before = self.state.read_bytes()
            self.bridge('save', id='new_machine', ok=False)
            self.assertEqual(self.state.read_bytes(), before)

    def test_exhausted_collision_suffixes_stop_with_edit_guidance(self):
        self.platform_fixture = {}
        descriptor = self.source / 'dot-contract.json'
        data = json.loads(descriptor.read_text())
        data.update(manifest_version=2, approval_version=3)
        descriptor.write_text(json.dumps(data))
        names = ['linux_ace_test'] + ['linux_ace_test_' + ''.join(pair)
                 for pair in itertools.product('abcdefghijklmnopqrstuvwxyz0123456789', repeat=2)]
        (self.source / 'manifest.json').write_text(json.dumps(dict(
            version=2, entries={}, profiles={}, tracking={},
            machines={name: dict(label=name, os='linux') for name in names})))
        error = self.bridge('suggest', ok=False)
        self.assertIn('choose a unique id', error)
        self.assertFalse(self.state.exists())

    def test_identity_save_does_not_interfere_with_pending_scope_recovery(self):
        journal = self.state.parent / 'recovery/fixture/transaction.json'
        journal.parent.mkdir(parents=True)
        journal.write_text(json.dumps(dict(status='pending', live_files='unchanged')))
        error = self.bridge('save', id='new_machine', ok=False)
        self.assertIn('pending tracking transaction', error)
        self.assertFalse(self.state.exists())


if __name__ == '__main__':
    unittest.main()
