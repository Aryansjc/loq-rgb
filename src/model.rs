//! Core data model: colours, effects, lighting configurations.
//!
//! Everything in this module is pure and hardware-independent so it can be
//! unit-tested exhaustively. The single source of truth for "what the
//! hardware accepts" lives here as typed values plus validation helpers.

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Number of independently addressable colour zones on the LOQ 4-zone
/// keyboard (verified against the 0xCC/0x16 feature report layout).
pub const ZONE_COUNT: usize = 4;

// ---------------------------------------------------------------------------
// Colour
// ---------------------------------------------------------------------------

/// An 8-bit-per-channel RGB colour. Serialized as `"#rrggbb"` so users can
/// hand-edit config files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub const fn black() -> Self {
        Self { r: 0, g: 0, b: 0 }
    }

    pub const fn white() -> Self {
        Self {
            r: 255,
            g: 255,
            b: 255,
        }
    }

    /// Parse `#rrggbb` or `rrggbb` (case-insensitive). Returns `None` on
    /// malformed input; this is the only colour parser the app uses.
    pub fn from_hex(s: &str) -> Option<Self> {
        let t = s.strip_prefix('#').unwrap_or(s);
        if t.len() != 6 {
            return None;
        }
        let nib = |c: u8| -> Option<u8> {
            match c {
                b'0'..=b'9' => Some(c - b'0'),
                b'a'..=b'f' => Some(c - b'a' + 10),
                b'A'..=b'F' => Some(c - b'A' + 10),
                _ => None,
            }
        };
        let b = t.as_bytes();
        let mut v = [0u8; 6];
        for (i, byte) in b.iter().enumerate() {
            v[i] = nib(*byte)?;
        }
        Some(Self {
            r: (v[0] << 4) | v[1],
            g: (v[2] << 4) | v[3],
            b: (v[4] << 4) | v[5],
        })
    }

    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// Hue in degrees [0, 360), saturation and value in [0, 1].
    pub fn to_hsv(self) -> (f32, f32, f32) {
        let r = self.r as f32 / 255.0;
        let g = self.g as f32 / 255.0;
        let b = self.b as f32 / 255.0;
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let d = max - min;
        let h = if d == 0.0 {
            0.0
        } else if max == r {
            60.0 * (((g - b) / d).rem_euclid(6.0))
        } else if max == g {
            60.0 * (((b - r) / d) + 2.0)
        } else {
            60.0 * (((r - g) / d) + 4.0)
        };
        let s = if max == 0.0 { 0.0 } else { d / max };
        (h, s, max)
    }

    /// Build from HSV. Hue in degrees [0, 360), saturation and value in
    /// [0, 1]. Out-of-range components are clamped.
    pub fn from_hsv(hue: f32, sat: f32, val: f32) -> Self {
        let h = (hue.rem_euclid(360.0)) / 60.0;
        let s = sat.clamp(0.0, 1.0);
        let v = val.clamp(0.0, 1.0);
        let c = v * s;
        let x = c * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
        let (r, g, b) = match h as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = v - c;
        Self::new(
            ((r + m) * 255.0).round() as u8,
            ((g + m) * 255.0).round() as u8,
            ((b + m) * 255.0).round() as u8,
        )
    }
}

impl Serialize for Rgb {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = Rgb;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an RGB hex string like \"#ff00aa\"")
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<Rgb, E> {
                Rgb::from_hex(s).ok_or_else(|| E::custom(format!("invalid colour {s:?}")))
            }
        }
        deserializer.deserialize_str(V)
    }
}

// ---------------------------------------------------------------------------
// Effects
// ---------------------------------------------------------------------------

