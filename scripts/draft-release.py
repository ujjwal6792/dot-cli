#!/usr/bin/env python3
"""Verify, safely retarget when proven necessary, and publish a staged draft."""
import argparse
from datetime import datetime
import json
import re
import subprocess
import sys
from pathlib import Path
from importlib.machinery import SourceFileLoader

ASSETS = SourceFileLoader(
    'verify_release_assets', str(Path(__file__).with_name('verify-release-assets.py'))
).load_module()
SHA = re.compile(r'[0-9a-f]{40}\Z')
TAG = re.compile(r'v0\.1\.0-rc\.[1-9][0-9]*\Z')


def gh(*args):
    return subprocess.run(['gh', 'api', *args], check=True, text=True, capture_output=True).stdout


def decode_pages(raw):
    decoder = json.JSONDecoder()
    cursor = 0
    while cursor < len(raw):
        while cursor < len(raw) and raw[cursor].isspace():
            cursor += 1
        if cursor == len(raw):
            break
        page, cursor = decoder.raw_decode(raw, cursor)
        yield page


def list_releases(repository, tag):
    found = []
    for page in decode_pages(gh('--paginate', f'repos/{repository}/releases')):
        if not isinstance(page, list):
            raise ValueError('GitHub release listing was not an array')
        found.extend(item for item in page if item.get('tag_name') == tag)
    if len(found) > 1:
        raise ValueError('multiple releases have the requested tag; refusing to choose')
    return found[0] if found else None


def get_release(repository, release_id):
    release = json.loads(gh(f'repos/{repository}/releases/{release_id}'))
    if release.get('id') != release_id:
        raise ValueError('release ID lookup returned a different release')
    return release


def verify_identity(release, tag, target=None):
    if (release.get('tag_name') != tag or release.get('draft') is not True
            or release.get('prerelease') is not True
            or (target is not None and release.get('target_commitish') != target)):
        raise ValueError('release identity/state does not match the approved draft')
    if not isinstance(release.get('id'), int):
        raise ValueError('GitHub release has no valid release ID')


def verify_assets(release, directory):
    ASSETS.verify(directory, [
        {'name': item.get('name'), 'size': item.get('size'), 'digest': item.get('digest')}
        for item in release.get('assets', [])
    ])


def tag_ref_is_absent(repository, tag):
    refs = json.loads(gh(f'repos/{repository}/git/matching-refs/tags/{tag}'))
    if not isinstance(refs, list):
        raise ValueError('GitHub tag-reference response was not an array')
    return all(item.get('ref') != f'refs/tags/{tag}' for item in refs)


def require_tag_absent(repository, tag):
    if not tag_ref_is_absent(repository, tag):
        raise ValueError('tag ref must remain absent until publication')


def is_ancestor(source_sha, target_sha):
    if not SHA.fullmatch(source_sha) or not SHA.fullmatch(target_sha):
        return False
    return subprocess.run(
        ['git', 'merge-base', '--is-ancestor', source_sha, target_sha],
        check=False,
    ).returncode == 0


def _time(value):
    return datetime.fromisoformat(value.replace('Z', '+00:00'))


def verified_origin_run(repository, release, source_sha):
    """Prove this draft came from an earlier successful build/smoke workflow run."""
    created = release.get('created_at')
    if not isinstance(created, str):
        raise ValueError('draft has no creation timestamp to bind to its source run')
    candidates = []
    endpoint = f'repos/{repository}/actions/workflows/release.yml/runs?event=workflow_dispatch&per_page=100'
    for page in decode_pages(gh('--paginate', endpoint)):
        runs = page.get('workflow_runs') if isinstance(page, dict) else None
        if not isinstance(runs, list):
            raise ValueError('GitHub workflow run listing was malformed')
        for run in runs:
            if (run.get('head_sha') != source_sha or run.get('event') != 'workflow_dispatch'
                    or run.get('head_branch') != 'main' or run.get('status') != 'completed'
                    or run.get('conclusion') != 'failure'):
                continue
            if not (_time(run['created_at']) <= _time(created) <= _time(run['updated_at'])):
                continue
            jobs_response = json.loads(gh('--paginate', f"repos/{repository}/actions/runs/{run['id']}/jobs"))
            jobs = jobs_response.get('jobs') if isinstance(jobs_response, dict) else None
            if not isinstance(jobs, list):
                raise ValueError('GitHub workflow job listing was malformed')
            results = {job.get('name'): job.get('conclusion') for job in jobs}
            if (results.get('verify') == 'success' and results.get('build') == 'success'
                    and results.get('installed-smoke') == 'success' and results.get('publish') == 'failure'):
                candidates.append(run)
    if len(candidates) != 1:
        raise ValueError('draft source is not proven by exactly one matching prior release workflow run')
    return candidates[0]


