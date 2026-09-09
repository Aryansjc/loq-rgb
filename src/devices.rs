//! Known Lenovo 4-zone RGB keyboard controllers and DMI helpers.
//!
//! The lighting controller is a USB HID device from ITE Tech (VID 0x048d).
//! The PID identifies the laptop generation/family; all entries below are
//! the "ITE Device(8295)" 4-zone controllers that speak the CC/16 protocol
//! implemented in [`crate::packet`]. PID list cross-checked against
//! LegionAura and legion-keyboard-custom.

use std::fs;

pub const ITE_VID: u16 = 0x048D;

/// (vid, pid, family label) — every entry is a verified 4-zone controller.
/// The `0xC9xx` range: 2020-2022 = 0xC5x, 0xC6x, 0xC7x; 2023 = 0xC8x;
/// 2024 = 0xC9x. The 2024 LOQ (this machine) is `0xC993`.
pub const KNOWN_CONTROLLERS: &[(u16, u16, &str)] = &[
    (ITE_VID, 0xC993, "Lenovo LOQ (2024)"),
    (ITE_VID, 0xC983, "Lenovo LOQ (2023)"),
    (ITE_VID, 0xC995, "Legion Pro (2024)"),
    (ITE_VID, 0xC994, "Legion (2024)"),
    (ITE_VID, 0xC985, "Legion Pro (2023)"),
    (ITE_VID, 0xC984, "Legion (2023)"),
    (ITE_VID, 0xC975, "Legion (2022)"),
    (ITE_VID, 0xC973, "IdeaPad Gaming (2022)"),
    (ITE_VID, 0xC965, "Legion (2021)"),
    (ITE_VID, 0xC963, "IdeaPad Gaming (2021)"),
    (ITE_VID, 0xC955, "Legion (2020)"),
];

/// Look up the family label for a (vid, pid).
pub fn controller_label(vid: u16, pid: u16) -> Option<&'static str> {
    KNOWN_CONTROLLERS
        .iter()
        .find(|(v, p, _)| *v == vid && *p == pid)
        .map(|(_, _, label)| *label)
}

/// The primary controller PID this project targets.
pub const PRIMARY_PID: u16 = 0xC993;

// ---------------------------------------------------------------------------
// DMI / sysfs helpers
// ---------------------------------------------------------------------------

const DMI_DIR: &str = "/sys/class/dmi/id";

/// Read a single DMI attribute, trimmed, or `None` when unavailable.
pub fn dmi_field(name: &str) -> Option<String> {
    let val = fs::read_to_string(format!("{DMI_DIR}/{name}")).ok()?;
    let val = val.trim().to_string();
    if val.is_empty() || val == "None" || val == "To be filled by O.E.M." {
        return None;
    }
    Some(val)
}

/// Friendly product name, e.g. "LOQ 15IRX9".
pub fn dmi_product_name() -> Option<String> {
    dmi_field("product_name")
}

/// Board/model number, e.g. "83DV".
pub fn dmi_board_name() -> Option<String> {
    dmi_field("board_name")
}

/// Vendor string, e.g. "LENOVO".
pub fn dmi_sys_vendor() -> Option<String> {
    dmi_field("sys_vendor")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c993_is_a_known_controller() {
        assert_eq!(controller_label(ITE_VID, 0xC993), Some("Lenovo LOQ (2024)"));
    }

    #[test]
    fn unknown_pid_is_none() {
        assert_eq!(controller_label(ITE_VID, 0x1234), None);
        assert_eq!(controller_label(0x1234, 0xC993), None);
    }

    #[test]
    fn no_duplicate_pids_in_table() {
        let mut pids: Vec<u16> = KNOWN_CONTROLLERS.iter().map(|(_, p, _)| *p).collect();
        let n = pids.len();
        pids.sort_unstable();
        pids.dedup();
        assert_eq!(pids.len(), n, "table contains duplicate PIDs");
    }
}
