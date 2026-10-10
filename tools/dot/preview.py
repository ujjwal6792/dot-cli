#!/usr/bin/env python3
"""Progressive disclosure for unified diff output; never execute file names."""
from pathlib import Path
import os
import sys


def sections(text):
    result = []
    prefix = []
    for line in text.splitlines():
        if line.startswith('diff --git '):
            result.append([line])
        elif result:
            result[-1].append(line)
        else:
            prefix.append(line)
    return prefix, result


def label(block, index):
    # Preserve git's quoting for paths containing unusual characters.
    name = block[0].removeprefix('diff --git ')
    for line in block:
        if line.startswith('+++ ') and line[4:] != '/dev/null':
            name = line[4:].removeprefix('b/')
            break
        if line.startswith('--- ') and line[4:] != '/dev/null':
            name = line[4:].removeprefix('a/')
    added = sum(line.startswith('+') and not line.startswith('+++') for line in block)
    removed = sum(line.startswith('-') and not line.startswith('---') for line in block)
    return f'{index + 1}: {name}  +{added} -{removed}'


def main():
    command, file, *args = sys.argv[1:]
    text = Path(file).read_text()
    prefix, blocks = sections(text)
    if command == 'summary':
        print(f'Changed-file summary: {len(blocks)} diff sections')
        # Non-diff output can include warnings; never silently discard it.
        print('\n'.join(prefix))
        for index, block in enumerate(blocks):
            print(label(block, index))
        if not blocks:
            print('No unified diff sections. Choose All changes to inspect full output.')
    elif command == 'choices':
        for index, block in enumerate(blocks):
            print(label(block, index))
    else:
        selection = args[0]
        lines = text.splitlines() if selection == 'All changes' else blocks[int(selection.split(':', 1)[0]) - 1]
        for line in lines:
            color = ''
            if command == 'color' and 'NO_COLOR' not in os.environ:
                if line.startswith('+'):
                    color = '\033[32m'
                elif line.startswith('-'):
                    color = '\033[31m'
                elif line.startswith(('@@', 'diff --git')):
                    color = '\033[36m'
            print(color + line + ('\033[0m' if color else ''))


if __name__ == '__main__':
    main()