def validate_stale_draft(repository, release, tag, target, directory, ancestor_check=None):
    source = release.get('target_commitish')
    ancestor_check = ancestor_check or is_ancestor
    if not isinstance(source, str) or not SHA.fullmatch(source) or not ancestor_check(source, target):
        raise ValueError('draft source is not a valid ancestor of current main')
    verified_origin_run(repository, release, source)
    verify_assets(release, directory)
    if not tag_ref_is_absent(repository, tag):
        raise ValueError('tag ref already exists; refusing to retarget or publish this draft')


def prepare(repository, tag, target, directory, inspect_only=False, allow_missing=False):
    if not TAG.fullmatch(tag) or not SHA.fullmatch(target):
        raise ValueError('tag or current target SHA is malformed')
    selected = list_releases(repository, tag)
    if selected is None:
        if allow_missing:
            return None
        raise ValueError('no staged draft release found; create/upload must happen before verification')
    release_id = selected.get('id')
    if not isinstance(release_id, int):
        raise ValueError('release listing has no valid release ID')
    release = get_release(repository, release_id)
    verify_identity(release, tag)
    if release.get('target_commitish') != target:
        validate_stale_draft(repository, release, tag, target, directory)
        if not inspect_only:
            gh('--method', 'PATCH', f'repos/{repository}/releases/{release_id}',
               '-F', f'target_commitish={target}', '-F', 'draft=true', '-F', 'prerelease=true')
            release = get_release(repository, release_id)
            verify_identity(release, tag, target)
            verify_assets(release, directory)
            require_tag_absent(repository, tag)
    else:
        verify_identity(release, tag, target)
        verify_assets(release, directory)
        require_tag_absent(repository, tag)
    return release_id


def publish(repository, release_id, tag, target, directory):
    # Re-read both mutable release state and the tag immediately before publishing.
    # GitHub documents no conditional-write/CAS contract, so external edits can
    # still race after this check; post-publish validation detects, not reverses, that case.
    require_tag_absent(repository, tag)
    release = get_release(repository, release_id)
    verify_identity(release, tag, target)
    verify_assets(release, directory)
    gh('--method', 'PATCH', f'repos/{repository}/releases/{release_id}',
       '-F', 'draft=false', '-F', 'prerelease=true')
    release = get_release(repository, release_id)
    if (release.get('tag_name') != tag or release.get('target_commitish') != target
            or release.get('draft') is not False or release.get('prerelease') is not True):
        raise ValueError('published release identity/state did not match the verified draft')
    verify_assets(release, directory)
    tag_commit = json.loads(gh(f'repos/{repository}/commits/{tag}')).get('sha')
    if tag_commit != target:
        raise ValueError('published Git tag does not resolve to the verified current-main commit')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repository', required=True)
    parser.add_argument('--tag', required=True)
    parser.add_argument('--target', required=True)
    parser.add_argument('--directory', type=Path, required=True)
    parser.add_argument('--inspect-only', action='store_true')
    parser.add_argument('--allow-missing', action='store_true')
    args = parser.parse_args()
    try:
        release_id = prepare(args.repository, args.tag, args.target, args.directory,
                             inspect_only=args.inspect_only, allow_missing=args.allow_missing)
        if release_id is None:
            print('')
            return 0
        if args.inspect_only:
            print(release_id)
            return 0
        publish(args.repository, release_id, args.tag, args.target, args.directory)
    except (ValueError, OSError, subprocess.CalledProcessError, KeyError, json.JSONDecodeError) as error:
        print(f'draft release publish refused: {error}', file=sys.stderr)
        return 1
    print(f'Published verified release ID {release_id} and confirmed tag target {args.target}')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
