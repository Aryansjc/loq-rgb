# Hardware report — LOQ 15IRX9 keyboard RGB

*Evidence collected during Phase 1 (planning) and updated during development.*

## Detected hardware (this machine)

| Item | Value | Source |
| --- | --- | --- |
| Vendor / board | LENOVO / LNVNB161216 | `/sys/class/dmi/id/board_vendor`, `board_name` |
| Product | LOQ 15IRX9 (model 83DV, BIOS NECN50WW) | DMI |
| Lighting controller | USB `048d:c993` "ITE Tech. Inc. ITE Device(8295)", 2 HID interfaces | USB sysfs (port 1-7), `/sys/class/hidraw/*/device/uevent` |
| Key matrix | separate devices: ITE `048d:c996` "ITE Device(8176)" and DaKai `1008:2020` "USB KEYBORD" | USB sysfs |
| Driver | `hid-generic` (no kernel driver claims the lighting controller; no LED-class node exists) | hidraw uevent, `/sys/class/leds` |
| Kernel | 7.2.2-1-cachyos | `uname -a` |

## Controller identity

`048d:c993` is the LOQ 2024 4-zone controller in the ITE "8295" family. PID
tables in LegionAura (`devices.json`) and legion-keyboard-custom
(`driver/src/lib.rs`, usage tuple `ff89/00cc`) independently map it to "2024
Lenovo LOQ"; legion-keyboard-custom's README lists **LOQ 15IRX9 as tested
hardware**.

The two HID interfaces on `048d:c993` both enumerate as hidraw nodes
(hidraw2/hidraw3 on this machine). The lighting endpoint is the vendor
interface (usage page `0xff89`, usage `0x00cc`); the app prefers that
interface and falls back to the first `048d:c993` node.

## Protocol (CC/16)

One 33-byte HID **feature report** (Report ID `0xCC`) describes the entire
keyboard state:

| Offset | Bytes | Meaning |
| --- | --- | --- |
| 0 | 1 | Report ID `0xCC` |
| 1 | 1 | Magic `0x16` |
| 2 | 1 | Effect: `0x00` off, `0x01` static, `0x03` breath, `0x04` wave, `0x06` smooth flow |
| 3 | 1 | Speed 1–4 |
| 4 | 1 | Brightness 1–2 (`0` only in the off state) |
| 5–16 | 12 | Zone 1–4 RGB (3 bytes each) — only read for static/breath |
| 17 | 1 | Reserved |
| 18 | 1 | Wave direction flag (right) |
| 19 | 1 | Wave direction flag (left) |
| 20–32 | 13 | Zero padding |

Read-back uses the same report id (`GET`); the controller returns the live
effect/speed/brightness/zone bytes.

Cross-checked against: LegionAura (C++, libusb `0x21/0x09/0x03CC`),
legion-keyboard-custom (Rust, hidapi `send_feature_report`, 33-byte buffer),
Kaveh's community script, and Lenovo Legion Toolkit's Windows
`LENOVO_RGB_KEYBOARD_STATE`. The Linux tools agree byte-for-byte on the layout
above. (Legion Toolkit's Windows struct packs 13 unused bytes at a different
offset; it was not treated as authoritative for Linux byte positions — the
live round trip on this unit settles any ambiguity.)

## Capability matrix

| Feature | Supported by hardware | Can be implemented | Notes |
| --- | --- | --- | --- |
| 4 independent colour zones | ✅ | ✅ | static + breath; 8-bit RGB per zone |
| Different colours on different zones at once | ✅ | ✅ | e.g. z1 red, z4 blue |
| Per-zone independent/mixed effects (z1 breath + z2 static) | ❌ | ❌ (not faked) | single global effect byte |
| Static | ✅ | ✅ | `0x01` — visually verified |
| Breathing (all zones in sync) | ✅ | ✅ | `0x03` — visually verified |
| Firmware wave band (`0x04`) | ✅ exists | ❌ deliberately not exposed | steps one colour at a time; replaced by the colour wave below |
| **Colour wave left / right** (continuous multicolour) | ❌ firmware | ✅ host-rendered | no firmware code exists (all undocumented codes probed & inert); the app draws ~25 static frames/s. A background animator keeps it running after the GUI closes |
| Smooth flow | ✅ | ✅ | `0x06` — whole keyboard, one colour shifting over time (verified) |
| Effect codes `0x02`, `0x05`, `0x07`–`0x0A` | ❌ inert | probe-only | interactive probe test provided; not exposed in UI |
| Full 24-bit colour | ✅ | ✅ | free picker, not preset-only |
| Brightness byte | ❌ **ignored by firmware** | ✅ via emulation | Low = host-side dimming ×0.6 of the colour bytes we control (static/breath/colour wave); visually verified; smooth flow cannot be dimmed and is labelled so |
| Speed | 1–4 | ✅ | animated effects |
| Off | ✅ | ✅ | `0x00` + brightness 0 — visually verified |
| Read lighting state back | ❌ on this unit | read-only app fallback | CC GET returns an 11-byte identity report (contains PID), not lighting state; app tracks last-applied config instead |
| Per-key RGB | ❌ | ❌ | 4-zone only |
| EC persistence across reboot/suspend | untested | re-apply at boot | autostart provided |

