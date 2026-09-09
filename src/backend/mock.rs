//! In-memory backend for tests. Records every applied config and serves a
//! configurable device state, with fault injection for error paths.

use std::any::Any;

use super::LightingBackend;
use crate::error::Error;
use crate::model::{DeviceState, LightingConfig};

#[derive(Debug, Default)]
pub struct MockBackend {
    /// Every config applied, in order (normalized form).
    pub applied: Vec<LightingConfig>,
    /// State served by `read_state`.
    pub state: Option<DeviceState>,
    /// When true, `apply` fails with the stored error.
    pub fail_apply: Option<Error>,
    /// When true, `read_state` fails with the stored error.
    pub fail_read: Option<Error>,
    pub desc: String,
}

impl MockBackend {
    pub fn new() -> Self {
        Self {
            desc: "mock controller 048d:c993".into(),
            ..Default::default()
        }
    }

    pub fn last_applied(&self) -> Option<&LightingConfig> {
        self.applied.last()
    }
}

impl LightingBackend for MockBackend {
    fn apply(&mut self, cfg: LightingConfig) -> Result<(), Error> {
        if let Some(e) = &self.fail_apply {
            return Err(e.clone());
        }
        let normalized = cfg.normalized();
        self.applied.push(normalized);
        Ok(())
    }

    fn read_state(&mut self) -> Result<DeviceState, Error> {
        if let Some(e) = &self.fail_read {
            return Err(e.clone());
        }
        self.state.ok_or_else(|| {
            Error::Unsupported("mock has no state configured — set MockBackend::state".into())
        })
    }

    fn device_desc(&self) -> String {
        self.desc.clone()
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}
