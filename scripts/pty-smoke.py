#!/usr/bin/env python3
"""PTY smoke test of the real binary: start, browse, help, resize, quit.

Usage: python3 -I scripts/pty-smoke.py <path-to-dotwarden>
Runs in a throw-away HOME; never touches the real one.
"""
import fcntl, os, pty, re, select, signal, struct, subprocess, sys, tempfile, termios, time

binary = os.path.abspath(sys.argv[1])
root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
home = tempfile.mkdtemp(prefix="dw-pty-")
os.makedirs(os.path.join(home, ".config/nvim"))
os.makedirs(os.path.join(home, ".cache"))
open(os.path.join(home, ".zshrc"), "w").write("export A=1\n")
open(os.path.join(home, ".gitconfig"), "w").write("[user]\n\tname = T\n\temail = t@example.com\n")
env = {
    "HOME": home,
    "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
    "TERM": "xterm-256color",
    "DOTWARDEN_LIBEXEC": os.environ.get("DOTWARDEN_LIBEXEC", os.path.join(root, "libexec")),
}
subprocess.run([binary, "init", "--name", "smoke"], env=env, check=True, stdout=subprocess.DEVNULL)

def setsize(fd, rows, cols):
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))

master, slave = pty.openpty()
setsize(slave, 24, 80)
proc = subprocess.Popen([binary], env=env, stdin=slave, stdout=slave, stderr=slave, close_fds=True)
os.close(slave)
screen = b""

def pump(until, timeout=5.0):
    global screen
    deadline = time.time() + timeout
    while time.time() < deadline:
        if until(screen):
            return
        r, _, _ = select.select([master], [], [], 0.1)
        if r:
            try:
                screen += os.read(master, 65536)
            except OSError:
                break
    raise SystemExit(f"timeout waiting; tail of output: {screen[-400:]!r}")

def expect(text, timeout=5.0):
    pump(lambda s: text.encode() in s, timeout)

def mark():
    global screen
    screen = b""

try:
    expect("[2] Files")
    expect("[5] Machines")
    expect(".zshrc")
    assert b".cache" not in screen, "hidden item was shown"
    mark(); os.write(master, b"?"); expect("Help (Esc to close)")
    mark(); os.write(master, b"\x1b"); time.sleep(0.3)
    mark(); os.write(master, b"."); expect(".cache")
    mark(); setsize(master, 14, 45); proc.send_signal(signal.SIGWINCH)
    pump(lambda s: b"[2] Files" in s and b"[3] Changes" not in s, 5.0)
    mark(); os.write(master, b"q")
    code = proc.wait(timeout=5)
    assert code == 0, f"exit code {code}"
    print("pty smoke: OK (start, hidden toggle, help, resize to 45x14, quit)")
finally:
    if proc.poll() is None:
        proc.kill()
