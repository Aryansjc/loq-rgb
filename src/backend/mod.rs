//! Hardware backends behind a single trait so every code path above the
//! backend can be tested against a mock.
//!
//! Two implementations:
//! - [`hid::HidBackend`]: real hardware via hidapi on /dev/hidraw.
//! - [`mock::MockBackend`]: in-memory fake used by unit/integration tests.

pub mod hid;
pub mod mock;

use std::any::Any;

use crate::error::Error;
use crate::model::{DeviceState, LightingConfig};

/// The minimal hardware surface the rest of the app needs. `apply` is the
/// only strictly required capability; `read_state` is what the firmware
/// supports through the GET feature report and may return
/// [`Error::Unsupported`].
pub trait LightingBackend: Send {
    /// Send a full lighting configuration to the hardware. The backend
    /// serialises access and transparently re-opens the device if it was
    /// unplugged.
    fn apply(&mut self, cfg: LightingConfig) -> Result<(), Error>;

    /// Read the current lighting state back from the controller.
    fn read_state(&mut self) -> Result<DeviceState, Error>;

    /// Human-readable device description for UI/CLI output.
    fn device_desc(&self) -> String;

    /// Downcast hook for tests that need to reach into a concrete backend.
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        None
    }
}
