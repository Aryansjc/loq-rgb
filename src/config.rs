//! Configuration persistence: profiles, the active profile and the saved
//! colour palette, stored as JSON under the XDG config directory.
//!
//! Guarantees:
//! - atomic writes (temp file + rename), so a crash never truncates config,
//! - corrupt files are backed up (never silently deleted) and defaults are
//!   restored,
//! - every value is validated on load; out-of-range values are clamped.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::model::{Effect, LightingConfig, Rgb};

pub const CONFIG_VERSION: u32 = 1;
const APP_DIR: &str = "loq-rgb";
const CONFIG_FILE: &str = "config.json";

/// Reserved profile name: the final step of the Fn+Space cycle (backlight
/// off). Auto-managed: it is added on load/use and cannot be deleted.
pub const OFF_PROFILE: &str = "Off";

/// Add the reserved Off profile to a config if it is missing.
/// Returns whether the config changed.
pub fn ensure_off_profile(cfg: &mut AppConfig) -> bool {
    if cfg.profiles.contains_key(OFF_PROFILE) {
        return false;
    }
    cfg.profiles.insert(
        OFF_PROFILE.to_string(),
        LightingConfig {
            effect: Effect::Off,
            speed: 1,
            brightness: 1,
            ..LightingConfig::default()
        }
        .normalized(),
    );
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppConfig {
    pub version: u32,
    pub active_profile: String,
    pub profiles: BTreeMap<String, LightingConfig>,
    /// Custom colours saved by the user for quick reuse.
    pub palette: Vec<Rgb>,
}

impl Default for AppConfig {
    fn default() -> Self {
        let mut profiles = BTreeMap::new();
        profiles.insert("Default".to_string(), LightingConfig::default());
        Self {
            version: CONFIG_VERSION,
            active_profile: "Default".to_string(),
            profiles,
            palette: Vec::new(),
        }
    }
}

impl AppConfig {
    /// Validate and repair on load: version check, active profile must
    /// exist, values clamped.
    pub fn repaired(mut self) -> Result<Self, Error> {
        if self.version != CONFIG_VERSION {
            return Err(Error::Config(format!(
                "unsupported config version {} (this build reads {CONFIG_VERSION})",
                self.version
            )));
        }
        for (name, cfg) in self.profiles.iter_mut() {
            if name.trim().is_empty() {
                return Err(Error::Config("profile name must not be empty".into()));
            }
            *cfg = cfg.normalized();
        }
        if self.profiles.is_empty() {
            self.profiles
                .insert("Default".into(), LightingConfig::default());
        }
        if !self.profiles.contains_key(&self.active_profile) {
            self.active_profile = self.profiles.keys().next().expect("non-empty").clone();
        }
        self.palette.truncate(64);
        Ok(self)
    }

    pub fn active_config(&self) -> LightingConfig {
        self.profiles
            .get(&self.active_profile)
            .copied()
            .unwrap_or_default()
            .normalized()
    }
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

/// XDG config dir for this app (`$XDG_CONFIG_HOME/loq-rgb` or
/// `~/.config/loq-rgb`).
pub fn config_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME")
        && !xdg.is_empty()
    {
        return PathBuf::from(xdg).join(APP_DIR);
    }
    if let Ok(home) = std::env::var("HOME")
        && !home.is_empty()
    {
        return PathBuf::from(home).join(".config").join(APP_DIR);
    }
    PathBuf::from(APP_DIR)
}

pub fn config_path() -> PathBuf {
    config_dir().join(CONFIG_FILE)
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

/// Loaded config plus its path, with save helpers.
#[derive(Debug, Clone)]
pub struct ConfigStore {
    pub path: PathBuf,
    pub cfg: AppConfig,
}

impl ConfigStore {
    /// Load from the default location. Missing file → defaults. Corrupt
    /// file → renamed to `config.json.corrupt-<ts>` and defaults returned.
    pub fn load() -> Result<Self, Error> {
        Self::load_from(&config_path())
    }

    pub fn load_from(path: &Path) -> Result<Self, Error> {
        let raw = match fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let store = Self {
                    path: path.to_path_buf(),
                    cfg: AppConfig::default(),
                };
                store.save()?;
                return Ok(store);
            }
            Err(e) => {
                return Err(Error::Config(format!(
                    "cannot read {}: {e}",
                    path.display()
                )));
            }
        };
        match serde_json::from_str::<AppConfig>(&raw) {
            Ok(cfg) => {
                let cfg = cfg.repaired()?;
                Ok(Self {
                    path: path.to_path_buf(),
                    cfg,
                })
            }
            Err(e) => {
                // Back up the corrupt file and start fresh — never destroy data.
                let backup = path.with_extension(format!("json.corrupt-{}", timestamp()));
                let _ = fs::rename(path, &backup);
                let store = Self {
                    path: path.to_path_buf(),
                    cfg: AppConfig::default(),
                };
                store.save()?;
                Err(Error::Config(format!(
                    "{} was invalid ({e}); backed up to {} and recreated with defaults",
                    path.display(),
                    backup.display()
                )))
            }
        }
    }

    /// Atomic save: write a temp file in the same directory, fsync, rename.
    pub fn save(&self) -> Result<(), Error> {
        write_atomic(&self.path, &self.cfg)
    }

    pub fn set_active_profile(&mut self, name: &str) -> Result<(), Error> {
        if !self.cfg.profiles.contains_key(name) {
            return Err(Error::Config(format!("no profile named {name:?}")));
        }
        self.cfg.active_profile = name.to_string();
        self.save()
    }

    pub fn upsert_profile(&mut self, name: &str, cfg: LightingConfig) -> Result<(), Error> {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(Error::Config("profile name must not be empty".into()));
        }
        if name == OFF_PROFILE && cfg.effect != Effect::Off {
            return Err(Error::Config(format!(
                "{OFF_PROFILE:?} is a reserved name for the backlight-off state"
            )));
        }
        self.cfg.profiles.insert(name, cfg.normalized());
        self.save()
    }

    pub fn add_palette_color(&mut self, color: Rgb) -> Result<(), Error> {
        if !self.cfg.palette.contains(&color) {
            self.cfg.palette.push(color);
            self.cfg.palette.truncate(64);
            self.save()?;
        }
        Ok(())
    }

    pub fn remove_palette_color(&mut self, color: Rgb) -> Result<(), Error> {
        let before = self.cfg.palette.len();
        self.cfg.palette.retain(|c| *c != color);
        if self.cfg.palette.len() != before {
            self.save()?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

fn timestamp() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "0".into())
}

