# loq-rgb — Lenovo LOQ keyboard RGB controller for Linux

A hardware-grounded RGB controller for the **4-zone ITE keyboard lighting** in
Lenovo **LOQ**, **Legion** and **IdeaPad Gaming** laptops (controller USB
`048d:c993` = LOQ 2024, verified on a **LOQ 15IRX9 / 83DV**).

Two programs share one codebase:

- `loq-rgb` — desktop GUI (egui): 4-zone colour editing with a full HSV colour
  picker, effect/speed/brightness control, profiles, saved colours, live
  readback and honest hardware-limitation panels.
- `loq-rgb-cli` — scripting/headless control and diagnostics.

Everything is honest by design: **only configurations the firmware can
actually represent are offered.** The keyboard applies ONE global effect; the
four zones differ only in colour (for static/breathing). Per-zone effect
mixing is not possible on this hardware and the UI says so instead of faking
it.

## What works

| Feature | Hardware | In app |
| --- | --- | --- |
| 4 independent zone colours (static/breathing) | ✅ | ✅ full colour picker per zone |
| Global effects: Off, Static, Breathing, Wave ←/→, Rainbow wave ←/→, Smooth flow | ✅ | ✅ (each visually verified on LOQ 15IRX9) |
| **Colour flow ←/→** (full colour wheel flowing as a continuous gradient) | ❌ firmware | ✅ **host-rendered**: the firmware has no such effect (all undocumented codes probed & inert), so this app draws it — real frames written ~25/s while the GUI or `listen-hotkeys` runs |
| Wave palette: the wave band renders from your four zone colours | ✅ | ✅ (verified on hardware) |
| Wave direction, speed 1–4 | ✅ | ✅ |
| Brightness Low/High | ⚠️ byte ignored | ✅ emulated: Low dims colour bytes ×0.6 (static/breath/wave/flow), verified |
| Reading the live state back from the controller | ⚠️ not on this unit | app tracks the last applied config instead; readback supported where the controller provides it |
| Per-zone mixed effects (zone 1 breath, zone 2 static) | ❌ firmware | shown as unsupported with reason |
| Per-key RGB | ❌ | not offered |
| Zone colours for rainbow wave/smooth/colour flow | ❌ | colour pickers disabled with reason |

## Install on Arch Linux

Dependencies for building: `rust`, `cargo`, `pkg-config`, `libusb`, and the
usual eframe build/runtime libs (`libxkbcommon`, `wayland`, X11 libs if you
run X11). Then:

```sh
cargo build --release
sudo install -Dm755 target/release/loq-rgb /usr/local/bin/loq-rgb
sudo install -Dm755 target/release/loq-rgb-cli /usr/local/bin/loq-rgb-cli
```

Or build a proper package from the provided PKGBUILD:

```sh
cd install   # place a source tarball next to it, or adjust source=
makepkg -si
```

### One-time device permission (required)

The lighting controller's hidraw node is root-only by default:

```sh
sudo loq-rgb-cli udev          # writes /etc/udev/rules.d/60-loq-rgb.rules
sudo udevadm control --reload-rules
sudo udevadm trigger
```

(`loq-rgb-cli udev --print` shows the rule if you prefer to install it by
hand. The packaged rule also ships in `/usr/lib/udev/rules.d/`.)

## Usage

```sh
loq-rgb                  # GUI

# CLI
loq-rgb-cli detect                              # show detected controller
loq-rgb-cli apply                               # re-apply the active profile
loq-rgb-cli apply --effect static --colors ff0000,00ff00,0000ff,ffffff
loq-rgb-cli apply --effect breath --speed 3 --brightness 2 --colors "#112233"
loq-rgb-cli apply --effect wave-right --speed 4
loq-rgb-cli apply --effect smooth
loq-rgb-cli apply --save "My profile"           # save the flag-built config
loq-rgb-cli apply --profile "My profile"        # switch + apply a profile
loq-rgb-cli profiles                            # list profiles
loq-rgb-cli status                              # read state from the keyboard
loq-rgb-cli dump-packet --effect static --colors ff0000  # debug: raw bytes
```

