//! The CC/16 feature-report packet format used by the ITE 4-zone RGB
//! keyboard controller (048d:c993 / "ITE Device(8295)").
//!
//! Packet layout (33 bytes total, report ID `0xCC` included as byte 0, which
//! is how hidapi feature reports are exchanged):
//!
//! ```text
//! [0]  0xCC          report ID
//! [1]  0x16          magic
//! [2]  effect        (see [`crate::model::Effect::code`])
//! [3]  speed         1..=4
//! [4]  brightness    1..=2 (0 only with effect 0 = off)
//! [5..16]  zone1..4 RGB, 3 bytes each — only meaningful for static/breath
//! [17] reserved (0)
//! [18] wave-direction flag A
//! [19] wave-direction flag B
//! [20..32] zero padding
//! ```
//!
//! The layout and semantics were cross-checked against three independent
//! open-source implementations — LegionAura, legion-keyboard-custom (which
//! lists the LOQ 15IRX9 as tested hardware) and the community script
//! documented on kaveh.page — all of which agree on this byte layout.
//! Visual direction labels are centralised below so one hardware
//! observation can relabel them if needed.

use crate::model::{DeviceState, Effect, LightingConfig, LiveEffect, Rgb, ZONE_COUNT};

pub const REPORT_ID: u8 = 0xCC;
pub const MAGIC: u8 = 0x16;
pub const PACKET_LEN: usize = 33;

// Byte indices inside the 33-byte buffer (index 0 is the report ID).
pub const IDX_EFFECT: usize = 2;
pub const IDX_SPEED: usize = 3;
pub const IDX_BRIGHTNESS: usize = 4;
pub const IDX_ZONES: usize = 5; // 12 consecutive bytes
pub const IDX_RESERVED: usize = 17;
/// Wave flag byte: set for the wave that sweeps **right** (byte 18).
pub const IDX_WAVE_RIGHT: usize = 18;
/// Wave flag byte: set for the wave that sweeps **left** (byte 19).
pub const IDX_WAVE_LEFT: usize = 19;

/// The 16.7 M-colour bytes the GUI shows for a zone become exactly these
/// three bytes; the firmware applies them to the physical LEDs.
fn zone_bytes(zones: &[Rgb; ZONE_COUNT]) -> [u8; 12] {
    let mut out = [0u8; 12];
    for (i, z) in zones.iter().enumerate() {
        let slot = &mut out[i * 3..i * 3 + 3];
        slot[0] = z.r;
        slot[1] = z.g;
        slot[2] = z.b;
    }
    out
}

/// Host-side dimming factor for brightness=1.
///
/// VERIFIED LIMITATION: on the LOQ 15IRX9 (048d:c993) the firmware ignores
/// the brightness byte entirely (Low and High look identical). To make the
/// Low setting meaningful we scale the zone colour bytes by this factor —
/// the same approach LegionAura uses for this hardware family. It applies
/// to every effect whose colour bytes we control (static, breathing and the
/// palette/rainbow waves); smooth flow's single internal colour cannot be
/// dimmed this way and is labelled accordingly in the UI.
pub const DIMMING_FACTOR: f32 = 0.6;

fn dim_rgb(c: Rgb) -> Rgb {
    Rgb::new(scale(c.r), scale(c.g), scale(c.b))
}

fn scale(c: u8) -> u8 {
    (c as f32 * DIMMING_FACTOR).round().clamp(0.0, 255.0) as u8
}

/// Automatic rainbow palette for the rainbow wave effects: four hues evenly
/// spread around the wheel (red → yellow-green → cyan → violet-blue). The
/// controller interpolates between the supplied colour samples as the wave
/// moves. Defined in the model (pure colour math) and re-exported here.
pub fn auto_rainbow_palette() -> [Rgb; ZONE_COUNT] {
    crate::model::rainbow_zones()
}