fn write_atomic(path: &Path, cfg: &AppConfig) -> Result<(), Error> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .map_err(|e| Error::Config(format!("cannot create {}: {e}", parent.display())))?;
    let mut json = serde_json::to_string_pretty(cfg)
        .map_err(|e| Error::Config(format!("cannot serialise config: {e}")))?;
    json.push('\n');

    let tmp = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("config"),
        std::process::id()
    ));
    let result = (|| -> std::io::Result<()> {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(json.as_bytes())?;
        f.sync_all()?;
        fs::rename(&tmp, path)?;
        Ok(())
    })();
    match result {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(Error::Config(format!(
                "cannot write {}: {e}",
                path.display()
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Effect, Rgb};

    fn roundtrip_store() -> (tempfile::TempDir, ConfigStore) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        (
            dir,
            ConfigStore {
                path: path.clone(),
                cfg: AppConfig::default(),
            },
        )
    }

    #[test]
    fn save_and_load_round_trip() {
        let (dir, store) = roundtrip_store();
        let path = store.path.clone();
        let mut store = store;
        store
            .upsert_profile(
                "gaming",
                LightingConfig {
                    effect: Effect::Breath,
                    speed: 3,
                    brightness: 1,
                    zones: [
                        Rgb::new(255, 0, 0),
                        Rgb::new(0, 255, 0),
                        Rgb::new(0, 0, 255),
                        Rgb::new(255, 255, 0),
                    ],
                },
            )
            .unwrap();
        store.set_active_profile("gaming").unwrap();
        store.add_palette_color(Rgb::new(1, 2, 3)).unwrap();

        let loaded = ConfigStore::load_from(&path).unwrap();
        assert_eq!(loaded.cfg, store.cfg);
        assert_eq!(loaded.cfg.active_profile, "gaming");
        let active = loaded.cfg.active_config();
        assert_eq!(active.effect, Effect::Breath);
        assert_eq!(active.zones[0], Rgb::new(255, 0, 0));
        assert_eq!(loaded.cfg.palette, vec![Rgb::new(1, 2, 3)]);
        drop(dir);
    }

    #[test]
    fn missing_file_creates_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::load_from(&dir.path().join("config.json")).unwrap();
        assert_eq!(store.cfg.profiles.len(), 1);
        assert!(store.cfg.profiles.contains_key("Default"));
        assert!(
            store.path.exists(),
            "defaults should be persisted immediately"
        );
    }

    #[test]
    fn corrupt_file_is_backed_up_and_defaults_returned() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, "{ not json !!!").unwrap();
        let res = ConfigStore::load_from(&path);
        let err = res.expect_err("corrupt config must be reported, not silent");
        assert!(
            err.to_string().contains("backed up"),
            "error should mention the backup: {err}"
        );
        let entries = fs::read_dir(dir.path()).unwrap().count();
        assert!(entries >= 2, "original must be backed up: {entries}");
        // A fresh load now succeeds with defaults.
        assert!(ConfigStore::load_from(&path).is_ok());
    }

    #[test]
    fn invalid_values_are_clamped_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut cfg = AppConfig::default();
        cfg.profiles.insert(
            "wild".into(),
            LightingConfig {
                effect: Effect::WaveLeft,
                speed: 99,
                brightness: 99,
                ..LightingConfig::default()
            },
        );
        cfg.active_profile = "wild".into();
        fs::write(&path, serde_json::to_string_pretty(&cfg).unwrap()).unwrap();

        let store = ConfigStore::load_from(&path).unwrap();
        let active = store.cfg.active_config();
        assert_eq!(active.speed, crate::model::SPEED_MAX);
        assert_eq!(active.brightness, crate::model::BRIGHTNESS_MAX);
    }

    #[test]
    fn active_profile_must_exist() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ConfigStore {
            path: dir.path().join("config.json"),
            cfg: AppConfig::default(),
        };
        assert!(store.set_active_profile("nope").is_err());
    }

    #[test]
    fn off_name_is_reserved_for_off_configs() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ConfigStore {
            path: dir.path().join("config.json"),
            cfg: AppConfig::default(),
        };
        // Saving a real (lit) config under the reserved name is refused…
        assert!(
            store
                .upsert_profile(super::OFF_PROFILE, LightingConfig::default())
                .is_err()
        );
        // …but the reserved Off state itself is allowed (it already exists).
        let off_cfg = LightingConfig {
            effect: Effect::Off,
            ..LightingConfig::default()
        };
        assert!(store.upsert_profile(super::OFF_PROFILE, off_cfg).is_ok());
    }

    #[test]
    fn profile_operations() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ConfigStore {
            path: dir.path().join("config.json"),
            cfg: AppConfig::default(),
        };
        store
            .upsert_profile("b", LightingConfig::default())
            .unwrap();
        store.set_active_profile("b").unwrap();
        store
            .upsert_profile("a", LightingConfig::default())
            .unwrap();
        // Deleting a non-active profile keeps the active one.
        crate::profiles::delete_profile(&mut store, "a").unwrap();
        assert!(!store.cfg.profiles.contains_key("a"));
        assert_eq!(store.cfg.active_profile, "b");
        assert!(store.set_active_profile("a").is_err());
    }
}
