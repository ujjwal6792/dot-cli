#!/usr/bin/env python3
"""Package a native Rust dot binary with the existing verified chezmoi pin."""
import argparse
import hashlib
import io
import json
import pathlib
import subprocess
import tarfile
import urllib.request

VERSION = '2.73.1'
LICENSE = '6ce2375d89522ac7a1417320c8406886bc0ecb984baf41e3c27b8a07a81a30b2'
PINS = {
    'darwin-arm64': ('19a8d9778d90476cc2f0463be69e9d82e2b91ba7b2361dbfc5348f243b1ba731', '48bb0988507562f6ee1dca129182d681cf5648ef55b80a2201049a23ed615980'),
    'linux-amd64': ('a3f958acb7d0e5d53fa43c2eec9bc8ae3b4fba8bcbfdce29a8881ca3ffc20a53', 'fbf312e45c5c1f629e1d994c201cdd666198bd556af38512f4432ae6d3952521'),
    'linux-arm64': ('e60275b27f505ae2086bfb87b144eacf7377a89803120f6bd933d1852a93b0c6', 'c8ca4eddd5777156be9aa64de63f24046866228b31ae28eb8b8509043a899ce3'),
}
def sha(data):
    return hashlib.sha256(data).hexdigest()

def build(binary, platform, version, out, dependency=None):
    archive_hash, binary_hash = PINS[platform]
    asset = f'chezmoi_{VERSION}_{platform.replace("-", "_")}.tar.gz'
    raw = pathlib.Path(dependency).read_bytes() if dependency else urllib.request.urlopen(
        f'https://github.com/twpayne/chezmoi/releases/download/v{VERSION}/{asset}', timeout=90).read()
    if sha(raw) != archive_hash:
        raise ValueError('Pinned upstream archive checksum mismatch')
    with tarfile.open(fileobj=io.BytesIO(raw)) as tar:
        content = {}
        for name, expected in [('chezmoi', binary_hash), ('LICENSE', LICENSE)]:
            member = tar.getmember(name)
            if not member.isfile():
                raise ValueError(f'Not a regular file: {name}')
            content[name] = tar.extractfile(member).read()
            if sha(content[name]) != expected:
                raise ValueError(f'Pinned dependency checksum mismatch: {name}')
    actual = subprocess.check_output([str(binary.resolve()), '--version'], text=True).strip()
    if actual != f'dot {version}':
        raise ValueError(f'Binary version {actual!r} does not match {version!r}')
    files = {'dot': (binary.read_bytes(), 0o755), 'libexec/chezmoi': (content['chezmoi'], 0o755),
             'licenses/chezmoi.txt': (content['LICENSE'], 0o644)}
    manifest = {'schema': 2, 'version': version, 'platform': platform, 'chezmoi': VERSION,
                'runtime': 'Rust + Ratatui; system Git',
                'files': {name: {'sha256': sha(data), 'mode': mode} for name, (data, mode) in files.items()}}
    files['release.json'] = (json.dumps(manifest, indent=2).encode() + b'\n', 0o644)
    out.mkdir(parents=True, exist_ok=True)
    destination = out / f'dot-{version}-{platform}.tar.gz'
    with tarfile.open(destination, 'w:gz') as tar:
        for name, (data, mode) in sorted(files.items()):
            header = tarfile.TarInfo(name)
            header.size, header.mode, header.mtime = len(data), mode, 0
            tar.addfile(header, io.BytesIO(data))
    destination.with_suffix(destination.suffix + '.sha256').write_text(f'{sha(destination.read_bytes())}  {destination.name}\n')
    return destination

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=pathlib.Path, default=pathlib.Path('target/release/dot'))
    parser.add_argument('--platform', choices=PINS, required=True)
    parser.add_argument('--version', required=True)
    parser.add_argument('--out', type=pathlib.Path, default=pathlib.Path('dist'))
    parser.add_argument('--dependency', type=pathlib.Path)
    args = parser.parse_args()
    print(build(args.binary, args.platform, args.version, args.out, args.dependency))
