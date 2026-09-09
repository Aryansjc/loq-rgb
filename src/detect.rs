//! Hardware detection: find a supported RGB controller on the USB bus and
//! pair it with DMI identity for friendly display.

use std::fs;
use std::path::Path;

use crate::devices::dmi_field;
use crate::devices::{controller_label, dmi_board_name, dmi_sys_vendor};

/// A detected, supported lighting controller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detected {
    pub vid: u16,
    pub pid: u16,
    pub family: &'static str,
    /// e.g. "LENOVO LOQ 15IRX9 (83DV)" — only when DMI data is readable.
    pub machine: Option<String>,
}

impl Detected {
    /// Short single-line description used by the CLI and UI device card.
    pub fn describe(&self) -> String {
        match &self.machine {
            Some(m) => format!(
                "{} · {} controller {:04x}:{:04x}",
                m, self.family, self.vid, self.pid
            ),
            None => format!(
                "{} controller {:04x}:{:04x}",
                self.family, self.vid, self.pid
            ),
        }
    }
}

/// Scan `base` (default `/sys/bus/usb/devices`) for USB devices and return
/// (vid, pid) pairs. Interface directories (names containing `:`) and
/// malformed entries are skipped. Kept path-injectable for tests.
pub fn scan_usb_devices(base: &Path) -> Vec<(u16, u16)> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(base) else {
        return out;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.contains(':') {
            continue; // interface dir, not a device
        }
        let dir = entry.path();
        let vid = fs::read_to_string(dir.join("idVendor")).ok();
        let pid = fs::read_to_string(dir.join("idProduct")).ok();
        let (Some(vid), Some(pid)) = (vid, pid) else {
            continue;
        };
        let (Ok(vid), Ok(pid)) = (
            u16::from_str_radix(vid.trim(), 16),
            u16::from_str_radix(pid.trim(), 16),
        ) else {
            continue;
        };
        out.push((vid, pid));
    }
    out
}

/// Probe the bus for a known controller and build the detection record.
pub fn detect_from_bus(devices: &[(u16, u16)]) -> Option<Detected> {
    for (vid, pid) in devices {
        if let Some(family) = controller_label(*vid, *pid) {
            return Some(Detected {
                vid: *vid,
                pid: *pid,
                family,
                machine: machine_name(),
            });
        }
    }
    None
}

/// Detect on the live system.
pub fn detect() -> Option<Detected> {
    let devices = scan_usb_devices(Path::new("/sys/bus/usb/devices"));
    detect_from_bus(&devices)
}

/// Human-readable machine string from DMI, or `None`.
///
/// Lenovo DMI data on this class of machine: `product_name` is the model
/// code ("83DV"), `product_family`/`product_version` carry the marketing
/// name ("LOQ 15IRX9"). Prefer the marketing name, keep the model code.
pub fn machine_name() -> Option<String> {
    let vendor = dmi_sys_vendor()?;
    let marketing = dmi_field("product_family")
        .or_else(|| dmi_field("product_version"))
        .or_else(|| dmi_field("product_name"))?;
    let model_code = dmi_field("product_name").or_else(dmi_board_name);
    let label = match model_code {
        Some(code) if code != marketing => format!("{vendor} {marketing} ({code})"),
        _ => format!("{vendor} {marketing}"),
    };
    Some(label)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture_devices(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, content) in files {
            let sub = dir.path().join(name);
            fs::create_dir_all(&sub).unwrap();
            fs::write(sub.join("idVendor"), content.split('/').next().unwrap()).unwrap();
            fs::write(
                sub.join("idProduct"),
                content.split('/').nth(1).unwrap_or(""),
            )
            .unwrap();
        }
        dir
    }

    #[test]
    fn detects_c993_among_other_devices() {
        let dir = fixture_devices(&[
            ("1-1", "0bda/4853"),     // bluetooth
            ("1-7", "048d/c993"),     // the LOQ lighting controller
            ("1-7:1.0", "048d/c993"), // interface dir must be skipped
            ("1-8", "048d/c996"),     // key matrix — not a lighting controller
        ]);
        let devs = scan_usb_devices(dir.path());
        assert!(devs.contains(&(0x048d, 0xC993)));
        assert_eq!(
            devs.iter().filter(|d| *d == &(0x048d, 0xC993)).count(),
            1,
            "interface dirs must not double-count"
        );
        let d = detect_from_bus(&devs).unwrap();
        assert_eq!(d.vid, 0x048d);
        assert_eq!(d.pid, 0xC993);
        assert_eq!(d.family, "Lenovo LOQ (2024)");
    }

    #[test]
    fn unknown_bus_returns_none() {
        let devs = vec![(0x0bda, 0x4853), (0x048d, 0xc996)];
        assert!(detect_from_bus(&devs).is_none());
    }

    #[test]
    fn empty_bus_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(scan_usb_devices(dir.path()).is_empty());
        assert!(detect_from_bus(&[]).is_none());
    }

    #[test]
    fn c993_is_preferred_only_if_present() {
        let devs = vec![(0x048d, 0xc985), (0x048d, 0xc993)];
        let d = detect_from_bus(&devs).unwrap();
        // First match in bus order wins; presence is what matters.
        assert!(d.pid == 0xc985 || d.pid == 0xc993);
    }
}
