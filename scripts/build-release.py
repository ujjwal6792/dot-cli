#!/usr/bin/env python3
"""Build review-only public archives; this script never publishes a release."""
import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile

from importlib.util import module_from_spec, spec_from_file_location

ROOT = Path(__file__).resolve().parents[1]
PLATFORMS = ('darwin-arm64', 'linux-amd64', 'linux-arm64')
spec = spec_from_file_location('public_export', ROOT / 'scripts/export-public.py')
exporter = module_from_spec(spec)
spec.loader.exec_module(exporter)


def build(version, output, binaries):
    if not re.fullmatch(r'[0-9]+\.[0-9]+\.[0-9]+(?:-[A-Za-z0-9.-]+)?', version):
        raise ValueError('version must be a filename-safe semantic version')
    if output.exists():
        raise ValueError('output already exists; no candidate was overwritten')
    # Prebuilt fixture/review inputs are data: never execute them while packaging.
    if binaries:
        for platform in PLATFORMS:
            path = binaries / ('dot-browser-' + platform)
            if path.is_symlink() or not path.is_file() or not path.stat().st_size:
                raise ValueError('missing or symlinked prebuilt browser: ' + platform)
    module = ROOT / 'tools/dot/browser'
    if not binaries and not (module / 'go.mod').is_file():
        raise ValueError('browser module from #24 is required to build; use --binaries only for reviewed prebuilt inputs')
    output.mkdir(parents=True)
    checksums = []
    browser_checksums = []
    with tempfile.TemporaryDirectory(prefix='dot-release-', dir=output) as temporary:
        temporary = Path(temporary)
        for platform in PLATFORMS:
            stage = temporary / platform / 'dot-cli'
            exporter.export(stage)
            target = stage / 'bin/dot-browser'
            target.parent.mkdir()
            if binaries:
                shutil.copyfile(binaries / ('dot-browser-' + platform), target)
            else:
                goos, goarch = platform.split('-')
                environment = {**os.environ, 'CGO_ENABLED': '0', 'GOOS': goos,
                               'GOARCH': goarch, 'GOWORK': 'off'}
                subprocess.run(['go', 'build', '-mod=readonly', '-trimpath',
                                '-buildvcs=false', '-o', str(target),
                                './cmd/dot-browser'], cwd=module, env=environment, check=True)
            target.chmod(0o755)
            browser_name = f'dot-browser-{version}-{platform}'
            shutil.copy2(target, output / browser_name)
            browser_checksums.append(hashlib.sha256(target.read_bytes()).hexdigest() + '  ' + browser_name)
            (stage / 'release.json').write_text(json.dumps({
                'format': 2, 'version': version, 'platform': platform, 'engine_api': 1,
                'prebuilt_input': bool(binaries)}, sort_keys=True) + '\n')
            name = f'dot-cli-{version}-{platform}.tar.gz'
            archive = output / name
            def public_metadata(member):
                member.uid = member.gid = 0
                member.uname = member.gname = ''
                member.mtime = 0
                member.pax_headers = {}
                return member
            # Set both tar metadata (above) and gzip's header timestamp so a
            # repeated build from the same source produces byte-identical assets.
            with archive.open('wb') as raw:
                with gzip.GzipFile(fileobj=raw, mode='wb', filename='', mtime=0) as compressed:
                    with tarfile.open(fileobj=compressed, mode='w') as stream:
                        stream.add(stage, arcname='dot-cli', filter=public_metadata)
            checksums.append(hashlib.sha256(archive.read_bytes()).hexdigest() + '  ' + name)
    (output / 'SHA256SUMS').write_text('\n'.join(checksums) + '\n')
    (output / 'BROWSER_SHA256SUMS').write_text('\n'.join(browser_checksums) + '\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--version', required=True)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--binaries', type=Path)
    args = parser.parse_args()
    try:
        build(args.version, args.output.resolve(), args.binaries)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, f'release build: {error}\n')