/// The colour bytes actually written to the LEDs for a config.
///
/// - User zone colours, dimmed by [`DIMMING_FACTOR`] at Low brightness, for
///   static/breath and the palette waves.
/// - The automatic rainbow palette (also dimmed at Low) for rainbow waves.
/// - Untouched zone colours for smooth/off (their bytes are not sent).
///
/// Single source of truth used by the packet builder, the readback
/// comparison and the GUI preview so "what you see" always equals "what is
/// sent".
pub fn led_zones(cfg: &LightingConfig) -> [Rgb; ZONE_COUNT] {
    let base = match cfg.effect {
        Effect::RainbowLeft | Effect::RainbowRight => auto_rainbow_palette(),
        Effect::FlowLeft | Effect::FlowRight => crate::flow::frame_zones(cfg, 0.0),
        _ => return effective_zones(cfg),
    };
    if cfg.brightness == 1 {
        base.map(dim_rgb)
    } else {
        base
    }
}

/// User zone colours with Low-brightness dimming applied, for the effects
/// that consume the user palette (static/breath/palette waves).
pub fn effective_zones(cfg: &LightingConfig) -> [Rgb; ZONE_COUNT] {
    let mut out = cfg.zones;
    if cfg.brightness == 1 && cfg.effect.writes_zone_bytes() {
        for c in &mut out {
            *c = dim_rgb(*c);
        }
    }
    out
}

/// Serialise a lighting config into the hardware packet. Pure function;
/// unit-tested byte-for-byte against golden vectors.
pub fn build_packet(cfg: &LightingConfig) -> [u8; PACKET_LEN] {
    let cfg = cfg.normalized();
    let mut p = [0u8; PACKET_LEN];
    p[0] = REPORT_ID;
    p[1] = MAGIC;
    p[IDX_EFFECT] = cfg.effect.code();
    p[IDX_SPEED] = cfg.speed;
    p[IDX_BRIGHTNESS] = cfg.brightness;
    if cfg.effect.writes_zone_bytes() {
        p[IDX_ZONES..IDX_ZONES + 12].copy_from_slice(&zone_bytes(&led_zones(&cfg)));
    }
    match cfg.effect {
        Effect::WaveRight | Effect::RainbowRight => p[IDX_WAVE_RIGHT] = 1,
        Effect::WaveLeft | Effect::RainbowLeft => p[IDX_WAVE_LEFT] = 1,
        _ => {}
    }
    p
}

fn effect_from_code(code: u8, flag_right: bool, flag_left: bool) -> LiveEffect {
    match code {
        0x00 => LiveEffect::Known(Effect::Off),
        0x01 => LiveEffect::Known(Effect::Static),
        0x03 => LiveEffect::Known(Effect::Breath),
        0x04 if flag_right => LiveEffect::Known(Effect::WaveRight),
        0x04 if flag_left => LiveEffect::Known(Effect::WaveLeft),
        0x06 => LiveEffect::Known(Effect::Smooth),
        other => LiveEffect::Unknown(other),
    }
}

