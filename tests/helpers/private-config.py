#!/usr/bin/env python3
"""Create a deterministic, public-safe synthetic private checkout for tests.

The fixture models the public contract boundary only: no owner inventory,
package pins, real identities, tokens, private history, or host paths.
"""
import argparse
import json
from pathlib import Path
import shutil


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--source', required=True, type=Path)
    parser.add_argument('--fixtures', required=True, type=Path)
    parser.add_argument('--empty', action='store_true',
                        help='create only the synthetic contract/catalog boundary')
    args = parser.parse_args()
    root = args.source
    (root / 'home').mkdir(parents=True, exist_ok=True)
    (root / 'tools/dot').mkdir(parents=True, exist_ok=True)
    (root / 'home/.chezmoiignore').write_text('tools/**\npackages/**\nscripts/**\nnotes\n')
    (root / 'home/.chezmoiremove').write_text('')
    shutil.copyfile(args.fixtures / 'modify_dot_gitconfig.tmpl', root / 'home/modify_dot_gitconfig.tmpl')
    (root / 'home/.chezmoitemplates').mkdir(exist_ok=True)
    shutil.copyfile(args.fixtures / '.chezmoitemplates/gitconfig.tmpl',
                    root / 'home/.chezmoitemplates/gitconfig.tmpl')

    entries = {}

    def add(key, destination, relative, content, *, os_names=('darwin', 'linux'),
            portability='managed-shared', impact='Unsafe', warnings=(), notices=(), tools=()):
        target = root / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(content)
        entries[key] = {
            'destination': destination, 'source': relative, 'os': list(os_names),
            'portability': portability, 'impact': impact, 'removals': [], 'hooks': [],
            'tools': list(tools), 'warnings': list(warnings), 'notices': list(notices),
        }

    if not args.empty:
      add('.bashrc', '.bashrc', 'home/dot_bashrc', "alias gt='lazygit'\n",
        notices=('shell-replacement',), tools=('bash', 'lazygit'))
      add('.bash_profile', '.bash_profile', 'home/dot_bash_profile', "alias gt='lazygit'\n",
        notices=('shell-replacement',), tools=('bash',))
      entries['.bash_profile']['requires'] = ['.bashrc']
      add('.zshrc', '.zshrc', 'home/dot_zshrc.tmpl', "alias gt='lazygit'\n",
        portability='managed-templated', notices=('shell-replacement',), tools=('zsh', 'lazygit'))

      entries['.gitconfig'] = {
        'destination': '.gitconfig', 'source': 'home/modify_dot_gitconfig.tmpl',
        'git_source': 'home/.chezmoitemplates/gitconfig.tmpl',
        'git_keys': ['user.name', 'user.email'], 'os': ['darwin', 'linux'],
        'portability': 'managed-templated', 'impact': 'Unsafe', 'removals': [], 'hooks': [],
        'tools': ['git'], 'integrations': ['git-config'], 'notices': ['config-only'],
    }
      add('.config/hypr/monitors.conf', '.config/hypr/monitors.conf',
        'home/dot_config/hypr/monitors.conf', '# synthetic desktop settings\n',
        os_names=('linux',), portability='managed-os-only',
        warnings=('desktop-arch-compatibility',), notices=('config-only',), tools=('Hyprland',))
      add('.config/systemd/user/xremap.service', '.config/systemd/user/xremap.service',
        'home/dot_config/systemd/user/xremap.service',
        '[Unit]\nDescription=Synthetic service fixture\n', os_names=('linux',),
        portability='managed-os-only', warnings=('desktop-arch-compatibility',),
        notices=('config-only',), tools=('systemctl', 'xremap', 'wl-paste'))
      add('.config/systemd/user/cliphist.service', '.config/systemd/user/cliphist.service',
        'home/dot_config/systemd/user/cliphist.service',
        '[Unit]\nDescription=Synthetic service fixture\n', os_names=('linux',),
        portability='managed-os-only', notices=('config-only',), tools=('systemctl', 'cliphist'))
      add('Library/LaunchAgents/com.dotfiles.drift.plist',
        'Library/LaunchAgents/com.dotfiles.drift.plist',
        'home/Library/LaunchAgents/com.dotfiles.drift.plist.tmpl',
        '<plist><dict><key>Label</key><string>fixture</string></dict></plist>\n',
        os_names=('darwin',), portability='managed-templated', notices=('config-only',),
        tools=('launchctl', 'dot-drift'))
      add('.tmux.conf', '.tmux.conf', 'home/dot_tmux.conf', '# synthetic tmux settings\n',
        notices=('config-only',), tools=('tmux',))
      add('.config/eza/theme.yml', '.config/eza/theme.yml', 'home/dot_config/eza/theme.yml',
        'colour: fixture\n', impact='Safe', tools=('eza',))

    (root / 'dot-contract.json').write_text(json.dumps({
        'protocol': 1, 'engine_api': {'min': 1, 'max': 1}, 'manifest_version': 1,
        'approval_version': 2, 'manifest': 'tools/dot/manifest.json', 'home_source': 'home',
        'required_adapters': ['intent', 'scope', 'gitconfig', 'skills', 'layout'],
    }) + '\n')
    (root / 'tools/dot/manifest.json').write_text(json.dumps({
        'version': 1, 'entries': entries, 'profiles': {'fixture': sorted(entries)},
    }, indent=2, sort_keys=True) + '\n')
    (root / 'tools/dot/groups.json').write_text(json.dumps({
        'eza': {'risk': 'Safe', 'os': ['darwin', 'linux'], 'paths': ['.config/eza']},
        'shell': {'risk': 'Unsafe', 'os': ['darwin', 'linux'], 'paths': ['.zshrc']},
        'skills': {'risk': 'Safe', 'os': ['darwin', 'linux'],
                   'paths': ['.agents/skills', '.gemini/skills']},
    }, indent=2, sort_keys=True) + '\n')
    # An explicit public engine path is supplied by each test's isolated config.


if __name__ == '__main__':
    main()
