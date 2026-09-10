//! Interactive hardware validation.
//!
//! These tests change the real keyboard backlight and ask YOU to confirm what
//! you see. They are `#[ignore]`d by default because visual confirmation is
//! the only proof a keyboard actually changed — an API call returning Ok is
//! not evidence.
//!
//! Run with:
//!     cargo test --test hardware -- --ignored --nocapture
//!
//! Requirements: the udev rule installed (`sudo loq-rgb-cli udev` +
//! reload/trigger) or run as root, and you watching the keyboard.
//!
//! Any answer that contradicts the app's assumption FAILS the test — that is
//! intentional. A failure means a code assumption (usually a wave-direction
//! or brightness label) must be corrected before the feature is trusted.

use hidapi::HidApi;
use loq_rgb::model::{Effect, LightingConfig, Rgb};
use loq_rgb::packet::{self, PACKET_LEN};

const VID: u16 = 0x048d;
const PID: u16 = 0xc993;

struct Device {
    dev: hidapi::HidDevice,
}

impl Device {
    fn open() -> Result<Self, String> {
        let api = HidApi::new().map_err(|e| e.to_string())?;
        let info = api
            .device_list()
            .find(|d| d.vendor_id() == VID && d.product_id() == PID)
            .ok_or_else(|| {
                format!("controller {VID:04x}:{PID:04x} not found — is this the right laptop?")
            })?;
        let dev = info.open_device(&api).map_err(|e| {
            format!(
                "{e}\nhint: install the udev rule (sudo loq-rgb-cli udev) and reload/trigger udev, \
                 or run the tests as root"
            )
        })?;
        Ok(Self { dev })
    }

    fn send(&self, cfg: &LightingConfig) -> Result<(), String> {
        let packet = packet::build_packet(cfg);
        self.dev
            .send_feature_report(&packet)
            .map_err(|e| e.to_string())
    }

    /// Send a raw 33-byte packet (for probing undocumented effect codes).
    fn send_raw(&self, p: &[u8; PACKET_LEN]) -> Result<(), String> {
        self.dev.send_feature_report(p).map_err(|e| e.to_string())
    }

    fn read(&self) -> Result<loq_rgb::model::DeviceState, String> {
        let mut buf = [0u8; PACKET_LEN];
        buf[0] = packet::REPORT_ID;
        let n = self
            .dev
            .get_feature_report(&mut buf)
            .map_err(|e| e.to_string())?;
        if n < PACKET_LEN {
            return Err(format!("short read: {n} bytes"));
        }
        packet::parse_state(&buf).ok_or_else(|| "unparseable state".to_string())
    }
}

fn ask(prompt: &str) -> bool {
    loop {
        eprintln!("{prompt}  [y/n] ");
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).unwrap();
        match line.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" => return true,
            "n" | "no" => return false,
            _ => eprintln!("(answer y or n)"),
        }
    }
}

fn pause(prompt: &str) {
    eprintln!("{prompt}  [press Enter] ");
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
}

fn zone_cfg(zone: usize, colour: Rgb) -> LightingConfig {
    let mut zones = [Rgb::black(); 4];
    zones[zone] = colour;
    LightingConfig {
        effect: Effect::Static,
        speed: 1,
        brightness: 2,
        zones,
    }
}

/// Baseline: put the keyboard into a known, comfortable state at the end of
/// every test so a failure never leaves the user with a weird backlight.
fn restore_white(d: &Device) {
    let _ = d.send(&LightingConfig {
        effect: Effect::Static,
        speed: 2,
        brightness: 2,
        zones: [Rgb::white(); 4],
    });
}

fn open_or_abort() -> Device {
    match Device::open() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("cannot open controller: {e}");
            std::process::exit(2);
        }
    }
}

/// Each of the four zones lights with a distinct colour; confirms physical
/// zone order left→right and that colours are independently controllable.
#[test]
#[ignore]
fn zones_light_independently_left_to_right() {
    let d = open_or_abort();
    let colours = [
        Rgb::new(255, 0, 0),
        Rgb::new(0, 255, 0),
        Rgb::new(0, 0, 255),
        Rgb::new(255, 255, 255),
    ];
    for (zone, colour) in colours.iter().enumerate() {
        d.send(&zone_cfg(zone, *colour)).unwrap();
        pause(&format!(
            "Zone {} should be the ONLY lit zone, colour #{}",
            zone + 1,
            colour.to_hex()
        ));
        restore_white(&d);
    }
    // Different colours on different zones simultaneously.
    d.send(&LightingConfig {
        effect: Effect::Static,
        speed: 1,
        brightness: 2,
        zones: colours,
    })
    .unwrap();
    assert!(
        ask("Do you see four DIFFERENT colours: red, green, blue, white (left → right)?"),
        "simultaneous per-zone colours did not match expectations"
    );
    restore_white(&d);
}

