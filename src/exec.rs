//! Running external tools without ever blocking the caller's thread on pipes,
//! with optional cancellation.

use std::io::Read;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};

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

fn drain<R: Read + Send + 'static>(mut r: R) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = r.read_to_end(&mut buf);
        buf
    })
}

/// Run `cmd` to completion, capturing output. When `cancel` fires the child is
/// killed and an error is returned. stdin is closed.
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
    let join = |h: thread::JoinHandle<Vec<u8>>| {
        h.join()
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .map_err(|_| anyhow!("output reader panicked"))
    };
    Ok(Output {
        status,
        stdout: join(out)?,
        stderr: join(err)?,
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