/// Lighting effects. These are exactly the effect codes the 4-zone ITE
/// firmware exposes (verified against the CC/16 protocol: 0x01 static,
/// 0x03 breath, 0x04 wave with a direction flag, 0x06 smooth flow).
///
/// Naming note: `WaveLeft`/`WaveRight` describe the *visual sweep
/// direction* as observed on hardware. Byte-level direction mapping is
/// centralised in [`crate::packet`] so a single hardware observation can
/// relabel it without touching anything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[derive(Default)]
pub enum Effect {
    Off,
    #[default]
    Static,
    Breath,
    WaveLeft,
    WaveRight,
    /// Full-spectrum wave: like the palette wave but the controller receives
    /// an automatic rainbow palette. Verified on hardware that the wave
    /// engine renders from the supplied zone-colour bytes.
    RainbowLeft,
    RainbowRight,
    /// HOST-RENDERED continuous colour flow (leftwards/rightwards): the
    /// firmware has no continuous multi-colour gradient effect (verified by
    /// probing every undocumented effect code), so this effect is drawn by
    /// this software — it continuously writes new static frames whose four
    /// zone colours are samples of a moving colour gradient. The zone
    /// colours act as the gradient's anchor palette. Honest label: the
    /// animation only runs while the GUI or the `listen-hotkeys` daemon is
    /// active, and transitions between the four physical zones are as smooth
    /// as 4-zone hardware allows.
    FlowLeft,
    FlowRight,
    Smooth,
}

impl std::fmt::Display for Effect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Effect::Off => "off",
            Effect::Static => "static",
            Effect::Breath => "breath",
            Effect::WaveLeft => "wave-left",
            Effect::WaveRight => "wave-right",
            Effect::RainbowLeft => "rainbow-left",
            Effect::RainbowRight => "rainbow-right",
            Effect::FlowLeft => "flow-left",
            Effect::FlowRight => "flow-right",
            Effect::Smooth => "smooth",
        };
        f.write_str(s)
    }
}

/// A rainbow-ish default palette: hues evenly spread around the wheel
/// (red → yellow-green → cyan → violet-blue).
pub fn rainbow_zones() -> [Rgb; ZONE_COUNT] {
    [
        Rgb::from_hsv(0.0, 1.0, 1.0),
        Rgb::from_hsv(90.0, 1.0, 1.0),
        Rgb::from_hsv(180.0, 1.0, 1.0),
        Rgb::from_hsv(270.0, 1.0, 1.0),
    ]
}

impl Effect {
    /// Raw firmware code for this effect (0 when `Off`). Wave and rainbow
    /// share the wave engine (0x04) and differ only in the palette bytes we
    /// supply. Host-rendered flow effects emit static frames, so their code
    /// is the static code (0x01).
    pub fn code(self) -> u8 {
        match self {
            Effect::Off => 0x00,
            Effect::Static => 0x01,
            Effect::FlowLeft | Effect::FlowRight => 0x01,
            Effect::Breath => 0x03,
            Effect::WaveLeft | Effect::WaveRight | Effect::RainbowLeft | Effect::RainbowRight => {
                0x04
            }
            Effect::Smooth => 0x06,
        }
    }

    /// Does the firmware read the per-zone colour bytes for this effect?
    ///
    /// Hardware-verified on the LOQ 15IRX9: static and breath render each
    /// zone's own colour, and the wave engine renders its moving pattern
    /// from the four zone-colour bytes (sending red/green/blue/white made
    /// the wave multicoloured). The host colour-flow effect uses the zone
    /// colours as its moving wave palette too (falling back to the full
    /// spectrum when they are all identical). Smooth flow drives its own
    /// colour.
    pub fn uses_zone_colors(self) -> bool {
        matches!(
            self,
            Effect::Static
                | Effect::Breath
                | Effect::WaveLeft
                | Effect::WaveRight
                | Effect::FlowLeft
                | Effect::FlowRight
        )
    }

    /// Does the packet carry zone-colour bytes for this effect at all
    /// (user palette for static/breath/wave/flow, auto rainbow palette for
    /// the rainbow waves)? Smooth and Off do not.
    pub fn writes_zone_bytes(self) -> bool {
        matches!(
            self,
            Effect::Static
                | Effect::Breath
                | Effect::WaveLeft
                | Effect::WaveRight
                | Effect::RainbowLeft
                | Effect::RainbowRight
                | Effect::FlowLeft
                | Effect::FlowRight
        )
    }

    /// Is this an animation (breath/wave/rainbow/flow/smooth) whose speed
    /// matters (a firmware byte, or the software tempo for host flow)?
    pub fn is_animated(self) -> bool {
        matches!(
            self,
            Effect::Breath
                | Effect::WaveLeft
                | Effect::WaveRight
                | Effect::RainbowLeft
                | Effect::RainbowRight
                | Effect::FlowLeft
                | Effect::FlowRight
                | Effect::Smooth
        )
    }

    /// Is this a host-rendered effect that needs a continuous renderer?
    pub fn is_host_rendered(self) -> bool {
        matches!(self, Effect::FlowLeft | Effect::FlowRight)
    }

