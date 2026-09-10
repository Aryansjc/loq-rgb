# Testing report

*Updated after the final full-suite run.*

## Summary

- **101 automated tests pass** (80 unit + 21 integration/CLI), 0 failures.
- 7 interactive hardware tests ship `#[ignore]`d (visual confirmation only).
- `cargo clippy --all-targets`: 0 warnings. `cargo fmt --check`: clean.
- Live hardware validation on this LOQ 15IRX9: 15 visual/readback checks run
  with the machine's user; all expected results confirmed except two genuine
  hardware findings (brightness byte ignored, no state readback), which were
  then implemented honestly and re-verified. Fn+Space cycling (with the
  requested Off-at-cycle-end), the continuous colour wave, and its survival
  after closing the GUI were all verified live.

## Test inventory

| Suite | Count | Covers |
| --- | --- | --- |
| `model` (unit) | 14 | hex/HSV colour maths, serde, effect codes vs protocol, **legacy wave-name migration**, zone-colour consumer policy, off colour retention, host-rendered flags |
| `packet` (unit) | 12 | golden packets per effect/off, **colour-wave frames are static packets (no wave flag)**, spectrum fallback + dimming, parse round trips, unknown-effect preservation, garbage rejection |
| `devices`/`detect`/`error`/`effects`/`controller`/hid (unit) | 16 | PID table, sysfs scan with interface-dir filtering, hints for every error variant, effect UI metadata (**one wave family only**), controller success/error bookkeeping |
| `config` (unit) | 7 | save/load round trip, default creation, corrupt-file backup+recovery, value clamping, profile ops, reserved-Off name guard |
| `hotkey` (unit) | 6 | cycle order (alphabetical + Off last), wrap past Off, reserved Off profile creation/normalisation, self-healing active selection, persistence of the Off step |
| `profiles` (unit) | 9 | safe deletion (non-active/active/last/reserved-OFF/missing, siblings untouched, restart persistence) **and safe rename** (collision, empty, reserved, active stays active, restart persistence) |
| `flow` (unit) | 11 | colour-wave engine: user-palette mode and full-spectrum mode, palette/spectrum selection, periodic seamless loop (no jump at wrap), per-frame smoothness (no flicker), direction opposition, speed/wrap, spectrum visits all hues, single dimming pass |
| `instance` (unit) | 4 | single-writer lock: exclusive acquire, second writer refused, release on drop, owner probe |
| `daemon` (unit) | 2 | animator binary resolution and log path |
| `gui` (unit) | 2 | readback/config matching incl. wave-frame ↔ static readback and effective (dimmed) colour bytes |
| `tests/controller_flow.rs` | 5 | **zone-change isolation at byte level**, every effect × every zone slot, single-global-effect enforcement, failed-apply keeps last good state, readback flow |
| `tests/persistence.rs` | 2 | profile survives restart cycles (3 simulated sessions), 20 restart cycles byte-stable |
| `tests/cli.rs` | 14 | golden `dump-packet` output, **colour-wave static frames + legacy name mapping**, invalid input rejection, mock apply reporting, profile lifecycle, delete/rename flows, bare-apply autostart path, udev rule content |
| `tests/hardware.rs` (ignored) | 7 | interactive visual suite: per-zone lighting, effect correctness, colour-wave direction, brightness, off, readback, undocumented-effect probe |

## Feature → test mapping (no untested features shipped)

- **Zone management**: `changing_one_zone_never_touches_the_others` (byte-level),
  `every_effect_on_every_zone_slot`, live hardware check (4 zones L→R).
- **Effects**: golden packets + policy tests + live visual checks of static,
  breathing, smooth flow and both colour-wave directions; the removed
  firmware wave is pinned as not exposed (`only_one_wave_family_is_exposed`),
  and legacy configs migrate (`legacy_effect_names_load_as_the_colour_wave`).
- **Colour wave continuity**: no loop jump, no per-frame flicker, all hues
  visited, direction opposition (`flow` suite, 11 tests).
- **Colours**: HSV/hex round trips, palette persistence, GUI matching logic,
  live check that picked colours hit the LEDs exactly.
- **Persistence**: `tests/persistence.rs` plus corrupt-file recovery; atomic
  write exercised by every save; GUI autosave keeps the active profile in
  sync with the hardware and the background animator.
- **Single writer**: lock tests plus a live check that a second daemon is
  refused and the animator outlives the GUI.
- **Error handling**: per-variant hints, failure injection through the mock,
  permission path verified live (before udev rule: clean hint; after: works).
- **Regression**: full suite re-run after every change (dimming, effect
  switching colour retention, CLI arg semantics, readback handling).

## Development-loop results (notable findings and fixes)

Each item was found by the Implement → Test → Critique loop, not by
inspection alone:

1. `HidBackend::open()` never selected a hidraw path — the CLI failed on real
   hardware while every mock test passed. Found during first live run; fixed;
   now covered by the live path.
2. GUI initially **zeroed zone colours when switching to wave/off** — would
   have destroyed user colours. Removed; packet builder now omits colour
   bytes instead. Covered by `model` colour-retention test.
3. GUI brightness edits bypassed the debounced apply path. Fixed.
4. egui 0.36 API differences (panels, `StrokeKind`, App trait) caught and
   corrected by compilation against the real dependency.
5. **Hardware finding:** brightness byte ignored by the firmware (Low == High
   visually). Implemented honest emulation (zone-colour dim ×0.6 for
   static/breath), labelled in the UI, verified visually.
6. **Hardware finding:** CC GET returns an 11-byte identity report on this
   unit, not lighting state. Readback is now reported as unsupported with an
   explanatory message; the app tracks last-applied config instead.
7. CLI `--save` without colours collided with the new bare-apply autostart
   path; config flags are now optional and the two intents are distinct.

## Hardware validation evidence (this LOQ 15IRX9)

All checks confirmed by the user watching the keyboard; `status` codes and
readbacks were never treated as proof:

- static four colours: zones lit red/green/blue/white left→right ✅
- breathing: sync pulse keeping zone colours ✅
- colour wave: continuous multicolour movement, both directions ✅
- colour wave keeps moving after the GUI window is closed (background animator) ✅
- only one wave family appears in the effect list ✅
- smooth flow animates ✅
- Low emulation visibly dimmer than High ✅ (raw byte had no effect)
- off encoding extinguishes the backlight ✅
- readback: 11-byte identity (contains PID `c993`) — no state readback ✅
- write path works through the udev-rule-granted hidraw node without root ✅

## Regression status

- Full suite re-run green after every change in this session.
- Clippy and rustfmt clean at the end of each milestone.
- The interactive hardware checks can be re-run any time:
  `cargo test --test hardware -- --ignored --nocapture` (requires the udev
  rule and a person watching the keyboard).
