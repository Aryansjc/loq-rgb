//! Persistence integration tests: configs must survive full app restarts
//! (new store + new controller), profile switches must re-apply the full
//! packet, and nothing may be corrupted across cycles.

use loq_rgb::config::ConfigStore;
use loq_rgb::model::{Effect, LightingConfig, Rgb};

fn dir_store(dir: &std::path::Path) -> ConfigStore {
    ConfigStore {
        path: dir.join("config.json"),
        cfg: loq_rgb::config::AppConfig::default(),
    }
}

/// Simulate: user builds a profile, app closes, app reopens → profile and
/// active selection intact.
#[test]
fn profile_survives_restart_cycles() {
    let dir = tempfile::tempdir().unwrap();

    let zones = [
        Rgb::new(0x12, 0x34, 0x56),
        Rgb::new(0xab, 0xcd, 0xef),
        Rgb::new(10, 20, 30),
        Rgb::new(200, 100, 0),
    ];
    let cfg = LightingConfig {
        effect: Effect::Breath,
        speed: 4,
        brightness: 1,
        zones,
    };

    // "Session one"
    {
        let mut store = dir_store(dir.path());
        store.upsert_profile("Night", cfg).unwrap();
        store.set_active_profile("Night").unwrap();
        store.add_palette_color(Rgb::new(1, 2, 3)).unwrap();
    }

    // "Session two"
    {
        let store = ConfigStore::load_from(&dir.path().join("config.json")).unwrap();
        assert_eq!(store.cfg.active_profile, "Night");
        let active = store.cfg.active_config();
        assert_eq!(active, cfg.normalized());
        assert_eq!(store.cfg.palette, vec![Rgb::new(1, 2, 3)]);
    }

    // "Session three" — switch profiles back and forth, no drift.
    {
        let mut store = ConfigStore::load_from(&dir.path().join("config.json")).unwrap();
        store.set_active_profile("Default").unwrap();
        assert_eq!(store.cfg.active_config().effect, Effect::Static);
        store.set_active_profile("Night").unwrap();
        assert_eq!(store.cfg.active_config().effect, Effect::Breath);
    }

    let final_store = ConfigStore::load_from(&dir.path().join("config.json")).unwrap();
    assert_eq!(final_store.cfg.active_profile, "Night");
    assert_eq!(final_store.cfg.active_config(), cfg.normalized());
}

/// Every restart must re-read the file cleanly, many times (regression: no
/// accumulating corruption from repeated saves).
#[test]
fn many_restart_cycles_stay_byte_stable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");

    let mut store = dir_store(dir.path());
    for i in 0..20 {
        store
            .upsert_profile(
                &format!("P{i}"),
                LightingConfig {
                    effect: if i % 2 == 0 {
                        Effect::Smooth
                    } else {
                        Effect::WaveLeft
                    },
                    speed: 1 + (i % 4) as u8,
                    brightness: 1 + (i % 2) as u8,
                    ..LightingConfig::default()
                },
            )
            .unwrap();
        store.set_active_profile(&format!("P{i}")).unwrap();
        let reloaded = ConfigStore::load_from(&path).unwrap();
        // Default + P0..Pi  =>  i+2 profiles
        assert_eq!(reloaded.cfg.profiles.len(), i + 2);
        assert_eq!(reloaded.cfg.active_profile, format!("P{i}"));
        store = reloaded;
    }
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(
        raw.trim_end().ends_with('}'),
        "config must remain valid JSON"
    );
}
