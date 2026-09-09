//! Error taxonomy with user-actionable hints. Every failure path in the app
//! renders one of these; no error is ever swallowed silently.

use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum Error {
    #[error("No supported keyboard controller found: {0}")]
    DeviceNotFound(String),

    #[error("Permission denied opening the controller: {0}")]
    PermissionDenied(String),

    #[error("I/O error talking to the controller: {0}")]
    Io(String),

    #[error("Unsupported operation: {0}")]
    Unsupported(String),

    #[error("Protocol error: {0}")]
    Protocol(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Another loq-rgb writer instance is already running: {0}")]
    AlreadyRunning(String),
}

impl Error {
    /// Guidance text the CLI/UI can show next to the error.
    pub fn hint(&self) -> &'static str {
        match self {
            Error::DeviceNotFound(_) => {
                "This tool targets the ITE 4-zone RGB controllers (048d:c993 = LOQ 2024). \
                 Run `lsusb` to check whether a 048d device is present, or `loq-rgb-cli detect`."
            }
            Error::PermissionDenied(_) => {
                "Install the udev rule once: `sudo loq-rgb-cli install-udev`, then reload rules \
                 (`sudo udevadm control --reload-rules && sudo udevadm trigger`) and reconnect \
                 the keyboard. Alternatively run the app with sudo."
            }
            Error::Io(_) => {
                "The controller may have been unplugged or suspended. The app retries \
                 automatically; if this persists, replug the laptop's internal keyboard \
                 (power cycle) or check `dmesg` for USB errors."
            }
            Error::Unsupported(_) => {
                "Your hardware reports it cannot do this. Only controls the device actually \
                 supports are enabled."
            }
            Error::Protocol(_) => {
                "The controller answered with data this app could not interpret. Report this \
                 output to the project."
            }
            Error::Config(_) => {
                "The configuration file was invalid; a backup was kept and defaults restored. \
                 Check the reported path."
            }
            Error::AlreadyRunning(_) => {
                "Only one lighting writer may run at a time. Stop the existing instance first: \
                 `pkill -f 'loq-rgb-cli listen-hotkeys'` (or close the other loq-rgb window)."
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_variant_has_a_hint() {
        for e in [
            Error::DeviceNotFound("x".into()),
            Error::PermissionDenied("x".into()),
            Error::Io("x".into()),
            Error::Unsupported("x".into()),
            Error::Protocol("x".into()),
            Error::Config("x".into()),
            Error::AlreadyRunning("x".into()),
        ] {
            assert!(!e.hint().is_empty());
            let _ = format!("{e}"); // Display must not panic
        }
    }
}
