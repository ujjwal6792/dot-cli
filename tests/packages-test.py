#!/usr/bin/env python3
"""Synthetic private lists and mocked managers; never query the host."""
import importlib.util
from pathlib import Path
import tempfile
import os
import unittest
from unittest.mock import patch


class ManagerFixture:
    def __init__(self):
        self.calls = []
        self.installed = 'fixture-cli\n'
        self.resolution = 'fixture-tmux 1.0\n'

    def __call__(self, argv, mutate=False):
        self.calls.append((argv, mutate))
        if '--version' in argv:
            return 'fixture manager 1.0'
        if '--print-architecture' in argv:
            return 'amd64\n'
        if '--dry-run' in argv or '-Sp' in argv or '-Si' in argv:
            return self.resolution
        if '-s' in argv:
            return 'Inst fixture-tmux (1.0 fixture)\nConf fixture-tmux (1.0 fixture)\n'
        if argv[0].endswith('dpkg-query'):
            return ''.join(name + '\tinstalled\n' for name in self.installed.splitlines())
        if '--cask' in argv:
            return ''
        return '' if mutate else self.installed

SPEC = importlib.util.spec_from_file_location('packages', Path(__file__).resolve().parents[1] / 'tools/dot/packages.py')
packages = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(packages)


class PackagesTest(unittest.TestCase):
    def test_apt_native_multiarch_names_are_installed_but_foreign_arch_is_not(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'packages').mkdir()
            (root / 'packages/apt.txt').write_text('fixture-cli\nfixture-tmux\n')
            runner = ManagerFixture()
            def query(argv, mutate=False):
                if argv[0].endswith('dpkg-query') and '-W' in argv:
                    return 'fixture-cli:arm64\tinstalled\nfixture-tmux:amd64\tinstalled\n'
                return runner(argv, mutate)
            engine = packages.Planner(root, platform='ubuntu', runner=query, uid=1000, which=lambda n: '/fixture/'+n)
            desired = {r['name']: r for r in engine.status()['rows'] if r['desired']}
            self.assertFalse(desired['fixture-tmux']['missing'])
            self.assertTrue(desired['fixture-cli']['missing'])

    def test_brew_package_names_cannot_be_local_ruby_or_json_programs(self):
        for name in ('fixture.rb', 'fixture.json', 'fixture-owner/fixture-tap/fixture.rb', 'fixture@1.rb'):
            self.assertTrue(packages.parse('brew', f'brew "{name}"\n')['diagnostics'], name)
        self.assertEqual(packages.parse('brew', 'brew "fixture-cli@1.2"\n')['names'], ['fixture-cli@1.2'])

    def test_qualified_formula_does_not_match_another_tap_or_core(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'packages').mkdir()
            (root / 'packages/Brewfile').write_text('brew "fixture-owner/fixture-tap/fixture-cli"\n')
            runner = ManagerFixture()
            engine = packages.Planner(root, platform='darwin', runner=runner, uid=1000, which=lambda n: '/fixture/'+n)
            desired = next(r for r in engine.status()['rows'] if r['desired'])
            self.assertTrue(desired['missing'])

    def test_nonregular_private_source_is_rejected_before_read(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'packages').mkdir()
            os.mkfifo(root / 'packages/apt.txt')
            engine = packages.Planner(root, platform='ubuntu', runner=lambda *a: self.fail('unexpected manager'))
            with self.assertRaisesRegex(ValueError, 'regular'):
                engine.load('apt')

    def test_changed_executable_or_manager_environment_invalidates_approval(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'packages').mkdir()
            (root / 'packages/Brewfile').write_text('brew "fixture-tmux"\n')
            binary = root / 'brew'
            binary.write_text('fixture executable revision 1\n')
            runner = ManagerFixture()
            engine = packages.Planner(root, platform='darwin', runner=runner, uid=1000, which=lambda n: str(binary))
            plan = engine.plan('browse')
            with patch.dict('os.environ', {'HOMEBREW_CASK_OPTS': '--appdir=/fixture/new-location'}):
                with self.assertRaisesRegex(ValueError, 'stale'):
                    engine.apply(plan, plan['id'], 'browse')
            binary.write_text('fixture executable revision 2\n')
            with self.assertRaisesRegex(ValueError, 'stale'):
                engine.apply(plan, plan['id'], 'browse')
            self.assertFalse(any(m for _, m in runner.calls))

    def test_cask_formula_identity_and_installed_deselection_never_uninstall(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'packages').mkdir()
            path = root / 'packages/Brewfile'
            path.write_text('brew "fixture-cli"\ncask "fixture-cli"\n')
            runner = ManagerFixture()
            engine = packages.Planner(root, platform='darwin', runner=runner, uid=1000, which=lambda n: '/fixture/'+n)
            rows = engine.status()['rows']
            self.assertEqual([(r['kind'], r['missing']) for r in rows], [('brew', False), ('cask', True)])
            plan = engine.plan('browse')
            self.assertEqual(plan['commands'], [['/fixture/brew', 'install', '--cask', '--', 'fixture-cli']])
            loaded = engine.load('brew')
            engine.save('brew', loaded['digest'], '# no desired packages\n')
            rows = engine.status()['rows']
            self.assertEqual([(r['desired'], r['installed'], r['missing']) for r in rows], [(False, True, False)])
            empty = engine.plan('init')
            engine.apply(empty, empty['id'], 'init')
            self.assertFalse(any(m for _, m in runner.calls))

    def test_symlinked_source_and_backups_fail_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            outside = root / 'outside'
            outside.write_text('fixture-cli\n')
            (root / 'packages').mkdir()
            path = root / 'packages/apt.txt'
            path.symlink_to(outside)
            engine = packages.Planner(root, platform='ubuntu', runner=lambda *a: self.fail('unexpected manager'))
            with self.assertRaisesRegex(ValueError, 'symlinked'):
                engine.load('apt')
            path.unlink()
            (root / 'packages/.backups').symlink_to(root)
            with self.assertRaisesRegex(ValueError, 'backup'):
                engine.save('apt', engine.load('apt')['digest'], 'fixture-tmux\n')
            self.assertFalse(path.exists())

    def test_sudo_absence_malformed_names_and_apt_noninstalled_states_are_safe(self):
        for manager in ('brew', 'pacman', 'yay', 'apt'):
            for line in ('--evil', 'fixture;touch sentinel', '$(touch sentinel)', 'fixture=9', 'fixture bad'):
                text = f'brew "{line}"\n' if manager == 'brew' else line + '\n'
                self.assertTrue(packages.parse(manager, text)['diagnostics'], (manager, line))
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'packages').mkdir()
            (root / 'packages/apt.txt').write_text('fixture-tmux\n')
            runner = ManagerFixture()
            def query(argv, mutate=False):
                if argv[0].endswith('dpkg-query') and '-W' in argv:
                    return 'fixture-cli\tinstalled\nfixture-tmux\tconfig-files\n'
                return runner(argv, mutate)
            engine = packages.Planner(root, platform='ubuntu', runner=query, uid=1000, which=lambda n: None if n=='sudo' else '/fixture/'+n)
            rows = engine.status()['rows']
            self.assertTrue(next(r for r in rows if r['name']=='fixture-tmux')['missing'])
            with self.assertRaisesRegex(ValueError, 'sudo is missing'):
                engine.plan('init')
            self.assertFalse(any(m for _, m in runner.calls))

    def test_ambiguous_arch_manager_sources_block_before_mutation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'packages').mkdir()
            for filename in ('pacman.txt', 'yay.txt'):
                (root / 'packages' / filename).write_text('fixture-tmux\n')
            runner = ManagerFixture()
            engine = packages.Planner(root, platform='arch', runner=runner, uid=1000, which=lambda n: '/fixture/'+n)
            with self.assertRaisesRegex(ValueError, 'conflicting'):
                engine.plan('browse')
            self.assertFalse(any(m for _, m in runner.calls))

    def test_supplied_private_import_is_bounded_backed_up_and_inert(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'supplied').mkdir()
            (root / 'supplied/fixture-list').write_text('fixture-cli\nfixture-tmux\n')
            engine = packages.Planner(root, platform='ubuntu', runner=lambda *a: self.fail('import ran manager'))
            result = engine.import_file('apt', engine.load('apt')['digest'], 'supplied/fixture-list')
            self.assertEqual(result['names'], ['fixture-cli', 'fixture-tmux'])
            with self.assertRaisesRegex(ValueError, 'private'):
                engine.import_file('apt', result['digest'], '../outside')

    def test_missing_manager_still_allows_private_editing_but_not_planning(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'packages').mkdir()
            (root / 'packages/apt.txt').write_text('fixture-tmux\n')
            engine = packages.Planner(root, platform='ubuntu', runner=lambda *a: self.fail('unexpected runner'), which=lambda n: None)
            overview = engine.overview()
            self.assertIn('apt', overview['sources'])
            self.assertEqual(overview['rows'][0]['status'], 'unavailable')
            self.assertIn('dpkg-query is missing', overview['notice'])
            with self.assertRaisesRegex(ValueError, 'missing'):
                engine.plan('browse')

    def test_byte_preserving_backups_and_unknown_inert_syntax(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'packages').mkdir()
            path = root / 'packages/Brewfile'
            raw = b'# fixture\r\nbrew "fixture-cli"\r\nsystem("touch sentinel")\r\n'
            path.write_bytes(raw)
            runner = ManagerFixture()
            engine = packages.Planner(root, platform='darwin', runner=runner, uid=1000, which=lambda n: '/fixture/'+n)
            loaded = engine.load('brew')
            self.assertEqual(loaded['text'].encode(), raw)
            with self.assertRaisesRegex(ValueError, 'unsupported source syntax'):
                engine.plan('browse')
            engine.save('brew', loaded['digest'], loaded['text'].replace('fixture-cli', 'fixture-tmux'))
            self.assertEqual(next((path.parent / '.backups').iterdir()).read_bytes(), raw)
            self.assertFalse((root / 'sentinel').exists())
            self.assertFalse(any(m for _, m in runner.calls))

    def test_readonly_resolution_and_source_freshness_fail_closed(self):
        for platform, manager, text in [('darwin', 'brew', 'brew "fixture-tmux"\n'), ('arch', 'pacman', 'fixture-tmux\n')]:
            with self.subTest(manager=manager), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                (root / 'packages').mkdir()
                path = root / 'packages' / packages.SOURCES[manager]
                path.write_text(text)
                runner = ManagerFixture()
                engine = packages.Planner(root, platform=platform, runner=runner, uid=1000, which=lambda n: '/fixture/' + n)
                plan = engine.plan('browse')
                self.assertIn('fixture-tmux 1.0', engine.review(plan))
                runner.resolution = 'fixture-tmux 2.0\n'
                with self.assertRaisesRegex(ValueError, 'stale'):
                    engine.apply(plan, plan['id'], 'browse')
                runner.resolution = 'fixture-tmux 1.0\n'
                path.write_text(text + '# edited\n')
                with self.assertRaisesRegex(ValueError, 'stale'):
                    engine.apply(plan, plan['id'], 'browse')
                self.assertFalse(any(m for _, m in runner.calls))

    def test_aur_is_separately_reviewed_unprivileged_and_no_fallback(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'packages').mkdir()
            (root / 'packages/yay.txt').write_text('fixture-aur\n')
            runner = ManagerFixture()
            factory = lambda **kw: packages.Planner(root, platform='arch', runner=runner,
                which=lambda name: '/fixture/bin/' + name, **kw)
            engine = factory(uid=1000)
            plan = engine.plan('browse')
            self.assertIn('AUR', engine.review(plan))
            with self.assertRaisesRegex(ValueError, 'acknowledgment'):
                engine.apply(plan, plan['id'], 'browse')
            with self.assertRaisesRegex(ValueError, 'unprivileged'):
                factory(uid=0).plan('browse')
            self.assertFalse(any(m for _, m in runner.calls))
            engine.apply(plan, plan['id'], 'browse', ack_aur=True)
            self.assertEqual([a for a, m in runner.calls if m],
                [['/fixture/bin/yay', '-S', '--aur', '--needed', '--', 'fixture-aur']])
            missing = packages.Planner(root, platform='arch', runner=runner, uid=1000,
                which=lambda name: None if name == 'yay' else '/fixture/bin/' + name)
            with self.assertRaisesRegex(ValueError, 'yay is missing'):
                missing.plan('browse')

    def test_status_and_exact_consent_freshness_for_each_os_and_both_entrypoints(self):
        for platform, manager, text in [('darwin', 'brew', 'brew "fixture-cli"\nbrew "fixture-tmux"\n'),
                                         ('arch', 'pacman', 'fixture-cli\nfixture-tmux\n'),
                                         ('ubuntu', 'apt', 'fixture-cli\nfixture-tmux\n')]:
            for entrypoint in ('browse', 'init'):
                with self.subTest(platform=platform, entrypoint=entrypoint), tempfile.TemporaryDirectory() as directory:
                    root = Path(directory)
                    (root / 'packages').mkdir()
                    (root / 'packages' / packages.SOURCES[manager]).write_text(text)
                    runner = ManagerFixture()
                    engine = packages.Planner(root, platform=platform, runner=runner,
                                              which=lambda name: '/fixture/bin/' + name, uid=1000)
                    state = engine.status()
                    self.assertEqual([(r['name'], r['desired'], r['installed'], r['missing']) for r in state['rows']],
                                     [('fixture-cli', True, True, False), ('fixture-tmux', True, False, True)])
                    plan = engine.plan(entrypoint)
                    self.assertEqual(plan['missing'][0]['name'], 'fixture-tmux')
                    self.assertFalse(any(mutate for _, mutate in runner.calls))
                    with self.assertRaisesRegex(ValueError, 'approval'):
                        engine.apply(plan, '', entrypoint)
                    changed = dict(plan, commands=[])
                    with self.assertRaisesRegex(ValueError, 'changed'):
                        engine.apply(changed, plan['id'], entrypoint)
                    runner.installed += 'fixture-tmux\n'
                    with self.assertRaisesRegex(ValueError, 'stale'):
                        engine.apply(plan, plan['id'], entrypoint)
                    runner.installed = 'fixture-cli\n'
                    engine.apply(plan, plan['id'], entrypoint)
                    mutations = [argv for argv, mutate in runner.calls if mutate]
                    self.assertEqual(mutations, plan['commands'])
                    self.assertTrue(all('fixture-tmux' in argv and 'fixture-cli' not in argv for argv in mutations))

    def test_private_save_backs_up_bytes_rejects_stale_and_never_runs_manager(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'packages').mkdir()
            path = root / 'packages/Brewfile'
            path.write_text('# keep me\nbrew "fixture-cli"\n')
            engine = packages.Planner(root, platform='darwin', runner=lambda *args: self.fail('save ran a manager'))
            loaded = engine.load('brew')
            engine.save('brew', loaded['digest'], '# keep me\nbrew "fixture-tmux"\n')
            self.assertEqual(path.read_text(), '# keep me\nbrew "fixture-tmux"\n')
            self.assertEqual(list((root / 'packages/.backups').iterdir())[0].read_text(), loaded['text'])
            with self.assertRaisesRegex(ValueError, 'stale'):
                engine.save('brew', loaded['digest'], '')

    def test_brewfile_is_data_not_executable_and_unknown_lines_survive(self):
        text = '# private fixture\nbrew "fixture-cli"\ncask "fixture-app"\nsystem("touch sentinel")\n'
        result = packages.parse('brew', text)
        self.assertEqual(result['names'], ['fixture-cli', 'fixture-app'])
        self.assertEqual(result['diagnostics'], ['line 4: unsupported syntax; install blocked'])
        self.assertEqual(result['text'], text)


if __name__ == '__main__':
    unittest.main()
