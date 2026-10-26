//! Running external tools without ever blocking the caller's thread on pipes,
//! with optional cancellation.

use std::io::Read;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

/// Cooperative cancellation flag shared between a job and its owner.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(Debug)]
pub struct Output {
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn success(&self) -> bool {
        self.status.success()
    }
}

/// How long to keep collecting output after the child has exited. A grandchild
/// that inherited the pipes (ssh with ControlPersist, a daemonised helper) can
/// hold them open for ever; whatever it printed later is not part of the result.
const PIPE_GRACE: Duration = Duration::from_millis(300);

/// Collects a pipe on its own thread. `done` is signalled at end of file.
struct Drain {
    buf: Arc<Mutex<Vec<u8>>>,
    done: mpsc::Receiver<()>,
}

fn drain<R: Read + Send + 'static>(mut r: R) -> Drain {
    let buf = Arc::new(Mutex::new(Vec::new()));
    let (tx, done) = mpsc::channel();
    let shared = Arc::clone(&buf);
    thread::spawn(move || {
        let mut chunk = [0u8; 8192];
        while let Ok(n) = r.read(&mut chunk) {
            if n == 0 {
                break;
            }
            shared
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .extend_from_slice(&chunk[..n]);
        }
        let _ = tx.send(());
    });
    Drain { buf, done }
}

impl Drain {
    /// Wait until `deadline` for end of file, then return what was read.
    fn finish(self, deadline: Instant) -> String {
        let left = deadline.saturating_duration_since(Instant::now());
        let _ = self.done.recv_timeout(left);
        let bytes = self.buf.lock().unwrap_or_else(|p| p.into_inner()).clone();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

/// Run `cmd` to completion, capturing output. When `cancel` fires the child is
/// killed and an error is returned. stdin is closed. Returns shortly after the
/// child exits even if a grandchild keeps its output pipes open.
pub fn run(mut cmd: Command, cancel: Option<&CancelToken>) -> Result<Output> {
    let desc = format!("{:?}", cmd.get_program());
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("starting {desc}"))?;
    let out = drain(child.stdout.take().expect("piped"));
    let err = drain(child.stderr.take().expect("piped"));
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if cancel.is_some_and(CancelToken::is_cancelled) {
            let _ = child.kill();
            let _ = child.wait();
            bail!("cancelled");
        }
        thread::sleep(Duration::from_millis(5));
    };
    let deadline = Instant::now() + PIPE_GRACE;
    Ok(Output {
        status,
        stdout: out.finish(deadline),
        stderr: err.finish(deadline),
    })
}

/// Like [`run`] but turns a non-zero exit into an error carrying stderr.
pub fn run_ok(cmd: Command, cancel: Option<&CancelToken>) -> Result<Output> {
    let desc = describe(&cmd);
    let out = run(cmd, cancel)?;
    if !out.success() {
        bail!(
            "`{desc}` failed ({}): {}",
            out.status,
            out.stderr
                .trim()
                .lines()
                .last()
                .unwrap_or("no error output")
        );
    }
    Ok(out)
}

pub fn describe(cmd: &Command) -> String {
    let mut parts = vec![
        std::path::Path::new(cmd.get_program())
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
    ];
    parts.extend(cmd.get_args().map(|a| a.to_string_lossy().into_owned()));
    parts.join(" ")
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn captures_output_and_status() {
        let mut c = Command::new("sh");
        c.args(["-c", "echo out; echo err >&2; exit 3"]);
        let o = run(c, None).unwrap();
        assert_eq!(o.stdout.trim(), "out");
        assert_eq!(o.stderr.trim(), "err");
        assert_eq!(o.status.code(), Some(3));
    }

    #[test]
    fn run_ok_reports_stderr() {
        let mut c = Command::new("sh");
        c.args(["-c", "echo boom >&2; exit 1"]);
        let e = run_ok(c, None).unwrap_err().to_string();
        assert!(e.contains("boom"), "{e}");
    }

    #[test]
    fn returns_when_a_grandchild_keeps_the_pipes_open() {
        // `sleep` inherits stdout/stderr and outlives the shell, like ssh
        // with ControlPersist. run() must not wait for it.
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut c = Command::new("sh");
            c.args(["-c", "sleep 8 & echo hello; echo oops >&2"]);
            let start = Instant::now();
            let out = run(c, None);
            let _ = tx.send((out, start.elapsed()));
        });
        let (out, took) = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("run() hung on the grandchild's pipes");
        let out = out.unwrap();
        assert!(took < Duration::from_secs(3), "{took:?}");
        assert_eq!(out.stdout.trim(), "hello");
        assert_eq!(out.stderr.trim(), "oops");
    }

    #[test]
    fn cancellation_kills_the_child_promptly() {
        let token = CancelToken::new();
        let t2 = token.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            t2.cancel();
        });
        let mut c = Command::new("sleep");
        c.arg("10");
        let start = Instant::now();
        let e = run(c, Some(&token)).unwrap_err().to_string();
        assert_eq!(e, "cancelled");
        assert!(start.elapsed() < Duration::from_secs(3));
    }
}
