//! Fn+Space profile cycling.
//!
//! Hardware findings (verified live on the LOQ 15IRX9, 048d:c993):
//! - Fn+Space emits EV_KEY code 240 (`KEY_KBDILLUMUP`) on the
//!   "Ideapad extra buttons" input node (VPC2004 ACPI device).
//! - While software control is active the firmware does NOT cycle natively,
//!   so a listener can cleanly cycle saved profiles on each press.
//!
//! Cycle order (per user request): saved profiles in alphabetical order,
//! then an **Off** state as the last step, then wrap back to the first
//! profile. `Off` is stored as a real profile (reserved name) so the active
//! selection always validates against the profile set. The reserved profile
//! helpers live in [`crate::config`] and are re-exported here for callers.

pub use crate::config::{OFF_PROFILE, ensure_off_profile};

use crate::config::AppConfig;
use crate::error::Error;

/// Key code produced by Fn+Space on this platform (kernel
/// `KEY_KBDILLUMUP`).
pub const FN_SPACE_CODE: u16 = 240;

/// Name of the input device that reports Fn+Space (Lenovo "Ideapad extra
/// buttons", driven by the VPC2004 ACPI device).
pub const IDEAPAD_BUTTONS_NAME: &str = "Ideapad extra buttons";

/// Ordered cycle: saved profiles alphabetically, then Off last.
pub fn cycle_order(cfg: &AppConfig) -> Vec<String> {
    let mut names: Vec<String> = cfg
        .profiles
        .keys()
        .filter(|n| n.as_str() != OFF_PROFILE)
        .cloned()
        .collect();
    names.sort();
    names.push(OFF_PROFILE.to_string());
    names
}

/// The profile that comes after `active` in the cycle (Off is last, then
/// wrap). Returns `None` when there are fewer than two steps.
pub fn next_profile_name(cfg: &AppConfig, active: &str) -> Option<String> {
    let order = cycle_order(cfg);
    if order.len() < 2 {
        return None;
    }
    let pos = order.iter().position(|n| n == active).unwrap_or(0);
    Some(order[(pos + 1) % order.len()].clone())
}

/// Advance the store to the next cycle step (persisting the selection) and
/// return the new active name. Applies nothing — the caller applies.
pub fn advance_profile(store: &mut crate::config::ConfigStore) -> Result<String, Error> {
    let changed = ensure_off_profile(&mut store.cfg);
    if changed {
        store.save()?;
    }
    let next = next_profile_name(&store.cfg, &store.cfg.active_profile).ok_or_else(|| {
        Error::Config("need at least one saved profile to cycle with Fn+Space".into())
    })?;
    store.set_active_profile(&next)?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::model::{Effect, LightingConfig, Rgb};
    use std::collections::BTreeMap;

    fn cfg_with(names: &[&str]) -> AppConfig {
        let mut profiles = BTreeMap::new();
        for n in names {
            profiles.insert(n.to_string(), LightingConfig::default());
        }
        let active = names.first().unwrap_or(&"Default").to_string();
        AppConfig {
            version: 1,
            active_profile: active,
            profiles,
            palette: vec![Rgb::white()],
        }
    }

    #[test]
    fn cycle_ends_with_off_and_wraps() {
        let mut cfg = cfg_with(&["Alpha", "Beta", "Gamma"]);
        ensure_off_profile(&mut cfg);
        assert_eq!(next_profile_name(&cfg, "Alpha").as_deref(), Some("Beta"));
        assert_eq!(next_profile_name(&cfg, "Gamma").as_deref(), Some("Off"));
        assert_eq!(
            next_profile_name(&cfg, "Off").as_deref(),
            Some("Alpha"),
            "wraps back to the first profile after Off"
        );
    }

    #[test]
    fn cycle_order_is_sorted_with_off_last() {
        let cfg = cfg_with(&["Gamma", "Alpha", "Off", "Beta"]);
        // "Off" is reserved and always the final step even if already present.
        let order = cycle_order(&cfg);
        assert_eq!(order, vec!["Alpha", "Beta", "Gamma", "Off"]);
    }

    #[test]
    fn off_profile_has_off_effect() {
        let mut cfg = cfg_with(&["Default"]);
        assert!(ensure_off_profile(&mut cfg));
        assert!(!ensure_off_profile(&mut cfg), "second call must be a no-op");
        let off = cfg.profiles.get(OFF_PROFILE).unwrap();
        assert_eq!(off.effect, Effect::Off);
        assert_eq!(
            off.brightness, 0,
            "off config forces brightness 0 on normalize"
        );
    }

    #[test]
    fn missing_active_profile_self_heals_to_first() {
        let mut cfg = cfg_with(&["Alpha", "Beta"]);
        ensure_off_profile(&mut cfg);
        assert_eq!(next_profile_name(&cfg, "Ghost").as_deref(), Some("Beta"));
    }

    #[test]
    fn only_off_profile_has_nowhere_to_cycle() {
        let mut cfg = cfg_with(&[]);
        ensure_off_profile(&mut cfg);
        assert_eq!(next_profile_name(&cfg, OFF_PROFILE), None);
    }

    #[test]
    fn advance_persists_selection_including_off() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = crate::config::ConfigStore {
            path: dir.path().join("config.json"),
            cfg: cfg_with(&["Alpha", "Beta"]),
        };
        assert_eq!(advance_profile(&mut store).unwrap(), "Beta");
        assert_eq!(advance_profile(&mut store).unwrap(), "Off");
        assert_eq!(store.cfg.active_profile, OFF_PROFILE);
        assert!(store.cfg.profiles.contains_key(OFF_PROFILE));
        // Persisted: reload sees the Off selection intact.
        let reloaded =
            crate::config::ConfigStore::load_from(&dir.path().join("config.json")).unwrap();
        assert_eq!(reloaded.cfg.active_profile, OFF_PROFILE);
        assert!(reloaded.cfg.profiles.contains_key(OFF_PROFILE));
    }
}
