#!/usr/bin/env python3
"""Regression: missing-source errors stay dismissed and allow terminal exit."""
import fcntl
import os
import pathlib
import pty
import re
import select
import struct
import subprocess
import sys
import tempfile
import termios
import time

binary = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/dot').resolve()
with tempfile.TemporaryDirectory(prefix='dot-missing-source-') as temp:
    root = pathlib.Path(temp)
    (root / 'home').mkdir()
    (root / 'home/.visible-on-startup').write_text('fixture\n')
    (root / 'native').mkdir()
    (root / 'native/chezmoi.toml').write_text('')
    args = [str(binary), '--config', str(root / 'dot.toml'), '--home', str(root / 'home'),
            '--source', str(root / 'missing'), '--state-dir', str(root / 'state'),
            '--chezmoi-config', str(root / 'native/chezmoi.toml')]
    for back, quit_key in [(b'\x1b', b'q'), (b'\r', b'q'), (None, b'q'), (None, b'\x03'), (b'i', b'q')]:
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 100, 0, 0))
        def child_setup():
            os.setsid()
            fcntl.ioctl(0, termios.TIOCSCTTY, 0)
        child = subprocess.Popen(args, stdin=slave, stdout=slave, stderr=slave,
                                 env={**os.environ, 'TERM': 'xterm-256color'}, preexec_fn=child_setup)
        output = bytearray()
        def drain(seconds):
            deadline = time.monotonic() + seconds
            while time.monotonic() < deadline:
                if select.select([master], [], [], 0.05)[0]:
                    try:
                        output.extend(os.read(master, 65536))
                    except OSError:
                        break
        try:
            drain(0.8)
            assert b'Welcome to dot' in output, output.decode(errors='replace')
            assert child.poll() is None
            if back == b'i':
                assert not (root / 'missing').exists(), 'browsing must not initialize source'
                assert b'Start fresh' in output
                os.write(master, back)
                deadline = time.monotonic() + 5
                while not (root / 'home/.agents/skills/dot/SKILL.md').exists() and time.monotonic() < deadline:
                    drain(0.1)
                drain(0.5)
                # Incremental redraws can move the cursor across unchanged spaces.
                visible = re.sub(rb'\x1b\[[0-?]*[ -/]*[@-~]', b' ', output)
                assert re.search(rb'Created\s+a\s+folder\s+for\s+your\s+saved\s+settings', visible)
                assert b'Powered by:' in output
                assert b'Open files' in output
                assert (root / 'missing/.git').is_dir(), 'setup action did not initialize Git source'
                assert (root / 'home/.agents/skills/dot/SKILL.md').is_file(), 'setup did not install the agent skill'
                assert (root / 'home/.visible-on-startup').read_text() == 'fixture\n'
            elif back is not None:
                # Reproduce the expired refresh timer which caused the modal loop.
                if back == b'\x1b':
                    drain(15.5)
                dismissed_at = len(output)
                os.write(master, back)
                drain(0.3)
                assert b'.visible-on-startup' in output[dismissed_at:], 'HOME files require navigation to appear'
                marker = len(output)
                os.write(master, b'?')
                drain(0.3)
                assert b'Keyboard and pointer help' in output[marker:], 'error reopened and swallowed help'
                os.write(master, b'\x1b')
                drain(0.2)
            os.write(master, quit_key)
            drain(0.5)
            child.wait(timeout=3)
            assert child.returncode == 0, output.decode(errors='replace')
            assert b'\x1b[?1049l' in output, 'terminal restoration missing'
        finally:
            if child.poll() is None:
                child.kill()
                child.wait()
            os.close(master)
            os.close(slave)
print('PASS: startup files, dismissal, expired timer, q/Ctrl+C exit, direct initialization and terminal restoration')
