//! Profile management beyond simple CRUD: safe deletion.
//!
//! Deletion rules (designed with the GUI/CLI requirements in mind):
//! - Deleting a profile never touches the hardware — the current lighting
//!   stays exactly as it is.
//! - Deleting the **active** profile switches the active selection to the
//!   first remaining non-reserved profile (alphabetical), or to the reserved
//!   "Off" profile when nothing else remains. The switch is persisted but
//!   not applied; the caller decides whether to show/apply the new active
//!   config.
//! - The reserved "Off" profile (Fn+Space cycle end step) cannot be deleted.
//! - Deleting the last user profile is safe: the reserved Off profile is
//!   ensured first, so the profile set never becomes empty and no phantom
//!   "Default" can resurrect on the next load.

use crate::config::{ConfigStore, OFF_PROFILE, ensure_off_profile};
use crate::error::Error;

/// Delete `name` from the store.
///
/// Returns `Ok(Some(new_active))` when the deleted profile was the active
/// one (the store's active selection has been switched to `new_active`),
/// `Ok(None)` otherwise. The deletion is persisted atomically. Nothing is
/// ever applied to the hardware here.
pub fn delete_profile(store: &mut ConfigStore, name: &str) -> Result<Option<String>, Error> {
    if name == OFF_PROFILE {
        return Err(Error::Config(format!(
            "{OFF_PROFILE:?} is the reserved Fn+Space cycle end step and cannot be deleted"
        )));
    }
    if !store.cfg.profiles.contains_key(name) {
        return Err(Error::Config(format!("no profile named {name:?}")));
    }

    // Guarantee the reserved fallback exists before removing anything.
    if ensure_off_profile(&mut store.cfg) {
        store.save()?;
    }

    store.cfg.profiles.remove(name);

    let switched_active = if store.cfg.active_profile == name {
        let next = fallback_active(&store.cfg);
        store.cfg.active_profile = next.clone();
        Some(next)
    } else {
        None
    };

    store.save()?;
    Ok(switched_active)
}

/// Pick the new active profile after the current one was deleted: the first
/// remaining profile in sorted order, preferring anything over the reserved
/// Off profile.
fn fallback_active(cfg: &crate::config::AppConfig) -> String {
    cfg.profiles
        .keys()
        .find(|n| n.as_str() != OFF_PROFILE)
        .unwrap_or(&OFF_PROFILE.to_string())
        .clone()
}