    /// Does this effect need a direction (a firmware flag, or software
    /// direction for host flow)?
    pub fn needs_direction(self) -> bool {
        matches!(
            self,
            Effect::WaveLeft
                | Effect::WaveRight
                | Effect::RainbowLeft
                | Effect::RainbowRight
                | Effect::FlowLeft
                | Effect::FlowRight
        )
    }
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// Valid speeds (firmware range 1..=4).
pub const SPEED_MIN: u8 = 1;
pub const SPEED_MAX: u8 = 4;
/// Valid brightness levels (firmware range 1..=2; 0 is only legal for Off).
pub const BRIGHTNESS_MIN: u8 = 1;
pub const BRIGHTNESS_MAX: u8 = 2;

/// The complete lighting state the hardware can hold: one global effect,
/// speed, brightness and four zone colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LightingConfig {
    pub effect: Effect,
    pub speed: u8,
    pub brightness: u8,
    pub zones: [Rgb; ZONE_COUNT],
}

impl Default for LightingConfig {
    fn default() -> Self {
        Self {
            effect: Effect::Static,
            speed: 2,
            brightness: 2,
            zones: [Rgb::white(); ZONE_COUNT],
        }
    }
}

impl LightingConfig {
    /// Clamp every field into the firmware-valid range. Colour values need
    /// no clamping (u8 is exact). Zone colours are deliberately preserved
    /// even for `Off`/wave/smooth: the packet builder simply does not send
    /// them for effects that ignore them, so switching back to static never
    /// loses the user's colours.
    pub fn normalized(mut self) -> Self {
        self.speed = self.speed.clamp(SPEED_MIN, SPEED_MAX);
        if self.effect == Effect::Off {
            // The off encoding forces these bytes; keep the stored config
            // sane so switching to another effect starts from valid state.
            self.brightness = 0;
            self.speed = 1;
        } else {
            self.brightness = self.brightness.clamp(BRIGHTNESS_MIN, BRIGHTNESS_MAX);
        }
        self
    }

    pub fn is_off(&self) -> bool {
        self.effect == Effect::Off
    }
}

// ---------------------------------------------------------------------------
// Device state readback
// ---------------------------------------------------------------------------

