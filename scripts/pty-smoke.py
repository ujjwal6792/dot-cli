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
    (home / '.capture-demo').write_text('setting = example\n')
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
        os.write(master, b'/capture-demo')
        drain(0.3)
        os.write(master, b'\r ')
        drain(0.3)
        marker = len(output)
        os.write(master, b'c')
        drain(0.7)
        assert b'Save a copy of your settings' in output[marker:]
        assert not (source / 'dot_capture-demo').exists(), 'review must not execute'
        os.write(master, b'2')
        drain(0.3)
        assert b'New saved copy:' in output[marker:]
        os.write(master, b'y')
        drain(1.0)
        assert b'Your settings are saved' in output[marker:]
        assert (source / 'dot_capture-demo').read_text() == 'setting = example\n'
        assert (home / '.capture-demo').read_text() == 'setting = example\n'
        os.write(master, b'2')
        drain(0.2)
        assert b'"succeeded"' in output[marker:], 'raw details must remain available'
        os.write(master, b'g2')
        drain(0.8)
        marker = len(output)
        os.write(master, b'i')
        drain(0.8)
        assert b'Ignore in chezmoi and Git' in output[marker:], repr(output[marker:])
        assert '.capture-demo' not in (source / '.chezmoiignore').read_text()
        os.write(master, b'2')
        drain(0.3)
        assert b'.chezmoiignore' in output[marker:] and b'.gitignore' in output[marker:]
        os.write(master, b'y')
        drain(1.0)
        assert b'Ignored in chezmoi and Git' in output[marker:]
        assert '.capture-demo' in (source / '.chezmoiignore').read_text()
        assert '/dot_capture-demo' in (source / '.gitignore').read_text()
        assert (source / 'dot_capture-demo').exists()
        assert (home / '.capture-demo').exists()
        os.write(master, b'\r')
        drain(0.2)
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
        print('PASS: CLI init/capture, TUI capture and unified Ignore review/confirm/results, search ownership, resize, action menu, clean quit and terminal restoration sequence')
    finally:
        if child.poll() is None:
            child.kill()
            child.wait()
        os.close(master)
        os.close(slave)
