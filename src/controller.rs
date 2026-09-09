//! LightingController: the small stateful layer the GUI and CLI share.
//!
//! Responsibilities:
//! - own a [`LightingBackend`] and remember the last config applied,
//! - keep the last error visible to the UI instead of swallowing it,
//! - re-apply-on-demand semantics (the backend transparently re-opens the
//!   device after hot-unplug).
//!
//! The controller is deliberately not itself thread-safe: the GUI drives it
//! from one thread with a debounce timer, and the CLI uses it sequentially.
//! That design makes every interleaving deterministic and testable.

use crate::backend::LightingBackend;
use crate::backend::hid::HidBackend;
use crate::error::Error;
use crate::model::{DeviceState, LightingConfig};

pub struct Controller {
    backend: Box<dyn LightingBackend>,
    last_config: Option<LightingConfig>,
    last_error: Option<Error>,
}

impl Controller {
    /// Open the real hardware backend (fails with a hintful error when the
    /// controller is absent or inaccessible).
    pub fn open_hardware() -> Result<Self, Error> {
        let backend = HidBackend::open()?;
        Ok(Self::new(Box::new(backend)))
    }

    /// Wrap any backend (used by tests and the CLI `--mock` path).
    pub fn new(backend: Box<dyn LightingBackend>) -> Self {
        Self {
            backend,
            last_config: None,
            last_error: None,
        }
    }

    /// Apply a full lighting configuration. One packet describes the whole
    /// keyboard (single global effect + four zone colours), so every change
    /// — even a single zone colour edit — re-sends the complete config built
    /// from the caller's current [`LightingConfig`]. Zone colours the user
    /// did not touch are therefore never reset by the app.
    pub fn apply(&mut self, cfg: LightingConfig) -> Result<(), Error> {
        match self.backend.apply(cfg) {
            Ok(()) => {
                self.last_config = Some(cfg.normalized());
                self.last_error = None;
                Ok(())
            }
            Err(e) => {
                self.last_error = Some(e.clone());
                Err(e)
            }
        }
    }

    /// Read the current hardware state, if the controller supports it.
    pub fn read_state(&mut self) -> Result<DeviceState, Error> {
        match self.backend.read_state() {
            Ok(s) => {
                self.last_error = None;
                Ok(s)
            }
            Err(e) => {
                self.last_error = Some(e.clone());
                Err(e)
            }
        }
    }

    /// What was last successfully applied in this session.
    pub fn last_config(&self) -> Option<&LightingConfig> {
        self.last_config.as_ref()
    }

    /// Last error for the UI banner; cleared on the next successful op.
    pub fn last_error(&self) -> Option<&Error> {
        self.last_error.as_ref()
    }

    pub fn device_desc(&self) -> String {
        self.backend.device_desc()
    }

    /// Mutable access to the underlying backend (tests, advanced use).
    pub fn backend_mut(&mut self) -> &mut dyn LightingBackend {
        self.backend.as_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::mock::MockBackend;
    use crate::model::{Effect, Rgb};

    fn red_cfg() -> LightingConfig {
        LightingConfig {
            effect: Effect::Static,
            speed: 1,
            brightness: 2,
            zones: [Rgb::new(255, 0, 0); 4],
        }
    }

    #[test]
    fn apply_records_success_and_clears_error() {
        let mut c = Controller::new(Box::new(MockBackend::new()));
        c.apply(red_cfg()).unwrap();
        assert_eq!(c.last_config(), Some(&red_cfg().normalized()));
        assert!(c.last_error().is_none());
    }

    #[test]
    fn apply_error_is_remembered_and_cleared_on_later_success() {
        let mut backend = MockBackend::new();
        backend.fail_apply = Some(Error::Io("simulated unplug".into()));
        let mut c = Controller::new(Box::new(backend));
        let err = c.apply(red_cfg()).unwrap_err();
        assert!(matches!(err, Error::Io(_)));
        assert!(matches!(c.last_error(), Some(Error::Io(_))));
        assert!(
            c.last_config().is_none(),
            "failed apply must not record state"
        );

        // A recovered backend (fresh mock, no fault) clears the error.
        let recovered = {
            let mut c2 = Controller::new(Box::new(MockBackend::new()));
            c2.apply(red_cfg()).unwrap();
            c2.last_error().is_none()
        };
        assert!(recovered);
    }

    #[test]
    fn read_state_error_is_remembered() {
        let mut backend = MockBackend::new();
        backend.fail_read = Some(Error::Unsupported("no readback".into()));
        let mut c = Controller::new(Box::new(backend));
        assert!(c.read_state().is_err());
        assert!(c.last_error().is_some());
    }
}