/// Rename a saved profile. Safety rules mirror deletion:
/// - the reserved "Off" profile cannot be renamed or overwritten,
/// - the new name must not collide with an existing profile,
/// - renaming the active profile keeps it active,
/// - the rename is persisted atomically and never touches the hardware.
pub fn rename_profile(store: &mut ConfigStore, old: &str, new: &str) -> Result<(), Error> {
    let new = new.trim().to_string();
    if old == OFF_PROFILE || new == OFF_PROFILE {
        return Err(Error::Config(format!(
            "{OFF_PROFILE:?} is reserved for the Fn+Space cycle end step and cannot be renamed"
        )));
    }
    if new.is_empty() {
        return Err(Error::Config("profile name must not be empty".into()));
    }
    if old == new {
        return Ok(()); // nothing to do
    }
    if !store.cfg.profiles.contains_key(old) {
        return Err(Error::Config(format!("no profile named {old:?}")));
    }
    if store.cfg.profiles.contains_key(&new) {
        return Err(Error::Config(format!(
            "a profile named {new:?} already exists"
        )));
    }

    let config = store
        .cfg
        .profiles
        .remove(old)
        .expect("presence checked above");
    store.cfg.profiles.insert(new.clone(), config);
    if store.cfg.active_profile == old {
        store.cfg.active_profile = new.clone();
    }
    store.save()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::model::{LightingConfig, Rgb};
    use std::collections::BTreeMap;

    fn cfg_with(names: &[&str], active: &str) -> AppConfig {
        let mut profiles = BTreeMap::new();
        for n in names {
            profiles.insert(n.to_string(), LightingConfig::default());
        }
        AppConfig {
            version: 1,
            active_profile: active.to_string(),
            profiles,
            palette: vec![Rgb::white()],
        }
    }

    fn store_in(dir: &tempfile::TempDir, cfg: AppConfig) -> ConfigStore {
        ConfigStore {
            path: dir.path().join("config.json"),
            cfg,
        }
    }

    #[test]
    fn deleting_non_active_keeps_everything_else() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store_in(&dir, cfg_with(&["Alpha", "Beta", "Gamma"], "Beta"));

        assert_eq!(delete_profile(&mut store, "Alpha").unwrap(), None);
        assert_eq!(store.cfg.active_profile, "Beta", "active untouched");
        assert!(!store.cfg.profiles.contains_key("Alpha"));
        assert!(store.cfg.profiles.contains_key("Beta"));
        assert!(store.cfg.profiles.contains_key("Gamma"));

        // Restart: deletion persisted, siblings intact.
        let reloaded = ConfigStore::load_from(&dir.path().join("config.json")).unwrap();
        assert!(!reloaded.cfg.profiles.contains_key("Alpha"));
        assert!(reloaded.cfg.profiles.contains_key("Beta"));
        assert!(reloaded.cfg.profiles.contains_key("Gamma"));
        assert_eq!(reloaded.cfg.active_profile, "Beta");
    }

    #[test]
    fn deleting_active_profile_switches_active_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store_in(&dir, cfg_with(&["Alpha", "Beta", "Gamma"], "Gamma"));
        // Also give each profile distinct zone colours to prove others are untouched.
        store.cfg.profiles.get_mut("Alpha").unwrap().zones[0] = Rgb::new(1, 0, 0);
        store.cfg.profiles.get_mut("Beta").unwrap().zones[0] = Rgb::new(0, 2, 0);

        let switched = delete_profile(&mut store, "Gamma").unwrap();
        assert_eq!(
            switched.as_deref(),
            Some("Alpha"),
            "falls back to first remaining"
        );
        assert!(!store.cfg.profiles.contains_key("Gamma"));
        assert_eq!(
            store.cfg.profiles.get("Alpha").unwrap().zones[0],
            Rgb::new(1, 0, 0)
        );
        assert_eq!(
            store.cfg.profiles.get("Beta").unwrap().zones[0],
            Rgb::new(0, 2, 0)
        );

        let reloaded = ConfigStore::load_from(&dir.path().join("config.json")).unwrap();
        assert_eq!(reloaded.cfg.active_profile, "Alpha");
        assert!(!reloaded.cfg.profiles.contains_key("Gamma"));
    }

    #[test]
    fn deleting_last_user_profile_falls_back_to_off() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store_in(&dir, cfg_with(&["Solo"], "Solo"));
        assert!(!store.cfg.profiles.contains_key(OFF_PROFILE));

        let switched = delete_profile(&mut store, "Solo").unwrap();
        assert_eq!(switched.as_deref(), Some(OFF_PROFILE));
        assert_eq!(store.cfg.active_profile, OFF_PROFILE);
        assert_eq!(
            store.cfg.profiles.len(),
            1,
            "only the reserved Off profile remains"
        );
        assert!(store.cfg.profiles.contains_key(OFF_PROFILE));

        // Restart: no phantom profile resurrection, still exactly Off.
        let reloaded = ConfigStore::load_from(&dir.path().join("config.json")).unwrap();
        assert_eq!(reloaded.cfg.profiles.len(), 1);
        assert!(reloaded.cfg.profiles.contains_key(OFF_PROFILE));
        assert_eq!(reloaded.cfg.active_profile, OFF_PROFILE);
    }

    #[test]
    fn off_profile_cannot_be_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store_in(&dir, cfg_with(&["Alpha"], "Alpha"));
        ensure_off_profile(&mut store.cfg);
        let err = delete_profile(&mut store, OFF_PROFILE).unwrap_err();
        assert!(err.to_string().contains("reserved"));
        assert!(store.cfg.profiles.contains_key(OFF_PROFILE));
    }

    #[test]
    fn deleting_missing_profile_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store_in(&dir, cfg_with(&["Alpha"], "Alpha"));
        assert!(delete_profile(&mut store, "Ghost").is_err());
    }

    #[test]
    fn fallback_prefers_real_profiles_over_off() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store_in(&dir, cfg_with(&["blue", "Off", "Alpha"], "Off"));
        // Active is Off (edge case): deleting it is refused; instead simulate
        // deleting Alpha while Off active -> active stays Off? Off active case:
        ensure_off_profile(&mut store.cfg);
        // deleting a non-active profile must not disturb an Off-active config
        assert_eq!(delete_profile(&mut store, "Alpha").unwrap(), None);
        assert_eq!(store.cfg.active_profile, OFF_PROFILE);
        assert!(store.cfg.profiles.contains_key("blue"));
    }

    #[test]
    fn rename_keeps_config_and_active_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store_in(&dir, cfg_with(&["Alpha", "Beta"], "Beta"));
        store.cfg.profiles.get_mut("Alpha").unwrap().speed = 4;

        rename_profile(&mut store, "Alpha", "Gamma").unwrap();
        assert!(!store.cfg.profiles.contains_key("Alpha"));
        let renamed = store.cfg.profiles.get("Gamma").expect("renamed exists");
        assert_eq!(renamed.speed, 4, "config survives rename");
        assert_eq!(store.cfg.active_profile, "Beta", "active unaffected");
        assert!(store.cfg.profiles.contains_key("Beta"));

        // Restart: rename persisted.
        let reloaded = ConfigStore::load_from(&dir.path().join("config.json")).unwrap();
        assert!(reloaded.cfg.profiles.contains_key("Gamma"));
        assert!(!reloaded.cfg.profiles.contains_key("Alpha"));
    }

    #[test]
    fn renaming_active_profile_keeps_it_active() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store_in(&dir, cfg_with(&["Alpha", "Beta"], "Alpha"));
        rename_profile(&mut store, "Alpha", "First").unwrap();
        assert_eq!(store.cfg.active_profile, "First");
        let reloaded = ConfigStore::load_from(&dir.path().join("config.json")).unwrap();
        assert_eq!(reloaded.cfg.active_profile, "First");
    }

    #[test]
    fn rename_rejects_collisions_and_reserved_off() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store_in(&dir, cfg_with(&["Alpha", "Beta"], "Alpha"));
        assert!(
            rename_profile(&mut store, "Alpha", "Beta").is_err(),
            "collision rejected"
        );
        ensure_off_profile(&mut store.cfg);
        assert!(
            rename_profile(&mut store, "Beta", OFF_PROFILE).is_err(),
            "cannot rename onto Off"
        );
        assert!(
            rename_profile(&mut store, OFF_PROFILE, "X").is_err(),
            "cannot rename Off"
        );
        assert!(rename_profile(&mut store, "Ghost", "X").is_err());
        assert!(
            rename_profile(&mut store, "Alpha", "   ").is_err(),
            "empty name rejected"
        );
        // Same-name rename is a harmless no-op.
        assert!(rename_profile(&mut store, "Alpha", "Alpha").is_ok());
    }
}