## Verification log (interactive, on this LOQ 15IRX9)

All checks were visual confirmations by the machine's user — an API call
returning success was never treated as proof:

1. Static 4 colours (red/green/blue/white) → zones 1–4 lit left→right exactly as configured. ✅
2. Breathing with four zone colours → all zones pulse in sync. ✅
3. Firmware wave band (`0x04`): sweeps left/right and renders from the four zone-colour bytes, but steps one colour at a time. Deliberately not exposed — replaced by the colour wave. ✅ (behaviour identified)
4. Smooth flow → one colour shifting across the whole keyboard. ✅
5. Brightness byte: Low == High visually → byte ignored by this firmware. Emulated dimming (×0.6 zone colours) visibly darker. ✅
6. Off (effect `0x00`) → backlight completely off. ✅
7. Readback: CC GET returns 11-byte identity data (contains `93 c9` = the PID) — no lighting-state readback on this unit.
8. Fn+Space reaches the OS: EV_KEY code 240 (`KEY_KBDILLUMUP`) on the "Ideapad extra buttons" input node (VPC2004 ACPI). ✅
9. While software control is active, Fn+Space does NOT cycle lighting natively (pressing it changed nothing) — so a listener cleanly owns the key. Live-tested: cycling saved profiles ends with a full Off step and wraps. ✅
10. Undocumented effect codes 0x02, 0x05, 0x07–0x0A are all inert — no hidden rainbow/flow effect exists in this firmware. ✅
11. **The wave engine renders from the zone-colour bytes**: sending red/green/blue/white made the firmware wave multicoloured (earlier tools zero those bytes, which is why waves looked plain). ✅
12. Host-rendered colour wave (~25 static frames/s) animates smoothly with exactly one writer. Multiple simultaneous writers cause visible fighting/flicker — now prevented by the single-writer lock. ✅
13. Lenovo EC Fn-combos interrupt lighting from inside the EC (verified: toggling the camera kill-switch changes the keyboard effect); the colour wave re-writes frames every ~40 ms and recovers. ✅
14. Final colour-wave look, tuned live: **full-spectrum mode at speed 2** reads as the intended smooth moving rainbow. Palette mode (user's four colours as blocks) is coarser — both remain available. Daemon cost: 0.1% CPU, 3.7 MB RSS. ✅
15. **The wave survives closing the GUI**: selecting a colour wave starts a background animator that holds the writer lock; closing the window leaves it running and the keyboard keeps animating. Verified live (animator pid alive with the lock, window gone, lighting still moving). ✅

## Honest answers to the design questions

- **"Zone 1 static, zone 4 rainbow":** zone colours exist, but a rainbow is a
  whole-keyboard effect (`smooth`/wave) with a fixed palette — it cannot be
  confined to one zone. The app shows the four zone colours for static/breath
  and clearly marks animated effects as whole-keyboard with zone colours not
  sent.
- **Mixed effects across zones:** impossible at the protocol level (one effect
  byte). The GUI explains this instead of pretending.
- **Unsupported features:** never shown as available; where a control would
  be meaningless (zone colours under wave, speed under static, direction for
  non-wave effects) it is disabled with the hardware reason.

## Known unknowns — resolved by hardware testing

| Question | Result |
| --- | --- |
| Which HID interface answers the CC report? | hidraw2 (interface 0). hidraw3 (interface 1) rejects feature reads ("broken pipe"). |
| Off encoding | `0x00` + brightness 0 turns the backlight off (verified). |
| Wave flag visual semantics | byte 18 = sweep right, byte 19 = sweep left (verified). |
| Brightness byte honoured? | No — ignored; Low is emulated by host-side colour dimming. |
| Effect codes `0x02`/`0x05` | not tested yet — probe test in `tests/hardware.rs` (run with `--ignored`). |
| Idle auto-off behaviour | not observed during the test session; re-apply is the recovery path.
