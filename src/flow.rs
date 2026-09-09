//! Host-rendered continuous colour wave (the "colour flow" effect).
//!
//! Why this effect exists at all (hardware analysis, verified live on the
//! LOQ 15IRX9):
//! - Firmware "Wave" (0x04) sweeps a SINGLE colour band: palette colours
//!   appear one after another, which reads as "one colour appears → delay →
//!   next colour". No CC packet can change that — it is the firmware's own
//!   design.
//! - Firmware "Smooth flow" (0x06) shifts the WHOLE keyboard through colours
//!   in lockstep — never multiple colours at once.
//! - No undocumented effect code provides a continuous multicolour wave
//!   (codes 0x02/0x05/0x07-0x0A are all inert).
//!
//! So this module renders the effect in software: every frame is ONE CC
//! feature report that sets all four zone colours simultaneously (there is
//! no sequential zone updating in this app — one report carries the whole
//! keyboard state). Frames are written at a steady ~25 fps.
//!
//! Two colour modes:
//! - **Palette mode**: the user's four zone colours act as colour blocks on
//!   a continuous loop. As the phase advances every zone glides smoothly
//!   through the palette (each colour blends into the next — no waiting, no
//!   steps), and the whole pattern moves left/right. The four physical
//!   zones are a hardware limit: at most four colour regions can be lit at
//!   any instant.
//! - **Spectrum mode** (fallback when all four zone colours are identical,
//!   i.e. untouched): the full 360° colour wheel is spread across the
//!   keyboard and glides the same way, so every zone visits every colour.
//!
//! The phase is periodic (period = 4 zone spacings) and the sampling
//! function is continuous across the wrap, so the loop never visibly jumps
//! or restarts.

use crate::model::{Effect, LightingConfig, Rgb, ZONE_COUNT};

/// Frame interval used by renderers (~25 fps: smooth without flooding USB).
pub const FRAME_INTERVAL: f32 = 0.040;
pub const FRAME_RATE: f32 = 1.0 / FRAME_INTERVAL;

/// Velocity of the wave along the keyboard, in zone spacings per second, for
/// each speed setting (1..=4). One spacing = one zone.
pub fn velocity(speed: u8) -> f32 {
    match speed.clamp(1, 4) {
        1 => 0.4,
        2 => 0.8,
        3 => 1.6,
        _ => 3.0,
    }
}

/// Are the four zone colours distinct enough to act as the wave palette?
/// Identical zones (e.g. untouched white defaults) mean "use the spectrum".
pub fn uses_palette(cfg: &LightingConfig) -> bool {
    !cfg.zones.iter().all(|c| *c == cfg.zones[0])
}

fn lerp_byte(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t)
        .round()
        .clamp(0.0, 255.0) as u8
}

/// Sample the periodic palette loop (period = ZONE_COUNT anchor spacings) at
/// a continuous position `pos`. Integer positions return the anchors
/// themselves; between them the colours blend smoothly.
fn sample_palette(anchors: &[Rgb; ZONE_COUNT], pos: f32) -> Rgb {
    let n = ZONE_COUNT as f32;
    let t = pos.rem_euclid(n);
    let idx = t.floor() as usize % ZONE_COUNT;
    let frac = t - t.floor();
    let next = (idx + 1) % ZONE_COUNT;
    let a = anchors[idx];
    let b = anchors[next];
    Rgb::new(
        lerp_byte(a.r, b.r, frac),
        lerp_byte(a.g, b.g, frac),
        lerp_byte(a.b, b.b, frac),
    )
}

/// Zone colours for one animation frame at gradient offset `phase` (in zone
/// spacings, any real value).
///
/// Direction: `FlowRight` moves the colours so they travel to the right;
/// `FlowLeft` travels left. At `phase == 0` palette mode shows the anchors
/// themselves on their zones (zone 1 = block colour 1, …); spectrum mode
/// shows hues 0°/90°/180°/270°.
pub fn frame_zones(cfg: &LightingConfig, phase: f32) -> [Rgb; ZONE_COUNT] {
    let palette_mode = uses_palette(cfg);
    let mut out = [Rgb::black(); ZONE_COUNT];
    for (i, zone) in out.iter_mut().enumerate() {
        // Subtracting the phase makes the whole pattern glide rightwards as
        // the phase grows (the value seen at a fixed zone is the value that
        // used to sit to its left).
        let pos = match cfg.effect {
            Effect::FlowRight => i as f32 - phase,
            Effect::FlowLeft => i as f32 + phase,
            _ => i as f32,
        };
        *zone = if palette_mode {
            sample_palette(&cfg.zones, pos)
        } else {
            // Full-spectrum wheel: hues evenly spread over the keyboard.
            let hue = (pos * 90.0).rem_euclid(360.0);
            Rgb::from_hsv(hue, 1.0, 1.0)
        };
    }
    out
}

