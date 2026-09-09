//! Effect policy: the layer that keeps the UI honest about what the
//! hardware can actually do simultaneously.
//!
//! Hardware facts (verified from the CC/16 protocol and the Windows/Linux
//! tools that implement it):
//! - The controller holds ONE global effect. There is no per-zone effect
//!   slot, so "zone 1 breathing, zone 2 static" is physically impossible
//!   and is never offered.
//! - Zone colours are read by the firmware only for static and breath.
//!   Wave and smooth use fixed internal palettes; editing zone colours for
//!   those effects would silently do nothing, so the UI disables them with
//!   an explanation.

use crate::model::Effect;

/// Why a control is unavailable — shown verbatim in the UI/CLI so no
/// control ever appears dead without a reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsupportedReason {
    /// Per-zone effect mixing does not exist in the firmware.
    NoPerZoneEffects,
    /// This effect ignores the per-zone colour bytes.
    EffectIgnoresZoneColors,
    /// This effect has no direction.
    EffectHasNoDirection,
    /// Static/off have no animation speed.
    EffectHasNoSpeed,
}

impl UnsupportedReason {
    pub fn text(self) -> &'static str {
        match self {
            UnsupportedReason::NoPerZoneEffects => {
                "The keyboard firmware applies ONE effect to the whole keyboard. \
                 Per-zone effects (e.g. breathing on zone 1, static on zone 2) are not \
                 possible on this hardware."
            }
            UnsupportedReason::EffectIgnoresZoneColors => {
                "This effect drives its own colours (a built-in firmware flow, an automatic \
                 rainbow palette, or the host-rendered full colour wheel) — the four zone \
                 colours you pick are not used."
            }
            UnsupportedReason::EffectHasNoDirection => {
                "Only the wave and flow effects have a direction."
            }
            UnsupportedReason::EffectHasNoSpeed => {
                "This effect is not animated, so the speed setting does not apply."
            }
        }
    }
}

/// All effects offered, in UI order, with the label shown to users.
/// This list is the single source the GUI and CLI both enumerate.
pub struct EffectInfo {
    pub effect: Effect,
    pub label: &'static str,
    pub blurb: &'static str,
}

pub const EFFECTS: &[EffectInfo] = &[
    EffectInfo {
        effect: Effect::Off,
        label: "Off",
        blurb: "Turns the backlight completely off.",
    },
    EffectInfo {
        effect: Effect::Static,
        label: "Static",
        blurb: "Four zone colours, held constant.",
    },
    EffectInfo {
        effect: Effect::Breath,
        label: "Breathing",
        blurb: "All four zone colours pulse in sync.",
    },
    EffectInfo {
        effect: Effect::WaveLeft,
        label: "Wave → left",
        blurb: "Multicolour wave sweeping leftwards. The four zone colours form the wave's palette.",
    },
    EffectInfo {
        effect: Effect::WaveRight,
        label: "Wave → right",
        blurb: "Multicolour wave sweeping rightwards. The four zone colours form the wave's palette.",
    },
    EffectInfo {
        effect: Effect::RainbowLeft,
        label: "Rainbow wave → left",
        blurb: "Full-spectrum wave sweeping leftwards with an automatic rainbow palette.",
    },
    EffectInfo {
        effect: Effect::RainbowRight,
        label: "Rainbow wave → right",
        blurb: "Full-spectrum wave sweeping rightwards with an automatic rainbow palette.",
    },
    EffectInfo {
        effect: Effect::FlowLeft,
        label: "Colour wave ←",
        blurb: "HOST-RENDERED moving colour wave. Your zone colours are the blocks of the wave (full colour spectrum if all four are identical); they glide leftwards with smooth blends. Runs while the GUI or the listener daemon is active.",
    },
    EffectInfo {
        effect: Effect::FlowRight,
        label: "Colour wave →",
        blurb: "HOST-RENDERED moving colour wave. Your zone colours are the blocks of the wave (full colour spectrum if all four are identical); they glide rightwards with smooth blends. Runs while the GUI or the listener daemon is active.",
    },
    EffectInfo {
        effect: Effect::Smooth,
        label: "Smooth flow",
        blurb: "Whole keyboard shifts smoothly through colours over time.",
    },
];

/// Find UI metadata for an effect.
pub fn info(effect: Effect) -> &'static EffectInfo {
    EFFECTS
        .iter()
        .find(|e| e.effect == effect)
        .expect("every Effect has UI metadata")
}

/// May the user edit per-zone colours for this effect?
pub fn zone_colors_editable(effect: Effect) -> bool {
    effect.uses_zone_colors()
}

/// If colours are not editable, why not?
pub fn zone_colors_disabled_reason(effect: Effect) -> Option<UnsupportedReason> {
    if effect.uses_zone_colors() {
        None
    } else {
        Some(UnsupportedReason::EffectIgnoresZoneColors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_model_effect_is_present_exactly_once() {
        let mut seen: Vec<Effect> = EFFECTS.iter().map(|e| e.effect).collect();
        seen.sort_by_key(|e| e.code());
        seen.dedup();
        let all = [
            Effect::Off,
            Effect::Static,
            Effect::Breath,
            Effect::WaveLeft,
            Effect::WaveRight,
            Effect::RainbowLeft,
            Effect::RainbowRight,
            Effect::FlowLeft,
            Effect::FlowRight,
            Effect::Smooth,
        ];
        let mut all_sorted = all.to_vec();
        all_sorted.sort_by_key(|e| e.code());
        assert_eq!(seen, all_sorted);
    }

    #[test]
    fn colour_editing_policy_matches_firmware() {
        // User palette editable for static, breathing, the palette waves and
        // the host colour-flow wave.
        for e in [
            Effect::Static,
            Effect::Breath,
            Effect::WaveLeft,
            Effect::WaveRight,
            Effect::FlowLeft,
            Effect::FlowRight,
        ] {
            assert!(zone_colors_editable(e), "{e:?}");
            assert_eq!(zone_colors_disabled_reason(e), None);
        }
        // Off, rainbow waves and smooth use their own visuals — user zone
        // colours are not editable.
        for e in [
            Effect::Off,
            Effect::RainbowLeft,
            Effect::RainbowRight,
            Effect::Smooth,
        ] {
            assert!(!zone_colors_editable(e), "{e:?}");
            assert_eq!(
                zone_colors_disabled_reason(e),
                Some(UnsupportedReason::EffectIgnoresZoneColors)
            );
        }
    }

    #[test]
    fn mixing_limitation_is_stated() {
        let r = UnsupportedReason::NoPerZoneEffects;
        assert!(r.text().contains("ONE effect"));
        assert!(r.text().contains("not possible"));
    }

    #[test]
    fn labels_are_unique() {
        let mut labels: Vec<&str> = EFFECTS.iter().map(|e| e.label).collect();
        let n = labels.len();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), n);
    }
}