/// Every documented effect must visibly do what the app claims.
#[test]
#[ignore]
fn effects_are_visibly_correct() {
    let d = open_or_abort();

    d.send(&zone_cfg(0, Rgb::new(255, 0, 0))).unwrap();
    assert!(
        ask("Static: is ONLY zone 1 lit solid red?"),
        "static per-zone colour failed"
    );

    d.send(&LightingConfig {
        effect: Effect::Breath,
        speed: 2,
        brightness: 2,
        zones: [
            Rgb::new(255, 0, 0),
            Rgb::new(0, 255, 0),
            Rgb::new(0, 0, 255),
            Rgb::new(255, 255, 255),
        ],
    })
    .unwrap();
    pause("Breathing: all four zone colours should pulse in sync — look at it, then press Enter");
    assert!(
        ask("Did all zones pulse together in sync?"),
        "breath did not behave as expected"
    );

    d.send(&LightingConfig {
        effect: Effect::Smooth,
        speed: 2,
        brightness: 2,
        ..LightingConfig::default()
    })
    .unwrap();
    assert!(
        ask("Smooth flow: does a continuous smooth colour flow animate across the keyboard?"),
        "smooth effect did not behave as expected"
    );

    restore_white(&d);
}

/// Verify the colour wave's direction is what the UI claims. The wave is
/// host-rendered, so the app itself must be writing frames during this test
/// (run it with the GUI open, or with `listen-hotkeys` running and a
/// colour-wave profile active).
#[test]
#[ignore]
fn colour_wave_directions_match_labels() {
    let d = open_or_abort();

    d.send(&LightingConfig {
        effect: Effect::FlowLeft,
        speed: 2,
        brightness: 2,
        ..LightingConfig::default()
    })
    .unwrap();
    pause(
        "Start the animator now (GUI open, or `loq-rgb-cli listen-hotkeys` with a \
         colour-wave profile active) and watch the movement.",
    );
    let left_ok = ask("Does the colour wave travel LEFT (towards Esc)?");

    d.send(&LightingConfig {
        effect: Effect::FlowRight,
        speed: 2,
        brightness: 2,
        ..LightingConfig::default()
    })
    .unwrap();
    let right_ok = ask("Does the colour wave travel RIGHT (towards the arrow keys)?");

    restore_white(&d);
    assert!(left_ok, "colour wave ← travelled the wrong way");
    assert!(right_ok, "colour wave → travelled the wrong way");
}

/// Brightness byte semantics: level 2 should be visibly brighter than 1.
/// If the firmware ignores the byte, this fails loudly instead of shipping a
/// fake brightness slider.
#[test]
#[ignore]
fn brightness_byte_is_honoured() {
    let d = open_or_abort();
    let cfg = |brightness: u8| LightingConfig {
        effect: Effect::Static,
        speed: 1,
        brightness,
        zones: [Rgb::white(); 4],
    };
    d.send(&cfg(1)).unwrap();
    pause("Brightness LOW — look at the keyboard, then press Enter.");
    d.send(&cfg(2)).unwrap();
    assert!(
        ask("Brightness HIGH — is the keyboard clearly brighter than the previous step?"),
        "brightness byte appears ignored by the firmware — represent this honestly in the UI"
    );
    restore_white(&d);
}

/// The off encoding: effect byte 0 + brightness 0 must extinguish the
/// backlight completely.
#[test]
#[ignore]
fn off_encoding_turns_everything_off() {
    let d = open_or_abort();
    d.send(&LightingConfig {
        effect: Effect::Static,
        speed: 1,
        brightness: 2,
        zones: [Rgb::white(); 4],
    })
    .unwrap();
    pause("Keyboard should be lit white now.");
    d.send(&LightingConfig {
        effect: Effect::Off,
        speed: 1,
        brightness: 0,
        ..LightingConfig::default()
    })
    .unwrap();
    assert!(
        ask("Is the backlight now completely OFF?"),
        "off encoding failed — keyboard still lit"
    );
    restore_white(&d);
}

/// The GET feature report must return the state we just set.
#[test]
#[ignore]
fn readback_round_trips_applied_state() {
    let d = open_or_abort();
    let cfg = LightingConfig {
        effect: Effect::Static,
        speed: 1,
        brightness: 2,
        zones: [
            Rgb::new(255, 0, 0),
            Rgb::new(0, 255, 0),
            Rgb::new(0, 0, 255),
            Rgb::new(200, 100, 50),
        ],
    };
    d.send(&cfg).unwrap();
    let state = d.read().expect("readback supported");
    let expect = [
        Rgb::new(255, 0, 0),
        Rgb::new(0, 255, 0),
        Rgb::new(0, 0, 255),
        Rgb::new(200, 100, 50),
    ];
    assert_eq!(
        state.zones, expect,
        "readback zone colours differ from what was sent"
    );
    assert_eq!(state.speed, 1);
    assert_eq!(state.brightness, 2);
    eprintln!("readback OK: {:?}", state.effect);
    restore_white(&d);
}

/// Probe undocumented effect codes. Informational only — nothing is asserted
/// because nothing is claimed. Results should be recorded in the hardware
/// report if any code produces a visible effect.
#[test]
#[ignore]
fn probe_undocumented_effect_codes() {
    let d = open_or_abort();
    for code in [0x02u8, 0x05, 0x07, 0x08] {
        let mut p = packet::build_packet(&LightingConfig {
            effect: Effect::Static,
            speed: 2,
            brightness: 2,
            zones: [Rgb::white(); 4],
        });
        p[packet::IDX_EFFECT] = code;
        d.send_raw(&p).unwrap();
        eprintln!("effect code 0x{code:02x} sent — watch the keyboard.");
        pause("press Enter when ready");
    }
    eprintln!("probe finished (no assertions — record anything visible in the docs)");
    restore_white(&d);
}