/// Advance the phase by one frame step at the given speed. Phase is kept in
/// [0, 4) (one full pass of the wave over the keyboard).
pub fn advance_phase(effect: Effect, speed: u8, phase: f32, dt: f32) -> f32 {
    let n = ZONE_COUNT as f32;
    match effect {
        Effect::FlowLeft | Effect::FlowRight => (phase + velocity(speed) * dt).rem_euclid(n),
        _ => phase,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_cfg(effect: Effect, zones: [Rgb; ZONE_COUNT]) -> LightingConfig {
        LightingConfig {
            effect,
            speed: 2,
            brightness: 2,
            zones,
        }
    }

    fn white_cfg(effect: Effect) -> LightingConfig {
        make_cfg(effect, [Rgb::white(); ZONE_COUNT])
    }

    fn palette_cfg(effect: Effect) -> LightingConfig {
        // Four clearly separated colour blocks: red, yellow, green, blue.
        make_cfg(
            effect,
            [
                Rgb::new(255, 0, 0),
                Rgb::new(255, 230, 0),
                Rgb::new(0, 220, 60),
                Rgb::new(0, 90, 255),
            ],
        )
    }

    #[test]
    fn untouched_defaults_use_the_full_spectrum() {
        let c = white_cfg(Effect::FlowRight);
        assert!(!uses_palette(&c));
        let frame = frame_zones(&c, 0.0);
        assert_eq!(frame, crate::model::rainbow_zones());
        // Multiple colours visible simultaneously.
        let mut uniq = frame.to_vec();
        uniq.dedup();
        assert_eq!(uniq.len(), ZONE_COUNT);
    }

    #[test]
    fn user_palette_is_used_when_zones_differ() {
        let c = palette_cfg(Effect::FlowRight);
        assert!(uses_palette(&c));
        // Phase 0: each zone shows exactly its own block colour.
        assert_eq!(frame_zones(&c, 0.0), c.zones);
    }

    #[test]
    fn palette_blocks_blend_smoothly_between_zones() {
        // Half a block later each zone sits between two palette colours —
        // no stepping: the blend is a weighted mix of neighbours.
        let c = palette_cfg(Effect::FlowRight);
        let frame = frame_zones(&c, 0.5);
        assert_eq!(frame[0], sample_palette(&c.zones, -0.5));
        // Mid-blend colours differ from the anchor they started at.
        assert_ne!(frame[0], c.zones[0]);
    }

    #[test]
    fn wave_is_periodic_and_loops_without_jump() {
        for c in [palette_cfg(Effect::FlowRight), white_cfg(Effect::FlowRight)] {
            // Exact periodicity.
            assert_eq!(frame_zones(&c, 4.0), frame_zones(&c, 0.0));
            assert_eq!(frame_zones(&c, 6.25), frame_zones(&c, 2.25));
            // Continuity across the loop point: frames just before and just
            // after the wrap are nearly identical (no restart jump). Custom
            // palettes have a tiny colour corner at the seam (their first and
            // last colours differ, so the blend approaching block 1 comes
            // from a different neighbour than the blend leaving block 4) —
            // a few RGB levels, invisible at 25 fps.
            let before = frame_zones(&c, 3.99);
            let after = frame_zones(&c, 0.01);
            for (x, y) in before.iter().zip(after.iter()) {
                let (dr, dg, db) = (
                    (x.r as i16 - y.r as i16).abs(),
                    (x.g as i16 - y.g as i16).abs(),
                    (x.b as i16 - y.b as i16).abs(),
                );
                assert!(
                    dr <= 12 && dg <= 12 && db <= 12,
                    "loop seam too large: {x:?} vs {y:?} (Δ {dr},{dg},{db})"
                );
            }
        }
    }

    #[test]
    fn consecutive_frames_are_smooth_no_flicker() {
        // Per-frame colour deltas must stay small at the slowest setting
        // (velocity 0.4 zones/s → 0.016 zones/frame).
        for mut c in [palette_cfg(Effect::FlowRight), white_cfg(Effect::FlowRight)] {
            c.speed = 1;
            let a = frame_zones(&c, 0.0);
            let b = frame_zones(&c, velocity(1) * FRAME_INTERVAL);
            for (x, y) in a.iter().zip(b.iter()) {
                let d = (x.r as i16 - y.r as i16).abs()
                    + (x.g as i16 - y.g as i16).abs()
                    + (x.b as i16 - y.b as i16).abs();
                assert!(d <= 24, "{x:?} -> {y:?}");
            }
        }
    }

    #[test]
    fn directions_move_opposite_ways() {
        let right = palette_cfg(Effect::FlowRight);
        let left = palette_cfg(Effect::FlowLeft);
        assert_ne!(frame_zones(&right, 0.5)[0], frame_zones(&left, 0.5)[0]);
    }

    #[test]
    fn every_spectrum_zone_visits_every_hue_over_a_cycle() {
        use std::collections::HashSet;
        let c = white_cfg(Effect::FlowRight);
        let mut hues_seen: [HashSet<u16>; ZONE_COUNT] = Default::default();
        for step in 0..400 {
            let frame = frame_zones(&c, step as f32 * 0.01);
            for (i, zone) in frame.iter().enumerate() {
                hues_seen[i].insert((zone.to_hsv().0 as u16 / 30) * 30);
            }
        }
        for (i, seen) in hues_seen.iter().enumerate() {
            assert_eq!(seen.len(), 12, "zone {i} must visit all 12 hue buckets");
        }
    }

    #[test]
    fn phase_advance_respects_speed_and_wraps() {
        assert!(velocity(4) > velocity(1));
        let p = advance_phase(Effect::FlowRight, 4, 3.9, FRAME_INTERVAL);
        assert!((0.0..4.0).contains(&p), "phase wraps into [0,4): {p}");
        assert_eq!(advance_phase(Effect::Static, 2, 1.0, 0.1), 1.0);
    }

    #[test]
    fn low_brightness_is_dimmed_once_by_the_builder() {
        let mut c = palette_cfg(Effect::FlowRight);
        c.brightness = 1;
        let packet = crate::packet::build_packet(&c);
        // 255*0.6=153 (0x99), 230*0.6=138 (0x8A), 220*0.6=132 (0x84),
        // 60*0.6=36 (0x24), 90*0.6=54 (0x36).
        assert_eq!(
            &packet[5..17],
            &[
                0x99, 0x00, 0x00, 0x99, 0x8A, 0x00, 0x00, 0x84, 0x24, 0x00, 0x36, 0x99
            ]
        );
    }
}
