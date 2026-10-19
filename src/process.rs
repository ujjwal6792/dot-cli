use anyhow::{Context, Result, bail};
use std::{
    io::{Read, Write},
    os::unix::process::CommandExt,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// No shell expansion. Credentials remain with system Git; background commands never prompt.
pub fn run(
    program: &str,
    args: &[String],
    cwd: Option<&Path>,
    input: Option<&[u8]>,
) -> Result<String> {
    let output = output(program, args, cwd, input)?;
    if !output.status.success() {
        bail!(
            "{} exited {}: {}",
            program,
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
pub fn output(
    program: &str,
    args: &[String],
    cwd: Option<&Path>,
    input: Option<&[u8]>,
) -> Result<std::process::Output> {
    output_with_timeout(
        program,
        args,
        cwd,
        input,
        Duration::from_secs(
            std::env::var("DOT_COMMAND_TIMEOUT")
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(120)
                .clamp(1, 3600),
        ),
    )
}
pub fn output_with_timeout(
    program: &str,
    args: &[String],
    cwd: Option<&Path>,
    input: Option<&[u8]>,
    timeout: Duration,
) -> Result<std::process::Output> {
    let mut cmd = Command::new(program);
    cmd.process_group(0);
    cmd.args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_PAGER", "cat")
        .env("NO_COLOR", "1")
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(path) = cwd {
        cmd.current_dir(path);
    }
    let mut child = cmd.spawn().with_context(|| {
        format!("Cannot start {program}; install it or configure its executable path")
    })?;
    let writer = if let Some(bytes) = input {
        let mut stdin = child.stdin.take().context("stdin unavailable")?;
        let bytes = bytes.to_vec();
        Some(std::thread::spawn(move || stdin.write_all(&bytes)))
    } else {
        None
    };
    let mut stdout = child.stdout.take().context("stdout unavailable")?;
    let mut stderr = child.stderr.take().context("stderr unavailable")?;
    let out = std::thread::spawn(move || {
        let mut b = vec![];
        stdout.read_to_end(&mut b).map(|_| b)
    });
    let err = std::thread::spawn(move || {
        let mut b = vec![];
        stderr.read_to_end(&mut b).map(|_| b)
    });
    let started = Instant::now();
    let mut status = None;
    loop {
        if status.is_none() {
            status = child.try_wait()?;
        }
        if status.is_some()
            && out.is_finished()
            && err.is_finished()
            && writer.as_ref().is_none_or(|w| w.is_finished())
        {
            break;
        }
        if started.elapsed() >= timeout {
            // Each child has its own group. Terminate descendants too, so an
            // abandoned hook cannot keep the output pipes or writer lock alive.
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            if status.is_none() {
                child.wait()?;
            }
            // Do not join potentially inherited pipes after the deadline.
            bail!(
                "{program} timed out; inspect partial results before retrying. DOT_COMMAND_TIMEOUT can extend the limit."
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    if let Some(writer) = writer {
        writer
            .join()
            .map_err(|_| anyhow::anyhow!("stdin writer stopped"))??;
    }
    let stdout = out
        .join()
        .map_err(|_| anyhow::anyhow!("stdout reader stopped"))??;
    let stderr = err
        .join()
        .map_err(|_| anyhow::anyhow!("stderr reader stopped"))??;
    Ok(std::process::Output {
        status: status.context("command status unavailable")?,
        stdout,
        stderr,
    })
}
pub fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| s.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timeout_reaps_hook_descendants() {
        let start = Instant::now();
        let r = output_with_timeout(
            "sh",
            &strings(&["-c", "sleep 5"]),
            None,
            None,
            Duration::from_millis(50),
        );
        assert!(r.unwrap_err().to_string().contains("timed out"));
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn timeout_covers_pipes_after_parent_exits() {
        let start = Instant::now();
        let r = output_with_timeout(
            "sh",
            &strings(&["-c", "sleep 5 & exit 0"]),
            None,
            None,
            Duration::from_millis(50),
        );
        assert!(r.unwrap_err().to_string().contains("timed out"));
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn drains_output_while_writing_large_input() {
        let bytes = vec![b'x'; 1024 * 1024];
        let output = output_with_timeout(
            "sh",
            &strings(&["-c", "dd if=/dev/zero bs=1024 count=1024 2>/dev/null; cat"]),
            None,
            Some(&bytes),
            Duration::from_secs(3),
        )
        .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout.len(), bytes.len() * 2);
        assert_eq!(&output.stdout[bytes.len()..], bytes);
    }
}
