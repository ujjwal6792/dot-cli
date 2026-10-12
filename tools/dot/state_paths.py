"""Resolve the one runtime state directory without falling back on errors."""
import os
from pathlib import Path
import sys


def validate_directory(directory):
    directory = Path(directory)
    value = str(directory)
    if (not directory.is_absolute() or os.path.normpath(value) != value or
            any(ord(char) < 32 or ord(char) == 127 for char in value)):
        raise ValueError('state directory must be absolute, normalized, and contain no control characters')
    for component in (directory, *directory.parents):
        if component.is_symlink():
            raise ValueError('state directory must not traverse symlinks: ' + str(component))
    return directory


def state_dir(home=None):
    home = Path(home if home is not None else os.environ['HOME'])
    explicit = os.environ.get('DOT_STATE_DIR')
    if explicit is not None:
        if not explicit:
            raise ValueError('DOT_STATE_DIR must be an absolute, normalized path without control characters')
        directory = validate_directory(explicit)
    elif os.environ.get('XDG_STATE_HOME'):
        directory = Path(os.environ['XDG_STATE_HOME']) / 'dot'
    else:
        directory = home.resolve() / '.local/state/dot'
    return validate_directory(directory)


def main():
    command = sys.argv[1:] or ['resolve']
    if command == ['resolve']:
        print(state_dir())
    elif command == ['approval-version']:
        import scope
        approval = scope.load(os.environ['HOME'])
        print(approval['version'] if approval is not None else 0)
    elif command == ['split-install']:
        import install_state
        print('yes' if install_state.is_split_install(state_dir()) else 'no')
    else:
        raise ValueError('unknown state-path operation')


if __name__ == '__main__':
    try:
        main()
    except (KeyError, OSError, ValueError) as error:
        print('dot state: ' + str(error), file=sys.stderr)
        raise SystemExit(1)
