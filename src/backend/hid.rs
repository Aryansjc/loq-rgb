//! Real hardware backend over hidapi (hidraw). The keyboard lighting
//! controller enumerates as `048d:c993` "ITE Device(8295)" with a
//! vendor-defined HID interface (usage page 0xff89, usage 0x00cc).
//!
//! Design notes:
//! - We talk feature reports (Report ID 0xCC) through hidraw, so the
//!   kernel's `hid-generic` driver stays bound to the device — nothing is
//!   detached or unbound, and the key matrix (a separate USB device) is
//!   never touched.
//! - The controller exposes two HID interfaces. We prefer the one whose
//!   usage matches 0xff89/0x00cc (the tuple documented by
//!   legion-keyboard-custom); if no interface reports that usage we fall
//!   back to the first `048d:c993` interface and let the feature-report
//!   round trip be the arbiter.
//! - After an I/O error the handle is dropped so the next call re-opens the
//!   device (hot-unplug/replug recovery).

use std::ffi::CString;

use hidapi::{HidApi, HidDevice, HidError};

use super::LightingBackend;
use crate::detect;
use crate::error::Error;
use crate::model::{DeviceState, LightingConfig};
use crate::packet::{self, PACKET_LEN, REPORT_ID};

/// Vendor usage page reported by the lighting interface.
const USAGE_PAGE: u16 = 0xff89;
const USAGE: u16 = 0x00cc;

pub struct HidBackend {
    api: HidApi,
    device: Option<HidDevice>,
    /// USB path of the last opened device, used for friendly re-open.
    last_path: Option<String>,
    desc: String,
}

impl HidBackend {
    /// Detect the controller on the bus and open a backend for it.
    pub fn open() -> Result<Self, Error> {
        let detected = detect::detect().ok_or_else(|| {
            Error::DeviceNotFound("no supported ITE 048d:c9xx controller on the USB bus".into())
        })?;
        let api = HidApi::new().map_err(|e| Error::Io(format!("hidapi init: {e}")))?;
        let mut backend = Self {
            api,
            device: None,
            last_path: None,
            desc: detected.describe(),
        };
        // Select the hidraw node for the detected controller, then open it so
        // CLI/GUI can fail fast with a permission hint when access is blocked.
        backend.pick(detected.vid, detected.pid)?;
        backend.open_device()?;
        Ok(backend)
    }

    /// Open a backend for a specific (vid, pid) — used by tests/CLI.
    pub fn open_for(vid: u16, pid: u16) -> Result<Self, Error> {
        let api = HidApi::new().map_err(|e| Error::Io(format!("hidapi init: {e}")))?;
        let mut backend = Self {
            api,
            device: None,
            last_path: None,
            desc: format!("controller {:04x}:{:04x}", vid, pid),
        };
        backend.pick(vid, pid)?;
        backend.open_device()?;
        Ok(backend)
    }

    fn pick(&mut self, vid: u16, pid: u16) -> Result<(), Error> {
        let devices = self
            .api
            .device_list()
            .filter(|d| d.vendor_id() == vid && d.product_id() == pid)
            .collect::<Vec<_>>();
        if devices.is_empty() {
            return Err(Error::DeviceNotFound(format!(
                "{:04x}:{:04x} not found on the USB bus",
                vid, pid
            )));
        }
        // Prefer the documented vendor-usage interface; fall back to any.
        let chosen = devices
            .iter()
            .find(|d| d.usage_page() == USAGE_PAGE && d.usage() == USAGE)
            .or_else(|| devices.first())
            .expect("non-empty");
        self.last_path = Some(chosen.path().to_string_lossy().to_string());
        Ok(())
    }

    fn open_device(&mut self) -> Result<(), Error> {
        if self.device.is_some() {
            return Ok(());
        }
        let Some(path) = self.last_path.clone() else {
            return Err(Error::DeviceNotFound("no controller path selected".into()));
        };
        let cpath = CString::new(path.as_str())
            .map_err(|_| Error::Io("controller path contained a NUL byte".into()))?;
        match self.api.open_path(&cpath) {
            Ok(dev) => {
                self.device = Some(dev);
                Ok(())
            }
            Err(e) => Err(classify_open_error(&e)),
        }
    }

    /// Drop a stale handle so the next call re-opens (device unplug/replug).
    fn invalidate(&mut self) {
        self.device = None;
    }

    fn ensure_open(&mut self) -> Result<(), Error> {
        self.open_device()
    }

    fn with_device<T>(
        &mut self,
        f: impl FnOnce(&HidDevice) -> Result<T, HidError>,
    ) -> Result<T, Error> {
        self.ensure_open()?;
        let device = self.device.as_ref().expect("just opened");
        match f(device) {
            Ok(v) => Ok(v),
            Err(e) => {
                self.invalidate();
                Err(classify_io_error(&e))
            }
        }
    }
}

impl LightingBackend for HidBackend {
    fn apply(&mut self, cfg: LightingConfig) -> Result<(), Error> {
        let packet = packet::build_packet(&cfg);
        self.with_device(|d| d.send_feature_report(&packet))?;
        Ok(())
    }

    fn read_state(&mut self) -> Result<DeviceState, Error> {
        let mut buf = [0u8; PACKET_LEN];
        buf[0] = REPORT_ID;
        let n = self.with_device(|d| d.get_feature_report(&mut buf))?;
        if n < PACKET_LEN {
            // VERIFIED on LOQ 15IRX9 (048d:c993): the controller answers the
            // CC GET with an 11-byte identity report (it carries the PID), not
            // lighting state. Report that honestly instead of guessing.
            return Err(Error::Unsupported(format!(
                "this controller answers the CC report with {n} bytes of identity data, not \
                 lighting state — state readback is not available on this unit; the app tracks \
                 the last applied configuration instead"
            )));
        }
        packet::parse_state(&buf)
            .ok_or_else(|| Error::Protocol("feature report did not parse as CC/16 state".into()))
    }

    fn device_desc(&self) -> String {
        self.desc.clone()
    }
}

fn classify_open_error(e: &HidError) -> Error {
    let msg = e.to_string();
    if msg.contains("not permitted") || msg.contains("ermission") {
        Error::PermissionDenied(msg)
    } else {
        Error::Io(msg)
    }
}

fn classify_io_error(e: &HidError) -> Error {
    let msg = e.to_string();
    if msg.contains("not found") || msg.contains("o such device") {
        Error::DeviceNotFound(msg)
    } else {
        Error::Io(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Constructing a backend requires the hardware; this test documents the
    /// expected failure mode on machines without the controller and must not
    /// panic.
    #[test]
    fn open_reports_device_not_found_or_succeeds() {
        match HidBackend::open() {
            Ok(_) => eprintln!("hardware present in CI environment — test is informational"),
            Err(e) => assert!(matches!(
                e,
                Error::DeviceNotFound(_) | Error::PermissionDenied(_)
            )),
        }
    }
}
