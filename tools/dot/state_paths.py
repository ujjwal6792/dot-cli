"""Resolve the one runtime state directory without falling back on errors."""
import os
from pathlib import Path


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