/// Parse a GET feature-report response back into a device state. The buffer
/// must include the report ID at index 0 (as hidapi returns it). Unknown
/// effect codes are preserved, not guessed.
pub fn parse_state(buf: &[u8]) -> Option<DeviceState> {
    if buf.len() < PACKET_LEN || buf[0] != REPORT_ID || buf[1] != MAGIC {
        return None;
    }
    let mut zones = [Rgb::black(); ZONE_COUNT];
    for (i, z) in zones.iter_mut().enumerate() {
        let slot = &buf[IDX_ZONES + i * 3..IDX_ZONES + i * 3 + 3];
        *z = Rgb::new(slot[0], slot[1], slot[2]);
    }
    let flag_right = buf[IDX_WAVE_RIGHT] != 0;
    let flag_left = buf[IDX_WAVE_LEFT] != 0;
    Some(DeviceState {
        effect: effect_from_code(buf[IDX_EFFECT], flag_right, flag_left),
        speed: buf[IDX_SPEED],
        brightness: buf[IDX_BRIGHTNESS],
        zones,
        flag_right,
        flag_left,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(effect: Effect, speed: u8, brightness: u8) -> LightingConfig {
        LightingConfig {
            effect,
            speed,
            brightness,
            zones: [
                Rgb::new(255, 0, 0),
                Rgb::new(0, 255, 0),
                Rgb::new(0, 0, 255),
                Rgb::new(255, 255, 255),
            ],
        }
    }

    #[test]
    fn static_golden_packet() {
        let p = build_packet(&cfg(Effect::Static, 1, 2));
        assert_eq!(p[0], 0xCC);
        assert_eq!(p[1], 0x16);
        assert_eq!(p[2], 0x01);
        assert_eq!(p[3], 1);
        assert_eq!(p[4], 2);
        assert_eq!(&p[5..17], &[255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255]);
        assert_eq!(p[17], 0);
        assert_eq!(p[18], 0);
        assert_eq!(p[19], 0);
        assert!(p[20..].iter().all(|b| *b == 0));
    }

    #[test]
    fn breath_golden_packet() {
        // brightness 1 → colours are host-dimmed by DIMMING_FACTOR (0.6):
        // 255*0.6 = 153 = 0x99. This is the verified behaviour on the LOQ
        // 15IRX9, whose firmware ignores the brightness byte.
        let p = build_packet(&cfg(Effect::Breath, 2, 1));
        assert_eq!(p[2], 0x03);
        assert_eq!(
            &p[5..17],
            &[0x99, 0, 0, 0, 0x99, 0, 0, 0, 0x99, 0x99, 0x99, 0x99]
        );
    }

    #[test]
    fn high_brightness_sends_colours_unscaled() {
        let p = build_packet(&cfg(Effect::Breath, 2, 2));
        assert_eq!(&p[5..17], &[255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255]);
    }

    #[test]
    fn dimming_applies_to_every_effect_that_writes_colour_bytes() {
        let static_low = cfg(Effect::Static, 1, 1);
        assert_ne!(effective_zones(&static_low), static_low.zones);
        // The palette wave now also carries user colours, so Low dims them.
        let wave_low = cfg(Effect::WaveLeft, 1, 1);
        assert_ne!(effective_zones(&wave_low), wave_low.zones);
        // Smooth does not write zone bytes — nothing to dim, bytes stay zero.
        let smooth_low = cfg(Effect::Smooth, 1, 1);
        let p = build_packet(&smooth_low);
        assert!(p[5..17].iter().all(|b| *b == 0));
    }

    #[test]
    fn wave_carries_the_palette_bytes() {
        // Hardware-verified: the wave engine renders from the zone colours.
        let l = build_packet(&cfg(Effect::WaveLeft, 2, 2));
        assert_eq!(l[2], 0x04);
        assert_eq!(l[IDX_WAVE_LEFT], 1);
        assert_eq!(l[IDX_WAVE_RIGHT], 0);
        assert_eq!(&l[5..17], &[255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255]);

        let r = build_packet(&cfg(Effect::WaveRight, 3, 2));
        assert_eq!(r[IDX_WAVE_RIGHT], 1);
        assert_eq!(r[IDX_WAVE_LEFT], 0);
    }

    #[test]
    fn rainbow_wave_golden_palette() {
        // Auto rainbow hues 0/90/180/270 at full S/V:
        //   h0   -> #ff0000
        //   h90  -> #80ff00
        //   h180 -> #00ffff
        //   h270 -> #8000ff
        let r = build_packet(&cfg(Effect::RainbowRight, 2, 2));
        assert_eq!(r[2], 0x04);
        assert_eq!(r[IDX_WAVE_RIGHT], 1);
        assert_eq!(
            &r[5..17],
            &[255, 0, 0, 128, 255, 0, 0, 255, 255, 128, 0, 255]
        );

        let l = build_packet(&cfg(Effect::RainbowLeft, 2, 2));
        assert_eq!(l[IDX_WAVE_LEFT], 1);
        assert_eq!(l[IDX_WAVE_RIGHT], 0);
        // At Low brightness the rainbow palette is host-dimmed (×0.6):
        // 255→153 (0x99), 128→77 (0x4D).
        let dim = build_packet(&cfg(Effect::RainbowRight, 2, 1));
        assert_eq!(
            &dim[5..17],
            &[0x99, 0, 0, 0x4D, 0x99, 0, 0, 0x99, 0x99, 0x4D, 0, 0x99]
        );
    }

    #[test]
    fn smooth_and_off_golden() {
        let s = build_packet(&cfg(Effect::Smooth, 4, 2));
        assert_eq!(s[2], 0x06);
        assert_eq!(s[18], 0);
        assert_eq!(s[19], 0);
        assert!(s[5..17].iter().all(|b| *b == 0), "smooth writes no palette");

        let o = build_packet(&cfg(Effect::Off, 2, 2));
        assert_eq!(o[2], 0x00);
        assert_eq!(o[3], 1, "off forces speed to 1");
        assert_eq!(o[4], 0, "off forces brightness to 0");
    }

    #[test]
    fn length_is_always_33() {
        for e in [
            Effect::Off,
            Effect::Static,
            Effect::Breath,
            Effect::WaveLeft,
            Effect::WaveRight,
            Effect::RainbowLeft,
            Effect::RainbowRight,
            Effect::Smooth,
        ] {
            assert_eq!(build_packet(&cfg(e, 2, 2)).len(), PACKET_LEN);
        }
    }

    #[test]
    fn parse_state_round_trips_build() {
        // Rainbow is byte-identical to the palette wave (same code + flag),
        // so it cannot be told apart by readback — excluded here on purpose.
        for e in [
            Effect::Static,
            Effect::Breath,
            Effect::WaveLeft,
            Effect::WaveRight,
            Effect::Smooth,
        ] {
            let cfg = cfg(e, 3, 1);
            let p = build_packet(&cfg);
            let st = parse_state(&p).expect("valid packet parses");
            assert_eq!(st.effect, LiveEffect::Known(e), "effect {e:?}");
            assert_eq!(st.speed, 3);
            assert_eq!(st.brightness, 1);
            if e.writes_zone_bytes() {
                // Round trip carries exactly the (possibly dimmed) bytes sent.
                assert_eq!(st.zones, led_zones(&cfg));
            }
        }
        // Off packet: effect known Off, brightness 0.
        let o = parse_state(&build_packet(&cfg(Effect::Off, 2, 2))).unwrap();
        assert_eq!(o.effect, LiveEffect::Known(Effect::Off));
    }

    #[test]
    fn parse_rainbow_packet_reports_wave_with_palette_bytes() {
        let p = build_packet(&cfg(Effect::RainbowRight, 2, 2));
        let st = parse_state(&p).unwrap();
        assert_eq!(st.effect, LiveEffect::Known(Effect::WaveRight));
        assert_eq!(st.zones, auto_rainbow_palette());
    }

    #[test]
    fn parse_state_preserves_unknown_effect() {
        let mut p = [0u8; PACKET_LEN];
        p[0] = REPORT_ID;
        p[1] = MAGIC;
        p[2] = 0x42; // a code this project does not map
        p[3] = 2;
        p[4] = 2;
        let st = parse_state(&p).unwrap();
        assert_eq!(st.effect, LiveEffect::Unknown(0x42));
    }

    #[test]
    fn parse_state_rejects_garbage() {
        assert!(parse_state(&[0u8; 10]).is_none());
        let mut bad = [0u8; PACKET_LEN];
        bad[0] = 0xAB; // wrong report id
        assert!(parse_state(&bad).is_none());
    }
}