Config lives in `$XDG_CONFIG_HOME/loq-rgb/config.json` (`~/.config/loq-rgb/`
by default): named profiles (whole keyboard configuration), the active
profile and your saved colour palette. Writes are atomic.

### Fn+Space profile cycling (and apply at login)

Verified on the LOQ 15IRX9: **Fn+Space reaches the OS** as a key event
(`KEY_KBDILLUMUP`, code 240, on the "Ideapad extra buttons" input node) and
the firmware does not cycle natively while software control is active — so a
small listener can own the key. Run it and Fn+Space cycles through your
saved profiles, **ending with a full Off step**, then wraps back to the first
profile:

```sh
loq-rgb-cli listen-hotkeys
```

To make it permanent at login:

```sh
cp /usr/share/loq-rgb/loq-rgb-autostart.desktop ~/.config/autostart/
```

(or copy `install/loq-rgb-autostart.desktop` when building from source). The
listener also applies the active profile once at startup. Notes:

- Cycle order is alphabetical, with a reserved profile named **"Off"** as the
  final step (added automatically). One more Fn+Space press past Off turns
  the lights back on.
- The input node carries only Fn-combo keys — never typed characters — and
  the udev rule grants read-only access to it.
- Re-run `loq-rgb-cli udev` after upgrading: it installs both the controller
  and the Fn+Space rules.

## Hardware protocol (summary)

The controller is an ITE Tech USB HID device (VID `048d`, "ITE Device(8295)")
exposing a vendor interface (usage page `0xff89`, usage `0x00cc`). A single
33-byte feature report (Report ID `0xCC`) holds the whole keyboard state:

```
CC 16 <effect> <speed> <brightness> <zone1 RGB> <zone2 RGB> <zone3 RGB>
<zone4 RGB> 00 <wave-right> <wave-left> 0…
```

Effect codes: `0x01` static, `0x03` breath, `0x04` wave (with a direction
flag byte; verified on hardware to render from the four supplied zone
colours), `0x06` smooth flow; `0x00` + brightness 0 is off. Rainbow waves
are wave code `0x04` with an automatic spectrum palette; the host-rendered
Colour flow effect is drawn in software as a stream of static frames (not a
firmware code). (State readback is not provided by this controller — see
`docs/hardware-report.md`.)

## Development

```sh
cargo test          # unit + integration tests (mock hardware)
cargo clippy --all-targets   # must be warning-free
cargo fmt --check
```

Hardware validation is an interactive, opt-in suite (visual confirmation is
the only proof a keyboard actually changed):

```sh
cargo test --test hardware -- --ignored --nocapture
```

## Limitations (not bugs)

- One global effect; zone colours are per-zone but the effect is not.
- **Colour flow is host-rendered**: it only animates while the GUI or the
  `listen-hotkeys` daemon is running, and the last frame stays lit when they
  close. Transitions between the four physical zones are limited by 4-zone
  hardware (each zone is one solid colour).
- Only ONE program should write lighting at a time (e.g. don't run two
  daemons). Multiple writers cause visible fighting/flicker.
- **Lenovo EC Fn-combos interrupt lighting** (verified: toggling the camera
  kill-switch changes the keyboard effect from inside the EC). Colour flow
  re-writes frames every ~40 ms so it recovers quickly; firmware effects may
  need a re-apply after such a toggle.
- Wave/rainbow wave render from the four zone-colour palette; smooth flow
  drives its own single colour — zone colours are ignored by those.
- The brightness byte is ignored by this firmware (verified); Low is
  emulated by dimming colour bytes ×0.6, which only affects effects whose
  colours we control — the GUI says so next to the control.
- State readback (GET) is not available on this unit: the controller answers
  with an 11-byte identity report. The app tracks the last-applied config
  and surfaces this honestly. Readback code remains for controllers that
  provide it.
- The ITE firmware may turn the backlight off after long idle on some models;
  if that happens, re-apply with `loq-rgb-cli apply` (or the GUI's Apply
  button).

## Credits / prior art

The protocol was cross-checked against LegionAura, legion-keyboard-custom
(tested on the LOQ 15IRX9), L5P-Keyboard-RGB, Lenovo Legion Toolkit (Windows)
and the community script on kaveh.page. This project is independent and not
affiliated with Lenovo.
