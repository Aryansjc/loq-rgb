//! Single-instance coordination for lighting writers.
//!
//! Live hardware testing proved that two programs writing the lighting
//! controller at once cause visible fighting/flicker. The host-rendered
//! colour wave makes this worse because it writes ~25 frames/s.
//!
//! This module provides an advisory single-writer lock:
//! - [`WriterLock::acquire`] takes an exclusive `flock` and records the PID
//!   (used by `loq-rgb-cli listen-hotkeys`, the background animator).
//! - [`WriterLock::probe`] reports whether another instance holds the lock
//!   and who it is (used by the GUI so it can avoid animating at the same
//!   time and can tell the user which process is in control).

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

use crate::error::Error;

/// The running instance that owns the lock, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockOwner {
    pub pid: u32,
}

const LOCK_FILENAME: &str = "writer.lock";

/// Default lock location (injectable for tests).
pub fn default_lock_dir() -> PathBuf {
    if let Ok(runtime) = std::env::var("XDG_RUNTIME_DIR")
        && !runtime.is_empty()
    {
        return PathBuf::from(runtime).join("loq-rgb");
    }
    if let Ok(cache) = std::env::var("XDG_CACHE_HOME")
        && !cache.is_empty()
    {
        return PathBuf::from(cache).join("loq-rgb");
    }
    if let Ok(home) = std::env::var("HOME")
        && !home.is_empty()
    {
        return PathBuf::from(home).join(".cache").join("loq-rgb");
    }
    std::env::temp_dir().join("loq-rgb")
}

/// The advisory single-writer lock. Dropping releases it.
pub struct WriterLock {
    _file: File,
    pub owner_pid: u32,
}

impl WriterLock {
    /// Try to take the lock. Returns `Ok(Some)` on success, `Ok(None)` when
    /// another instance already holds it (no error — that is the expected
    /// "already running" outcome).
    pub fn acquire(dir: &Path) -> Result<Option<Self>, Error> {
        std::fs::create_dir_all(dir)
            .map_err(|e| Error::Io(format!("cannot create lock dir {}: {e}", dir.display())))?;
        let path = dir.join(LOCK_FILENAME);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| Error::Io(format!("cannot open lock file {}: {e}", path.display())))?;

        // flock(LOCK_EX | LOCK_NB) via libc through the raw fd.
        let fd = file.as_raw_fd();
        // SAFETY: fd is a valid open file descriptor; flock does not touch the
        // fd's lifetime. Linux: LOCK_EX=2, LOCK_NB=4.
        let rc = unsafe { libc_flock(fd, 2 | 4) };
        if rc != 0 {
            // Busy: another writer holds it.
            return Ok(None);
        }

        // We own it: record our PID (also for readers who may only have read
        // access to the file). Truncating is safe — we hold the lock.
        file.set_len(0).ok();
        let mut file = file;
        writeln!(file, "{}", std::process::id()).ok();
        file.flush().ok();
        let owner_pid = std::process::id();
        Ok(Some(Self {
            _file: file,
            owner_pid,
        }))
    }
}

/// Read who currently owns the lock, verifying that the lock is *actually*
/// held. A lock file left behind by a crashed or exited process reports
/// `None` rather than a phantom owner.
pub fn probe(dir: &Path) -> Option<LockOwner> {
    let path = dir.join(LOCK_FILENAME);
    let mut file = OpenOptions::new().read(true).write(true).open(&path).ok()?;
    let fd = file.as_raw_fd();

    // Try to take the lock ourselves. Success means nobody held it, so we
    // release immediately and report "not held".
    // Linux: LOCK_EX=2, LOCK_NB=4, LOCK_UN=8.
    let rc = unsafe { libc_flock(fd, 2 | 4) };
    if rc == 0 {
        unsafe { libc_flock(fd, 8) };
        return None;
    }

    // Someone holds it: read the PID they recorded.
    let mut content = String::new();
    file.read_to_string(&mut content).ok()?;
    let pid: u32 = content.trim().parse().ok()?;
    Some(LockOwner { pid })
}

/// True when `pid` is alive (best-effort, for display purposes).
pub fn process_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(target_os = "linux")]
unsafe fn libc_flock(fd: i32, operation: i32) -> i32 {
    unsafe extern "C" {
        fn flock(fd: i32, operation: i32) -> i32;
    }
    // SAFETY: valid fd, valid operation constants.
    unsafe { flock(fd, operation) }
}

#[cfg(not(target_os = "linux"))]
unsafe fn libc_flock(_fd: i32, _operation: i32) -> i32 {
    -1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_is_acquired_once_and_blocks_a_second_writer() {
        let dir = tempfile::tempdir().unwrap();
        let first = WriterLock::acquire(dir.path())
            .unwrap()
            .expect("first writer");
        assert_eq!(first.owner_pid, std::process::id());

        // A second acquire on the same file (even in the same process the
        // flock is per-open-file-description, so it must fail).
        let second = WriterLock::acquire(dir.path()).unwrap();
        assert!(second.is_none(), "second writer must be refused");

        // Probe reports the first owner.
        let owner = probe(dir.path()).expect("lock file exists with a pid");
        assert_eq!(owner.pid, std::process::id());
    }

    #[test]
    fn lock_is_released_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        {
            let _lock = WriterLock::acquire(dir.path()).unwrap().expect("acquire");
        }
        // After drop the lock is free again.
        let again = WriterLock::acquire(dir.path()).unwrap();
        assert!(again.is_some(), "lock must be reusable after release");
    }

    #[test]
    fn lock_stays_held_while_the_guard_is_alive() {
        // Regression: the guard must be held for as long as the caller keeps
        // it (the daemon kept a stale copy in an inner scope, so the lock was
        // released immediately and two animators could run at once).
        let dir = tempfile::tempdir().unwrap();
        let guard = WriterLock::acquire(dir.path()).unwrap().expect("acquire");
        // While `guard` is alive, another acquire is refused and probe sees us.
        assert!(
            WriterLock::acquire(dir.path()).unwrap().is_none(),
            "second writer must be refused while the guard is alive"
        );
        assert!(probe(dir.path()).is_some());
        drop(guard);
        assert!(
            probe(dir.path()).is_none(),
            "released lock must report unheld"
        );
    }

    #[test]
    fn probe_reports_stale_lock_file_as_unheld() {
        // A lock file whose owner has exited must NOT look like a live owner.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path()).unwrap();
        std::fs::write(dir.path().join(LOCK_FILENAME), "999999\n").unwrap();
        assert_eq!(
            probe(dir.path()),
            None,
            "an unheld lock file must not report a phantom owner"
        );
    }

    #[test]
    fn probe_missing_lock_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(probe(dir.path()), None);
    }

    #[test]
    fn process_alive_for_self() {
        assert!(process_alive(std::process::id()));
        assert!(!process_alive(u32::MAX));
    }
}
