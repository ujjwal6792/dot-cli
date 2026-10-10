#!/usr/bin/env python3
"""Verify candidate integrity and exact public archive boundary without extraction."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import tarfile

from importlib.util import module_from_spec, spec_from_file_location

spec = spec_from_file_location('release_build', Path(__file__).with_name('build-release.py'))
builder = module_from_spec(spec)
spec.loader.exec_module(builder)
ALLOWED = set(builder.exporter.PUBLIC) | {'tools/dot/' + name for name in builder.exporter.ENGINE}
ALLOWED |= set(builder.exporter.BUILD) | set(builder.exporter.WORKFLOWS)
ALLOWED |= {'bin/dot-browser', 'release.json'}
GO = {'tools/dot/browser/' + name for name in builder.exporter.GO} | {'tools/dot/browser_api.py'}
REQUIRED = set(ALLOWED)
ALLOWED |= GO
TOKEN = re.compile(rb'ghp_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY|AKIA[0-9A-Z]{16}')


def check(root):
    browsers = {}
    for line in (root / 'BROWSER_SHA256SUMS').read_text().splitlines():
        match = re.fullmatch(r'([a-f0-9]{64})  (dot-browser-[0-9]+\.[0-9]+\.[0-9]+(?:-[A-Za-z0-9.-]+)?-(?:darwin-arm64|linux-amd64|linux-arm64))', line)
        if not match or match[2] in browsers:
            raise ValueError('invalid browser checksum entry')
        digest, name = match.groups()
        binary = root / name
        if binary.is_symlink() or not binary.stat().st_mode & 0o111 or hashlib.sha256(binary.read_bytes()).hexdigest() != digest:
            raise ValueError('browser checksum mismatch: ' + name)
        browsers[name] = digest
    if len(browsers) != len(builder.PLATFORMS):
        raise ValueError('expected one browser for each supported platform')
    lines = (root / 'SHA256SUMS').read_text().splitlines()
    if len(lines) != len(builder.PLATFORMS):
        raise ValueError('expected one archive for each supported platform')
    seen = set()
    release_files = {'SHA256SUMS', 'BROWSER_SHA256SUMS', *browsers}
    for line in lines:
        match = re.fullmatch(r'([a-f0-9]{64})  (dot-cli-([0-9]+\.[0-9]+\.[0-9]+(?:-[A-Za-z0-9.-]+)?)-(darwin-arm64|linux-amd64|linux-arm64)\.tar\.gz)', line)
        if not match:
            raise ValueError('invalid checksum entry')
        digest, name, version, platform = match.groups()
        release_files.add(name)
        if platform in seen:
            raise ValueError('duplicate release platform')
        seen.add(platform)
        archive = root / name
        if archive.is_symlink() or hashlib.sha256(archive.read_bytes()).hexdigest() != digest:
            raise ValueError('checksum mismatch: ' + name)
        files, members = set(), set()
        with tarfile.open(archive) as stream:
            for member in stream:
                if member.name in members:
                    raise ValueError('duplicate archive member')
                members.add(member.name)
                if member.uid != 0 or member.gid != 0 or member.uname or member.gname:
                    raise ValueError('private build-machine ownership metadata')
                if member.isdir():
                    if member.name != 'dot-cli' and not any(('dot-cli/' + path).startswith(member.name + '/') for path in ALLOWED):
                        raise ValueError('unexpected payload directory: ' + member.name)
                    continue
                relative = member.name.removeprefix('dot-cli/')
                if not member.isfile() or member.name != 'dot-cli/' + relative or relative not in ALLOWED:
                    raise ValueError('unexpected payload: ' + member.name)
                if member.mode & 0o7000 or member.size > 128 * 1024 * 1024:
                    raise ValueError('unsafe mode or oversized member')
                content = stream.extractfile(member).read()
                if TOKEN.search(content):
                    raise ValueError('credential-shaped content in candidate')
                if relative == 'release.json':
                    data = json.loads(content)
                    if (not isinstance(data, dict) or
                            set(data) != {'format', 'version', 'platform', 'engine_api', 'prebuilt_input'} or
                            type(data['format']) is not int or
                            type(data['engine_api']) is not int or
                            type(data['prebuilt_input']) is not bool):
                        raise ValueError('unexpected release metadata')
                    if data.get('format') != 2 or data.get('version') != version or data.get('platform') != platform or data.get('engine_api') != 1:
                        raise ValueError('release identity mismatch')
                if relative == 'bin/dot-browser' and not member.mode & 0o111:
                    raise ValueError('browser is not executable')
                if relative == 'bin/dot-browser' and browsers.get(f'dot-browser-{version}-{platform}') != hashlib.sha256(content).hexdigest():
                    raise ValueError('archive/browser mismatch')
                files.add(relative)
        if not REQUIRED <= files or (files & GO and not GO <= files):
            raise ValueError('missing public payload files')
    if seen != set(builder.PLATFORMS):
        raise ValueError('unsupported platform set')
    if {path.name for path in root.iterdir()} != release_files:
        raise ValueError('unexpected release-directory payload')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    args = parser.parse_args()
    try:
        check(args.directory)
    except (ValueError, OSError, tarfile.TarError) as error:
        parser.exit(1, f'release check: {error}\n')
    print('Candidate checksums and public boundary verified; provenance/native smoke review still required.')
