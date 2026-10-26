//! The mutation lock (spec §9): mutating commands take it and wait up to 10 s
//! rather than failing instantly. Read-only commands never take it.

use std::fs::{self, File, TryLockError};
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

pub const DEFAULT_WAIT: Duration = Duration::from_secs(10);
/// Overrides the wait in milliseconds (tests).
pub const WAIT_ENV: &str = "DOTWARDEN_LOCK_WAIT_MS";

/// The lock stayed held by another dotwarden command for the whole wait. Callers
/// that can retry (the TUI) look for this to say "busy, try again".
#[derive(Debug)]
pub struct Busy(pub String);

impl std::fmt::Display for Busy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Busy {}

pub fn is_busy(e: &anyhow::Error) -> bool {
    e.chain().any(|c| c.downcast_ref::<Busy>().is_some())
}

/// Call right after releasing a lock you are about to take again: flock is not
/// fair and waiters poll, so without a pause the next iteration would win every
/// time and starve other commands.
pub fn yield_to_waiters() {
    thread::sleep(Duration::from_millis(3 * POLL.as_millis() as u64));
}

const POLL: Duration = Duration::from_millis(25);

/// Held for as long as the value lives; released when dropped or the process exits.
#[derive(Debug)]
pub struct Lock {
    _file: File,
}

pub fn wait_from_env(env: &crate::paths::Env) -> Duration {
    env.get(std::ffi::OsStr::new(WAIT_ENV))
        .and_then(|v| v.to_str())
        .and_then(|v| v.parse::<u64>().ok())
        .map_or(DEFAULT_WAIT, Duration::from_millis)
}

/// Take the exclusive lock stored at `path`, waiting up to `wait`.
pub fn acquire(path: &Path, wait: Duration) -> Result<Lock> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    lock_file(file, path, wait)
}

/// Like [`acquire`] but never creates anything: `None` when the lock file does
/// not exist yet (read-only commands must not write).
pub fn acquire_existing(path: &Path, wait: Duration) -> Result<Option<Lock>> {
    match File::open(path) {
        Ok(file) => lock_file(file, path, wait).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("opening {}", path.display())),
    }
}

fn lock_file(file: File, path: &Path, wait: Duration) -> Result<Lock> {
    let deadline = Instant::now() + wait;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(Lock { _file: file }),
            Err(TryLockError::WouldBlock) => {}
            Err(TryLockError::Error(e)) => {
                return Err(e).with_context(|| format!("locking {}", path.display()));
            }
        }
        if Instant::now() >= deadline {
            return Err(anyhow::Error::new(Busy(format!(
                "another dotwarden command has held the lock for more than {} s ({})",
                wait.as_secs_f32(),
                path.display()
            ))));
        }
        thread::sleep(POLL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_for_a_holder_that_releases_in_time() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("lock");
        let held = acquire(&p, Duration::from_secs(1)).unwrap();
        let releaser = thread::spawn(move || {
            thread::sleep(Duration::from_millis(300));
            drop(held);
        });
        let start = Instant::now();
        let _second = acquire(&p, Duration::from_secs(5)).unwrap();
        assert!(
            start.elapsed() >= Duration::from_millis(250),
            "did not wait"
        );
        releaser.join().unwrap();
    }

    #[test]
    fn gives_up_after_the_wait_with_a_clear_error() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("lock");
        let _held = acquire(&p, Duration::from_secs(1)).unwrap();
        let start = Instant::now();
        let e = acquire(&p, Duration::from_millis(200)).unwrap_err();
        assert!(start.elapsed() >= Duration::from_millis(190));
        assert!(e.to_string().contains("another dotwarden command"), "{e}");
    }

    #[test]
    fn released_when_dropped() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("lock");
        drop(acquire(&p, Duration::from_secs(1)).unwrap());
        acquire(&p, Duration::from_millis(100)).unwrap();
    }
}
