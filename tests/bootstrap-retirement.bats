#!/usr/bin/env bats
load bootstrap-helper

# Retirement now belongs to #27, not bootstrap. Exercise the former ownership
# hazards as data and assert the ENTIRE target tree (bytes, modes, links) remains
# unchanged even with --replace-chezmoi. Preserve the old scenarios separately
# for the eventual explicit migration entrypoint, never silently skip them.
preserved_legacy() {
  python3 - "$1" <<'PY'
import os,pathlib
home=pathlib.Path(os.environ['HOME']); case=__import__('sys').argv[1]
config=home/'.config/chezmoi'; config.mkdir(parents=True)
(config/'chezmoi.toml').write_text('sourceDir="/legacy/source"\n[data]\nname="keep"\nemail="keep@example.org"\n')
source=home/'.local/share/chezmoi'; source.mkdir(parents=True)
(source/'private').write_bytes(b'private source bytes\x00')
cache=home/'.cache/chezmoi'; cache.mkdir(parents=True)
(cache/'item').write_text('unrelated cache')
(config/'chezmoistate.boltdb').write_bytes(b'opaque existing tracker state')
state=home/'.local/state/dot'; state.mkdir(parents=True)
(state/'adoption.json').write_text('{"version":2,"groups":{"eza":["shared"]}}\n')
if case in ('tokens','comments','hooks','protected','custom','mixed'):
    text={'tokens':'"cacheDir" = """/legacy/cache"""\n',
          'comments':'# preserve comment\r\ncacheDir = "/legacy/cache" # inline\r\n',
          'hooks':'[hooks.read-source-state.pre]\ncommand="touch"\nargs=["'+str(home/'hook-ran')+'"]\n',
          'protected':'cacheDir="'+str(home/'.ssh')+'"\n',
          'custom':'cacheDir="'+str(home/'project')+'"\n',
          'mixed':'[data.extra]\nvalue="keep # value"\n'}[case]
    with (config/'chezmoi.toml').open('a',newline='') as stream: stream.write(text)
if case in ('partial','occupied','reintroduced','record','selection'):
    (state/'retirement.json').write_text('{"stage":"incomplete","pending":["source"]}\n')
    (state/'chezmoi-cache').mkdir(); (state/'chezmoi-cache/item').write_text('do not overwrite')
if case in ('symlink','backup-link','xdg-link'):
    (home/'destination').write_text('sentinel outside old source')
    (source/'link').symlink_to(home/'destination')
if case in ('home-git','commondir','linked','git-hooks','git-includes'):
    git=home/'.git'; git.mkdir()
    (git/'HEAD').write_text('ref: refs/heads/main\n')
    (git/'commondir').write_text('../unrelated\n')
    (git/'hooks').mkdir(); hook=git/'hooks/post-checkout'
    hook.write_text('#!/bin/sh\ntouch "'+str(home/'hook-ran')+'"\n'); hook.chmod(0o755)
if case in ('bare','cache-repo','project','source-repo'):
    (cache/'HEAD').write_text('ref: refs/heads/main\n'); (cache/'objects').mkdir()
if case in ('identity','credential','signing'):
    (home/'.gitconfig').write_text('[user]\nname=Keep\nemail=keep@example.org\nsigningkey=keep-key\n[credential]\nhelper=keep-helper\n')
if case in ('automation','startup','dynamic'):
    (home/'.zshenv').write_text('ZDOTDIR=$(touch "'+str(home/'startup-ran')+'")\n')
    service=home/'.config/systemd/user'; service.mkdir(parents=True)
    (service/'dotfiles-sync.service').write_text('keep inactive service')
PY
  legacy_snapshot > "$BATS_TEST_TMPDIR/before"
  : > "$TRACE"
  run install_dot --yes --replace-chezmoi --stage migration
  [ "$status" -eq 0 ]
  [[ "$output" == *'Migration is owned by #27'* ]]
  legacy_snapshot > "$BATS_TEST_TMPDIR/after"
  cmp "$BATS_TEST_TMPDIR/before" "$BATS_TEST_TMPDIR/after"
  [ ! -s "$TRACE" ]
  [ ! -e "$HOME/hook-ran" ]
  [ ! -e "$HOME/startup-ran" ]
}

legacy_snapshot() {
  python3 - <<'PY'
import hashlib,json,os,pathlib,stat
home=pathlib.Path(os.environ['HOME']); entries=[]
for root,dirs,files in os.walk(home,followlinks=False):
    for name in sorted(dirs+files):
        path=pathlib.Path(root)/name; info=path.lstat()
        data=os.readlink(path) if path.is_symlink() else hashlib.sha256(path.read_bytes()).hexdigest() if path.is_file() else ''
        entries.append((str(path.relative_to(home)),stat.S_IMODE(info.st_mode),data))
print(json.dumps(sorted(entries)))
PY
}

@test "migration detection preserves unsupported path tokens" { preserved_legacy tokens; }
@test "migration detection preserves private source checkout" { preserved_legacy source-repo; }
@test "migration detection preserves CRLF comments and spacing" { preserved_legacy comments; }
@test "migration detection never executes config hooks" { preserved_legacy hooks; }
@test "migration detection preserves partial journals" { preserved_legacy partial; }
@test "migration detection preserves occupied destinations" { preserved_legacy occupied; }
@test "replacement option never authorizes bootstrap retirement" { preserved_legacy record; }
@test "migration detection preserves reintroduced state" { preserved_legacy reintroduced; }
@test "migration detection preserves approval selection bytes" { preserved_legacy selection; }
@test "migration detection never claims incidental source markers" { preserved_legacy project; }
@test "migration detection preserves custom manager paths" { preserved_legacy custom; }
@test "migration detection preserves cache repositories" { preserved_legacy cache-repo; }
@test "migration detection preserves bare repository metadata" { preserved_legacy bare; }
@test "migration detection preserves commondir metadata" { preserved_legacy commondir; }
@test "migration detection never retires HOME Git metadata" { preserved_legacy home-git; }
@test "migration detection creates no backup or recovery journal" { preserved_legacy partial; }
@test "migration detection preserves XDG link destinations" { preserved_legacy xdg-link; }
@test "migration detection preserves linked Git metadata" { preserved_legacy linked; }
@test "migration detection never resumes partial retirement" { preserved_legacy partial; }
@test "migration detection preserves mixed settings" { preserved_legacy mixed; }
@test "migration detection preserves configured machine identity" { preserved_legacy identity; }
@test "migration detection preserves symlink ownership hazards" { preserved_legacy symlink; }
@test "migration detection never follows source links for backups" { preserved_legacy backup-link; }
@test "migration detection preserves dynamic startup overrides" { preserved_legacy dynamic; }
@test "migration detection never performs Git cleanup" { preserved_legacy git-includes; }
@test "migration detection never activates or moves legacy automation" { preserved_legacy automation; }
@test "migration detection never runs Git hooks" { preserved_legacy git-hooks; }
@test "migration detection preserves credential and signing settings" { preserved_legacy credential; }
@test "migration detection preserves protected live-path settings" { preserved_legacy protected; }
