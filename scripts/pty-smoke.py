#!/usr/bin/env python3
"""Exercise the built TUI against a private temporary fixture, never the real HOME."""
import fcntl
import json
import os
import pathlib
import pty
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time

binary = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else 'target/debug/dot').resolve()
with tempfile.TemporaryDirectory(prefix='dot-pty-') as temp:
    root = pathlib.Path(temp)
    home, source, state = [root / name for name in ('home', 'source', 'state')]
    for path in (home, source, state, root / 'chez-config'):
        path.mkdir()
    native = root / 'chez-config/chezmoi.toml'
    native.write_text('')
    dotconfig = root / 'dot.toml'
    args = [str(binary), '--config', str(dotconfig), '--home', str(home), '--source', str(source), '--state-dir', str(state), '--chezmoi-config', str(native)]
    def run(*command):
        return subprocess.check_output(args + list(command), text=True)
    run('init', '--machine', 'fixture-mac')
    (home / '.config/ghostty').mkdir(parents=True)
    (home / '.config/ghostty/config').write_text('font-size = 14\n')
    plan = json.loads(run('plan', 'capture', '.config/ghostty/config'))
    result = json.loads(run('execute', plan['id'], '--approve', plan['id']))
    assert result['failed'] is None, result
    (home / '.config/ghostty/config').write_text('font-size = 16\n')
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 35, 120, 0, 0))
    original = termios.tcgetattr(slave)
    def child_setup():
        os.setsid()
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)
    env = os.environ.copy()
    env['TERM'] = 'xterm-256color'
    child = subprocess.Popen(args, stdin=slave, stdout=slave, stderr=slave, env=env, preexec_fn=child_setup)
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
        drain(1.5)
        assert child.poll() is None, output.decode(errors='replace')
        os.write(master, b'/q2Wconfig')
        drain(0.4)
        assert child.poll() is None, 'search text exited the application'
        os.write(master, b'\x1b')
        drain(0.2)
        # All actions remains reachable at narrow width.
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 14, 45, 0, 0))
        os.kill(child.pid, signal.SIGWINCH)
        os.write(master, b':')
        drain(0.3)
        os.write(master, b'\x1b')
        drain(0.3)
        os.write(master, b'q')
        drain(0.5)
        if child.poll() is None:
            os.write(master, b'q')
        child.wait(timeout=5)
        assert child.returncode == 0, output.decode(errors='replace')
        text = output.decode(errors='replace')
        try:
            restored = termios.tcgetattr(slave)
            assert restored[3] == original[3], 'terminal local flags were not restored'
        except termios.error:
            # macOS revokes the slave after the controlling session exits.
            assert '\x1b[?1049l' in text, 'alternate-screen restoration was not emitted'
        assert 'Files' in text and 'Actions' in text and 'q2Wconfig' in text
        print('PASS: CLI init/capture, real PTY launch, search ownership, resize, action menu, clean quit and terminal restoration sequence')
    finally:
        if child.poll() is None:
            child.kill()
            child.wait()
        os.close(master)
        os.close(slave)
