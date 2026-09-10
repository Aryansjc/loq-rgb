//! Starting and managing the background animator process.
//!
//! The colour wave is host-rendered, so *something* must keep writing frames.
//! The GUI can animate while it is open, but if the user closes the window
//! the wave would freeze on its last frame. To avoid that, the GUI starts the
//! `loq-rgb-cli listen-hotkeys` daemon as a detached child the first time a
//! host-rendered effect is selected. The daemon:
//!
//! - takes the single-writer lock (so the GUI stops writing frames),
//! - applies the active profile,
//! - animates the wave, and keeps animating after the GUI exits,
//! - also provides Fn+Space profile cycling.
//!
//! The daemon is intentionally a separate process: closing the GUI must not
//! stop the lighting, and one writer at a time is a hard requirement.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::error::Error;

/// Locate the `loq-rgb-cli` binary next to the running GUI binary (or on
/// `PATH` as a fallback).
pub fn cli_binary_path() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        let sibling = exe.with_file_name("loq-rgb-cli");
        if sibling.exists() {
            return Some(sibling);
        }
    }
    // Fall back to PATH resolution by returning the bare name.
    Some(PathBuf::from("loq-rgb-cli"))
}

/// Where the daemon's stderr goes, so a failed auto-start can be diagnosed.
pub fn log_path() -> PathBuf {
    let dir = crate::instance::default_lock_dir();
    dir.join("animator.log")
}

/// Start the background animator detached from this process. Returns the
/// child PID. The child takes the writer lock itself; callers should re-check
/// the lock rather than assume the animation has started.
pub fn spawn_background_animator() -> Result<u32, Error> {
    let exe = cli_binary_path()
        .ok_or_else(|| Error::Io("cannot locate the loq-rgb-cli binary".into()))?;

    let log_file = {
        if let Some(parent) = log_path().parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path())
            .ok()
    };

    let mut cmd = Command::new(&exe);
    cmd.arg("listen-hotkeys")
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    match log_file {
        Some(f) => {
            cmd.stderr(Stdio::from(f));
        }
        None => {
            cmd.stderr(Stdio::null());
        }
    }
    // Put the child in its own process group so it survives the GUI closing
    // (and is not killed by a Ctrl+C aimed at the terminal that launched it).
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

    let child = cmd
        .spawn()
        .map_err(|e| Error::Io(format!("cannot start {}: {e}", exe.display())))?;
    Ok(child.id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_binary_path_resolves_to_something() {
        // In tests the sibling does not exist, so it falls back to the name.
        let path = cli_binary_path().expect("a path is always produced");
        assert!(!path.as_os_str().is_empty());
    }

    #[test]
    fn log_path_lives_in_the_lock_directory() {
        let log = log_path();
        assert_eq!(log.file_name().unwrap(), "animator.log");
    }
}
