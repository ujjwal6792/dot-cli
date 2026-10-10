#!/usr/bin/env python3
"""Create a review candidate from an explicit allowlist, never a Git mirror."""
import argparse
from pathlib import Path
import shutil

ROOT = Path(__file__).resolve().parents[1]
ENGINE = ('dot', 'contract.py', 'engine.json', 'gitconfig.py', 'intent.py', 'init.py',
          'layout.py', 'migration.py', 'picker.py', 'preview.py', 'scope.py', 'skills.py',
           'state.py', 'ui.sh', 'tracking.py', 'file_metadata.py', 'packages.py', 'updates.py',
           'install_state.py')
PUBLIC = ('README.md', 'bootstrap.sh', 'install.sh', 'scripts/create-private-repo.sh',
           'skills/setup-dots/SKILL.md', 'skills/dot/SKILL.md', 'docs/private-contract.md',
           'docs/install-record-contract.md')
BUILD = ('scripts/export-public.py', 'scripts/build-release.py', 'scripts/check-release.py')
WORKFLOWS = ('.github/workflows/release.yml',)
GO = ('go.mod', 'go.sum', 'client.go', 'json.go', 'model.go', 'packages.go', 'cmd/dot-browser/main.go')


def export(output):
    if output.exists():
        raise ValueError('output already exists; review/remove candidate explicitly')
    # Legacy local staging path only; the distribution identity is dots-cli.
    public = ROOT / 'public/dots-installer' if (ROOT / 'public/dots-installer').is_dir() else ROOT
    inputs = [(public / name, name) for name in PUBLIC + WORKFLOWS]
    inputs += [(ROOT / name, name) for name in BUILD]
    inputs += [(ROOT / 'tools/dot' / name, 'tools/dot/' + name) for name in ENGINE]
    if (ROOT / 'tools/dot/browser/go.mod').exists():
        inputs += [(ROOT / 'tools/dot/browser' / name, 'tools/dot/browser/' + name) for name in GO]
        inputs.append((ROOT / 'tools/dot/browser_api.py', 'tools/dot/browser_api.py'))
    for source, _ in inputs:
        if source.resolve() != source or not source.is_file():
            raise ValueError('missing or symlinked public input: ' + str(source))
    output.mkdir(parents=True)
    for source, name in inputs:
        target = output / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, target)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    try:
        export(args.output)
    except (ValueError, OSError) as error:
        parser.exit(1, f'public export: {error}\n')