/// What the controller reports through the GET feature report. Unknown
/// effect codes (e.g. set by the hardware Fn key while we were away) are
/// preserved instead of being guessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveEffect {
    Known(Effect),
    Unknown(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceState {
    pub effect: LiveEffect,
    pub speed: u8,
    pub brightness: u8,
    pub zones: [Rgb; ZONE_COUNT],
    /// Raw wave flag bytes, preserved for diagnostics.
    pub flag_right: bool,
    pub flag_left: bool,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        for s in [
            "#000000", "#ffffff", "#ff0000", "#00ff00", "#0000ff", "#12abCD",
        ] {
            let rgb = Rgb::from_hex(s).unwrap();
            assert_eq!(rgb.to_hex(), s.to_lowercase());
        }
    }

    #[test]
    fn hex_parses_without_hash() {
        assert_eq!(Rgb::from_hex("ff0000"), Some(Rgb::new(255, 0, 0)));
    }

    #[test]
    fn hex_rejects_bad_input() {
        for bad in [
            "", "#fff", "#gggggg", "12345", "#12345", "1234567", "0xff0000",
        ] {
            assert_eq!(Rgb::from_hex(bad), None, "should reject {bad:?}");
        }
    }

    #[test]
    fn hsv_round_trip_for_primary_colours() {
        for c in [
            Rgb::new(255, 0, 0),
            Rgb::new(0, 255, 0),
            Rgb::new(0, 0, 255),
            Rgb::new(255, 255, 255),
            Rgb::new(0, 0, 0),
            Rgb::new(128, 64, 200),
        ] {
            let (h, s, v) = c.to_hsv();
            let back = Rgb::from_hsv(h, s, v);
            // Allow 1/255 rounding tolerance.
            assert!(
                (back.r as i16 - c.r as i16).abs() <= 1
                    && (back.g as i16 - c.g as i16).abs() <= 1
                    && (back.b as i16 - c.b as i16).abs() <= 1,
                "hsv round trip mismatch: {c:?} -> {back:?}"
            );
        }
    }

    #[test]
    fn effect_codes_match_protocol() {
        assert_eq!(Effect::Off.code(), 0x00);
        assert_eq!(Effect::Static.code(), 0x01);
        assert_eq!(Effect::Breath.code(), 0x03);
        assert_eq!(Effect::WaveLeft.code(), 0x04);
        assert_eq!(Effect::WaveRight.code(), 0x04);
        assert_eq!(Effect::RainbowLeft.code(), 0x04);
        assert_eq!(Effect::RainbowRight.code(), 0x04);
        assert_eq!(Effect::Smooth.code(), 0x06);
    }

    #[test]
    fn zone_colour_consumers_are_static_breath_wave_and_flow() {
        // Static/breath render each zone's colour; hardware-verified that the
        // wave engine also renders from the zone bytes (our palette). The
        // host colour flow uses the zone colours as its wave palette too.
        for e in [
            Effect::Static,
            Effect::Breath,
            Effect::WaveLeft,
            Effect::WaveRight,
            Effect::FlowLeft,
            Effect::FlowRight,
        ] {
            assert!(e.uses_zone_colors(), "{e:?}");
        }
        // Rainbow waves take an automatic palette; smooth and off draw their
        // own visuals — the user's zone colours are not editable for these.
        for e in [
            Effect::Off,
            Effect::RainbowLeft,
            Effect::RainbowRight,
            Effect::Smooth,
        ] {
            assert!(!e.uses_zone_colors(), "{e:?}");
        }
    }

    #[test]
    fn zone_bytes_are_written_for_all_colour_driven_effects() {
        for e in [
            Effect::Static,
            Effect::Breath,
            Effect::WaveLeft,
            Effect::WaveRight,
            Effect::RainbowLeft,
            Effect::RainbowRight,
            Effect::FlowLeft,
            Effect::FlowRight,
        ] {
            assert!(e.writes_zone_bytes(), "{e:?}");
        }
        for e in [Effect::Off, Effect::Smooth] {
            assert!(!e.writes_zone_bytes(), "{e:?}");
        }
    }

    #[test]
    fn flows_are_host_rendered_animated_directional() {
        for e in [Effect::FlowLeft, Effect::FlowRight] {
            assert!(e.is_host_rendered(), "{e:?}");
            assert!(e.is_animated(), "{e:?}");
            assert!(e.needs_direction(), "{e:?}");
        }
        assert!(!Effect::Static.is_host_rendered());
        assert!(!Effect::Smooth.is_host_rendered());
    }

    #[test]
    fn normalized_clamps_and_off_keeps_colours() {
        let cfg = LightingConfig {
            effect: Effect::WaveLeft,
            speed: 99,
            brightness: 0,
            ..LightingConfig::default()
        };
        let n = cfg.normalized();
        assert_eq!(n.speed, SPEED_MAX);
        assert_eq!(n.brightness, BRIGHTNESS_MIN);

        let colours = [
            Rgb::new(1, 2, 3),
            Rgb::new(4, 5, 6),
            Rgb::new(7, 8, 9),
            Rgb::new(10, 11, 12),
        ];
        let off = LightingConfig {
            effect: Effect::Off,
            speed: 3,
            brightness: 2,
            zones: colours,
        }
        .normalized();
        assert_eq!(off.brightness, 0);
        assert_eq!(off.speed, 1);
        // Switching effects must never lose the user's colours; the packet
        // builder omits them for Off/wave/smooth instead.
        assert_eq!(off.zones, colours);
    }

    #[test]
    fn rgb_serde_as_hex_string() {
        let json = serde_json::to_string(&Rgb::new(0x12, 0xab, 0xcd)).unwrap();
        assert_eq!(json, "\"#12abcd\"");
        let back: Rgb = serde_json::from_str(&json).unwrap();
        assert_eq!(back, Rgb::new(0x12, 0xab, 0xcd));
        assert!(serde_json::from_str::<Rgb>("\"#nope\"").is_err());
    }

    #[test]
    fn effect_serde_kebab_case() {
        for (e, s) in [
            (Effect::Off, "\"off\""),
            (Effect::Static, "\"static\""),
            (Effect::WaveLeft, "\"wave-left\""),
            (Effect::Smooth, "\"smooth\""),
        ] {
            assert_eq!(serde_json::to_string(&e).unwrap(), s);
            assert_eq!(serde_json::from_str::<Effect>(s).unwrap(), e);
        }
    }
}
