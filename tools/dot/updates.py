#!/usr/bin/env python3
"""Independent, clean fast-forward updates; candidate code is never executed."""
import argparse
import ast
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import stat
import urllib.parse
import urllib.request

from contract import check


def git(root, *args):
    command = ['git', '-c', 'core.hooksPath=/dev/null', '-c', 'core.fsmonitor=false', '-C', str(root)]
    filters = subprocess.run(command + ['config', '--name-only', '--get-regexp',
                                        r'^filter\..*\.(smudge|clean|process|required)$'],
                             text=True, stdout=subprocess.PIPE, check=False)
    if filters.returncode not in (0, 1):
        raise ValueError('cannot inspect checkout filters safely')
    overrides = []
    for key in filters.stdout.splitlines():
        overrides += ['-c', key + ('=false' if key.endswith('.required') else '=')]
    return subprocess.check_output(command + overrides + list(args), text=True).strip()


def materialize(repo, revision, destination):
    # Read blobs rather than extracting an archive: no traversal or symlink writes.
    records = subprocess.check_output(['git', '-C', str(repo), 'ls-tree', '-rz',
                                      revision]).split(b'\0')
    for record in filter(None, records):
        header, raw = record.split(b'\t', 1)
        mode, kind, oid = header.decode().split()
        relative = Path(os.fsdecode(raw))
        if (relative.is_absolute() or '..' in relative.parts or
                any(part.casefold() == '.git' for part in relative.parts) or
                mode not in ('100644', '100755') or kind != 'blob'):
            raise ValueError('unsupported candidate payload (symlink/submodule/path)')
        path = destination / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(subprocess.check_output(['git', '-C', str(repo), 'cat-file', 'blob', oid]))
        path.chmod(0o755 if mode == '100755' else 0o644)


def plan(engine, source, approval):
    import intent
    negotiated = check(engine, source)
    intent.MANIFEST_PATH = negotiated['manifest']
    intent.MANIFEST_RELATIVE = Path(negotiated['manifest']).relative_to(source).as_posix()
    return intent.plan(source, Path(os.environ['HOME']),
                       {'version': 1, 'mode': 'update', 'profile': approval['profile']})


def comparable(value):
    return {key: item for key, item in value.items() if key not in ('root', 'id')}


def managed_browser(root, source):
    """Only an install-record-bound, exact regular artifact is exempt from dirt."""
    path = root / 'bin/dot-browser'
    if not path.exists() and not path.is_symlink():
        return None
    record_path = Path(os.environ['HOME']).resolve() / '.local/state/dot/install.json'
    if record_path.resolve() != record_path or path.resolve() != path:
        raise ValueError('managed browser/installation record must not traverse symlinks')
    record = json.loads(record_path.read_text())
    browser = record.get('browser')
    if (record.get('version') != 2 or Path(record.get('engine', '')).resolve() != root or
            Path(record.get('directory', '')).resolve() != source or not isinstance(browser, dict) or
            browser.get('path') != 'bin/dot-browser' or browser.get('mode') != 0o755 or
            not stat.S_ISREG(path.stat().st_mode) or stat.S_IMODE(path.stat().st_mode) != 0o755 or
            hashlib.sha256(path.read_bytes()).hexdigest() != browser.get('sha256')):
        raise ValueError('managed browser does not match installation record; preserve/review it before updating')
    return record_path.read_bytes(), stat.S_IMODE(record_path.stat().st_mode), path.read_bytes()


def clean(root, browser):
    records = git(root, 'status', '--porcelain=v1', '--untracked-files=all', '--ignored').splitlines()
    # Never exempt tracked changes, entire directories, or other bin files.
    return all(not item or browser is not None and item in
               ('?? bin/dot-browser', '!! bin/dot-browser') for item in records)


def browser_artifact(url, digest, destination):
    if not url or not digest or len(digest) != 64 or any(c not in '0123456789abcdefABCDEF' for c in digest):
        raise ValueError('browser update requires a reviewed URL/path and SHA-256')
    parsed = urllib.parse.urlsplit(url)
    if parsed.scheme == 'https' and parsed.netloc and not (parsed.username or parsed.password or parsed.query or parsed.fragment):
        with urllib.request.urlopen(url, timeout=60) as response:
            if urllib.parse.urlsplit(response.url).scheme != 'https':
                raise ValueError('browser redirect must remain HTTPS')
            destination.write_bytes(response.read())
    elif Path(url).is_absolute() and not parsed.scheme:
        destination.write_bytes(Path(url).read_bytes())
    else:
        raise ValueError('browser URL must be credential-free HTTPS or an absolute fixture path')
    if hashlib.sha256(destination.read_bytes()).hexdigest() != digest.lower():
        raise ValueError('browser artifact checksum mismatch; no update applied')
    destination.chmod(0o755)


