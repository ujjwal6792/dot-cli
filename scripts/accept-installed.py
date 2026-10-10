#!/usr/bin/env python3
"""Run unchanged public-outcome fixtures with a reviewed extracted release.

Fixtures supply isolated HOME, synthetic private data/local remotes and fake
managers. Only setup paths change; assertions and browser interactions do not.
Verify the archive with check-release.py before extracting and invoking this.
This is macOS fixture evidence, not native Linux/manager/provenance acceptance.
"""
import argparse
import hashlib
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def prepare(name, release):
    engine = release / 'tools/dot'
    text = (ROOT / 'tests' / name).read_text()
    text = text.replace('REPO="$(cd "$BATS_TEST_DIRNAME/.." && pwd)"', f'REPO="{ROOT}"')
    if name.endswith('-test.py'):
        text = text.replace('REPO = Path(__file__).resolve().parents[1]', f'REPO = Path({str(ROOT)!r})')
        if name == 'tracking-cli-test.py':
            text = text.replace("shutil.copytree(REPO / 'tools/dot', self.engine,",
                                f"shutil.copytree(Path({str(engine)!r}), self.engine,")
        elif name == 'browser-api-test.py':
            text = text.replace("REPO / 'tools' / 'dot' / name", f"Path({str(engine)!r}) / name")
            for member in ('scope.py', 'engine.json', 'dot'):
                text = text.replace(f"REPO / 'tools' / 'dot' / '{member}'", f"Path({str(engine / member)!r})")
        elif name == 'packages-test.py':
            text = text.replace("Path(__file__).resolve().parents[1] / 'tools/dot/packages.py'",
                                f"Path({str(engine / 'packages.py')!r})")
    elif name == 'intent.bats':
        pass  # public intent.bats is already synthetic; no injection needed
    elif name == 'installed-migration.bats':
        text = text.replace('cp "$REPO/tools/dot/"*.py "$REPO/tools/dot/dot" "$REPO/tools/dot/engine.json" "$CANDIDATE/tools/dot/"',
                            f'cp "{engine}/"*.py "{engine}/dot" "{engine}/engine.json" "$CANDIDATE/tools/dot/"')
    elif name == 'bootstrap-helper.bash':
        text = text.replace('BOOT="$REPO/bootstrap.sh"', f'BOOT="{release}/bootstrap.sh"')
        text = text.replace('cp "$REPO/tools/dot/"*.py "$REPO/tools/dot/engine.json" "$REPO/tools/dot/dot" "$BATS_TEST_TMPDIR/engine-seed/tools/dot/"',
                            f'cp "{engine}/"*.py "{engine}/engine.json" "{engine}/dot" "$BATS_TEST_TMPDIR/engine-seed/tools/dot/"')
        text = text.replace('cp "$REPO/tools/dot/engine.json" "$BATS_TEST_TMPDIR/engine-seed/tools/dot/engine.json"',
                             f'cp "{engine}/engine.json" "$BATS_TEST_TMPDIR/engine-seed/tools/dot/engine.json"')
        text = text.replace('cp "$REPO/tools/dot/ui.sh"', f'cp "{engine}/ui.sh"')
    elif name.startswith('browser-') or name in ('tracking-ui.bats', 'split-bootstrap.bats'):
        text = text.replace('"$REPO/tools/dot/"', f'"{engine}/"')
        for file in ('engine.json', 'dot'):
            text = text.replace(f'"$REPO/tools/dot/{file}"', f'"{engine}/{file}"')
    return text


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--release', required=True, type=Path)
    parser.add_argument('--temporary-directory', required=True, type=Path)
    parser.add_argument('--previous-browser', type=Path, help='reviewed older prebuilt fixture for actual artifact replacement')
    args = parser.parse_args()
    release = args.release.resolve(strict=True)
    for path in ('tools/dot/dot', 'bootstrap.sh', 'bin/dot-browser'):
        if not (release / path).is_file():
            parser.error('missing reviewed release member: ' + path)
    if args.previous_browser and hashlib.sha256(args.previous_browser.read_bytes()).digest() == hashlib.sha256((release / 'bin/dot-browser').read_bytes()).digest():
        parser.error('previous browser fixture must differ from the reviewed current artifact')
    names = ('intent.bats', 'browser-ui.bats', 'tracking-ui.bats',
             'browser-approval.bats', 'browser-safety.bats', 'init.bats',
              'bootstrap-helper.bash', 'installed-migration.bats', 'split-bootstrap.bats',
              'installed-browser-update.bats',
             'tracking-cli-test.py', 'browser-api-test.py', 'packages-test.py')
    with tempfile.TemporaryDirectory(prefix='installed-acceptance-', dir=args.temporary_directory) as directory:
        root = Path(directory)
        for name in names:
            (root / name).write_text(prepare(name, release))
        import shutil
        shutil.copytree(ROOT / 'tests' / 'fixtures', root / 'fixtures')
        no_go = root / 'no-go'
        no_go.mkdir()
        trace = root / 'target-go-called'
        (no_go / 'go').write_text('#!/bin/sh\necho called >> "$DOT_ACCEPTANCE_GO_TRACE"\necho unexpected-target-Go >&2\nexit 97\n')
        (no_go / 'go').chmod(0o755)
        env = {**os.environ, 'DOT_BROWSER_TEST_BINARY': str(release / 'bin/dot-browser'),
               'DOT_UPDATE_ENGINE_FIXTURE': str(release / 'tools/dot'),
               'DOT_ACCEPTANCE_GO_TRACE': str(trace), 'TMPDIR': str(args.temporary_directory.resolve()),
                'PATH': str(no_go) + ':' + os.environ['PATH']}
        if args.previous_browser:
            env['DOT_BROWSER_BASELINE_TEST_BINARY'] = str(args.previous_browser.resolve(strict=True))
        def run(command):
            subprocess.run(command, env=env, check=True, timeout=1200)
            if trace.exists():
                raise ValueError('target Go was invoked, even if its failure was swallowed')
        for suite in ('intent.bats', 'browser-ui.bats', 'tracking-ui.bats',
                      'browser-approval.bats', 'browser-safety.bats', 'init.bats',
                       'installed-migration.bats'):
            run(['bats', str(root / suite)])
        for suite in ('split-bootstrap.bats', 'installed-browser-update.bats'):
            run(['bats', str(root / suite)])
        run(['bats', str(ROOT / 'tests/independent-updates.bats')])
        for suite in ('tracking-cli-test.py', 'browser-api-test.py', 'packages-test.py'):
            run(['python3', str(root / suite)])


if __name__ == '__main__':
    main()
