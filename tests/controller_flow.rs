//! Integration tests: full apply flows through the controller against the
//! mock backend, focused on the guarantees the UI depends on.

use loq_rgb::backend::mock::MockBackend;
use loq_rgb::controller::Controller;
use loq_rgb::error::Error;
use loq_rgb::model::{DeviceState, Effect, LightingConfig, LiveEffect, Rgb};
use loq_rgb::packet::{PACKET_LEN, build_packet};

fn static_cfg(zones: [Rgb; 4]) -> LightingConfig {
    LightingConfig {
        effect: Effect::Static,
        speed: 1,
        brightness: 2,
        zones,
    }
}

/// THE zone-isolation guarantee: changing one zone must rebuild the packet
/// from the whole current config so the other three zones keep their exact
/// colours. This is the "changing one zone must not overwrite the others"
/// requirement, verified at the byte level.
#[test]
fn changing_one_zone_never_touches_the_others() {
    let backend = MockBackend::new();
    let mut controller = Controller::new(Box::new(backend));

    let red = Rgb::new(255, 0, 0);
    let green = Rgb::new(0, 255, 0);
    let blue = Rgb::new(0, 0, 255);
    let white = Rgb::new(255, 255, 255);

    controller
        .apply(static_cfg([red, green, blue, white]))
        .unwrap();

    // User edits zone 3 only → new config keeps zones 1, 2, 4 untouched.
    let edited = LightingConfig {
        zones: [red, green, Rgb::new(12, 34, 56), white],
        ..controller.last_config().copied().unwrap()
    };
    controller.apply(edited).unwrap();

    let applied = controller.last_config().unwrap();
    assert_eq!(applied.zones[0], red);
    assert_eq!(applied.zones[1], green);
    assert_eq!(applied.zones[2], Rgb::new(12, 34, 56));
    assert_eq!(applied.zones[3], white);

    // Same guarantee at packet level.
    let packet = build_packet(applied);
    let z = |i: usize| {
        let o = 5 + i * 3;
        [packet[o], packet[o + 1], packet[o + 2]]
    };
    assert_eq!(z(0), [255, 0, 0]);
    assert_eq!(z(1), [0, 255, 0]);
    assert_eq!(z(2), [12, 34, 56]);
    assert_eq!(z(3), [255, 255, 255]);
}

/// Each effect applied to every zone individually (all four zone slots) must
/// produce a valid packet and record correctly.
#[test]
fn every_effect_on_every_zone_slot() {
    let backend = MockBackend::new();
    let mut controller = Controller::new(Box::new(backend));
    let colours = [
        Rgb::new(255, 0, 0),
        Rgb::new(0, 255, 0),
        Rgb::new(0, 0, 255),
        Rgb::new(200, 100, 50),
    ];
    for effect in [
        Effect::Off,
        Effect::Static,
        Effect::Breath,
        Effect::WaveLeft,
        Effect::WaveRight,
        Effect::Smooth,
    ] {
        for zone in 0..4 {
            let mut zones = [Rgb::black(); 4];
            zones[zone] = colours[zone];
            let cfg = LightingConfig {
                effect,
                speed: 1 + zone as u8,
                brightness: if zone % 2 == 0 { 1 } else { 2 },
                zones,
            };
            controller.apply(cfg).unwrap();
            let packet = build_packet(controller.last_config().unwrap());
            assert_eq!(packet.len(), PACKET_LEN);
            assert_eq!(packet[0], 0xCC);
        }
    }
}

/// Mixed per-zone effects must not be expressible: the policy denies colour
/// editing for non-colour effects and the packet carries a single effect
/// byte. This test pins the honest behaviour (no fake "zone effects").
#[test]
fn single_global_effect_byte_is_enforced() {
    use loq_rgb::effects::{info, zone_colors_editable};
    for effect in [
        Effect::Off,
        Effect::Static,
        Effect::Breath,
        Effect::WaveLeft,
        Effect::WaveRight,
        Effect::Smooth,
    ] {
        let cfg = LightingConfig {
            effect,
            ..LightingConfig::default()
        };
        let p = build_packet(&cfg);
        assert_eq!(p[2], effect.code());
        // Only colour effects expose zone colour editing.
        assert_eq!(zone_colors_editable(effect), effect.uses_zone_colors());
        // UI label exists.
        assert!(!info(effect).label.is_empty());
    }
}

/// Failed applies surface errors and do not update last_config.
#[test]
fn failed_apply_keeps_previous_good_state() {
    let mut controller = Controller::new(Box::new(MockBackend::new()));
    controller.apply(static_cfg([Rgb::white(); 4])).unwrap();
    let mock = controller
        .backend_mut()
        .as_any_mut()
        .and_then(|b| b.downcast_mut::<MockBackend>())
        .expect("mock backend");
    mock.fail_apply = Some(Error::Io("unplugged".into()));
    let err = controller
        .apply(static_cfg([Rgb::new(1, 2, 3); 4]))
        .unwrap_err();
    assert!(matches!(err, Error::Io(_)));
    assert_eq!(
        controller.last_config().unwrap().zones[0],
        Rgb::white(),
        "last good state must survive a failed apply"
    );
}

/// Read-back path: mock state flows through the controller and the GUI can
/// display "what the hardware currently reports".
#[test]
fn read_state_flows_through_controller() {
    let mut backend = MockBackend::new();
    let state = DeviceState {
        effect: LiveEffect::Known(Effect::Breath),
        speed: 2,
        brightness: 1,
        zones: [Rgb::new(10, 20, 30); 4],
        flag_right: false,
        flag_left: false,
    };
    backend.state = Some(state);
    let mut controller = Controller::new(Box::new(backend));
    let read = controller.read_state().unwrap();
    assert_eq!(read, state);
}