def atomic_write(path, data, mode):
    fd, name = tempfile.mkstemp(dir=path.parent)
    try:
        with os.fdopen(fd, 'wb') as stream:
            stream.write(data)
        os.chmod(name, mode)
        os.replace(name, path)
    finally:
        if os.path.exists(name):
            os.unlink(name)


def update(command, yes, browser_url=None, browser_sha256=None):
    engine = Path(os.environ['DOT_ENGINE']).resolve()
    source = Path(os.environ['DOT_SOURCE']).resolve()
    check(engine, source)
    if command == 'pull':
        import scope
        import intent
        approval_path = scope.profile_path(os.environ['HOME'])
        approval_state = intent.file_state(approval_path)
        approval = scope.load(os.environ['HOME'])
        if approval is None or approval['version'] not in (2, 3):
            raise ValueError('pull requires an approved exact scope; no unscoped fallback')
        current = plan(engine, source, approval)
        for action in current['actions']:
            expected = (None if action['remove'] else
                        {'sha256': action.get('git_result') or hashlib.sha256(action['text'].encode()).hexdigest(),
                         'mode': action['mode']})
            if action['before'] != expected:
                raise ValueError('machine drift exists in approved scope; resolve it before pull')
    root = Path(git(engine if command == 'self-update' else source, 'rev-parse', '--show-toplevel'))
    if command == 'pull' and root != source:
        raise ValueError('DOT_SOURCE must be the private checkout root')
    if command == 'self-update' and (root == source or root in source.parents or source in root.parents):
        raise ValueError('public checkout must be independent of private source')
    browser = managed_browser(root, source) if command == 'self-update' else None
    if (browser_url or browser_sha256) and (command != 'self-update' or browser is None):
        raise ValueError('browser update requires a verified installed public browser')
    if not clean(root, browser):
        raise ValueError('checkout is dirty; preserve/commit local work before updating')
    head = git(root, 'rev-parse', 'HEAD')
    branch = git(root, 'symbolic-ref', '--short', 'HEAD')
    remote = git(root, 'config', 'branch.' + branch + '.remote')
    ref = git(root, 'config', 'branch.' + branch + '.merge')
    if remote == '.' or not ref.startswith('refs/heads/'):
        raise ValueError('configure a remote branch upstream before updating')
    url = git(root, 'remote', 'get-url', remote)
    with tempfile.TemporaryDirectory(prefix='dot-update-') as temporary:
        temporary = Path(temporary).resolve()
        repo = temporary / 'remote.git'
        git(temporary, 'init', '--bare', '-q', str(repo))
        git(repo, 'fetch', '-q', '--no-tags', str(root), head)
        git(repo, 'fetch', '-q', '--no-tags', url, ref)
        candidate = git(repo, 'rev-parse', 'FETCH_HEAD')
        try:
            git(repo, 'merge-base', '--is-ancestor', head, candidate)
        except subprocess.CalledProcessError:
            raise ValueError('local commits are ahead or diverged; refusing reset') from None
        tree = temporary / 'candidate'
        tree.mkdir()
        materialize(repo, candidate, tree)
        if command == 'pull':
            git(tree, 'init', '-q')
            git(tree, 'fetch', '-q', '--no-tags', str(repo), candidate)
            git(tree, 'read-tree', candidate)
            # Establish candidate HEAD without checkout filters or private hooks.
            git(tree, 'update-ref', 'HEAD', candidate)
        candidate_engine = tree / engine.relative_to(root) if command == 'self-update' else engine
        candidate_source = source if command == 'self-update' else tree
        check(candidate_engine, candidate_source)
        if command == 'self-update':
            if (tree / 'bin/dot-browser').exists():
                raise ValueError('candidate must not track the locally managed browser artifact')
            artifact = temporary / 'browser'
            if browser_url or browser_sha256:
                browser_artifact(browser_url, browser_sha256, artifact)
            elif browser is not None and git(repo, 'diff', '--name-only', head, candidate, '--', 'tools/dot/browser'):
                raise ValueError('browser source changed; supply --browser-url and --browser-sha256 for the reviewed release artifact')
            forbidden = [tree / name for name in ('home', 'packages', 'dot-contract.json')]
            forbidden += [candidate_engine / name for name in ('manifest.json', 'groups.json')]
            if any(path.exists() for path in forbidden):
                raise ValueError('candidate public payload contains private configuration; review the distribution')
            for name in ('state.py', 'preview.py', 'picker.py', 'ui.sh', 'updates.py'):
                if not (candidate_engine / name).is_file():
                    raise ValueError('candidate public engine payload is incomplete: ' + name)
            # Parse only; neither candidate modules nor candidate CLI are executed.
            for module in candidate_engine.glob('*.py'):
                try:
                    ast.parse(module.read_text(), filename=str(module))
                except (SyntaxError, UnicodeError):
                    raise ValueError('candidate Python payload is invalid: ' + module.name) from None
            subprocess.run(['bash', '-n', str(candidate_engine / 'dot')], check=True)
            subprocess.run(['bash', '-n', str(candidate_engine / 'ui.sh')], check=True)
            print('Public tooling only: ' + head + ' → ' + candidate, flush=True)
            print(git(repo, 'diff', '--stat', head, candidate), flush=True)
            if artifact.exists():
                print('Reviewed browser artifact (bin/dot-browser): ' +
                      hashlib.sha256(browser[2]).hexdigest() + ' → ' +
                      hashlib.sha256(artifact.read_bytes()).hexdigest(), flush=True)
        else:
            # No raw private diff: bounded Git previews must not expose credentials.
            import intent
            reviewed = plan(engine, tree, approval)
            intent.preview(reviewed)
        if not yes:
            print('Preview only; rerun with --yes to explicitly apply the reviewed update.')
            return
        if git(root, 'rev-parse', 'HEAD') != head or not clean(root, browser) or (browser is not None and managed_browser(root, source) != browser):
            raise ValueError('checkout changed during review; retry')
        check(candidate_engine, candidate_source)
        if command == 'pull' and (intent.file_state(approval_path) != approval_state or
                                  plan(engine, tree, approval) != reviewed):
            raise ValueError('target/approval changed during review; retry')
        git(root, 'fetch', '-q', '--no-tags', str(repo), candidate)
        if git(root, 'rev-parse', 'HEAD') != head or not clean(root, browser) or (browser is not None and managed_browser(root, source) != browser):
            raise ValueError('checkout changed before fast-forward; retry')
        if command == 'pull' and (intent.file_state(approval_path) != approval_state or
                                  plan(engine, tree, approval) != reviewed):
            raise ValueError('target/approval changed before fast-forward; retry')
        git(root, 'merge', '--ff-only', candidate)
        if command == 'self-update' and artifact.exists():
            record_path = Path(os.environ['HOME']).resolve() / '.local/state/dot/install.json'
            record = json.loads(browser[0])
            record['browser']['sha256'] = hashlib.sha256(artifact.read_bytes()).hexdigest()
            try:
                atomic_write(root / 'bin/dot-browser', artifact.read_bytes(), 0o755)
                atomic_write(record_path, (json.dumps(record, indent=2) + '\n').encode(), browser[1])
            except OSError:
                installed = root / 'bin/dot-browser'
                if installed.read_bytes() != browser[2] or stat.S_IMODE(installed.stat().st_mode) != 0o755:
                    atomic_write(installed, browser[2], 0o755)
                if record_path.read_bytes() != browser[0] or stat.S_IMODE(record_path.stat().st_mode) != browser[1]:
                    atomic_write(record_path, browser[0], browser[1])
                # Restore only the verified transaction's tracked tree, never
                # discard concurrent tracked work or unrelated local artifacts.
                if not git(root, 'diff', '--name-only', candidate) and clean(root, browser):
                    git(root, 'read-tree', '--reset', '-u', head)
                    git(root, 'update-ref', 'refs/heads/' + branch, head, candidate)
                raise ValueError('browser/record promotion failed; previous artifact restored; inspect checkout before retry') from None
        if command == 'pull':
            actual = plan(engine, source, approval)
            if comparable(actual) != comparable(reviewed):
                raise ValueError('reviewed pull plan changed; configuration was not applied')
            intent.apply(source, Path(os.environ['HOME']), actual, actual['id'], False)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=('self-update', 'pull'))
    parser.add_argument('--yes', action='store_true')
    parser.add_argument('--browser-url')
    parser.add_argument('--browser-sha256')
    args = parser.parse_args()
    try:
        update(args.command, args.yes, args.browser_url, args.browser_sha256)
    except subprocess.CalledProcessError:
        # Do not print subprocess argument lists containing authenticated URLs.
        print('dot update: Git or candidate validation failed; verify upstream/access; no reset was performed', file=sys.stderr)
        return 1
    except (OSError, ValueError, KeyError) as error:
        print('dot update: ' + str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
