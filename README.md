# loq-rgb — Lenovo LOQ keyboard RGB controller for Linux

A hardware-grounded RGB lighting controller for the **4-zone ITE keyboard
lighting** found in Lenovo **LOQ / Legion / IdeaPad Gaming** laptops. It talks
directly to the controller over USB HID, so it works without Windows-only
software. Developed and verified on a **Lenovo LOQ 15IRX9 (model 83DV)**.

Two programs share one codebase:

- `loq-rgb` — desktop GUI (egui). Four zone colours, a full HSV colour
  picker, every supported effect, profiles, brightness and speed.
- `loq-rgb-cli` — terminal control, scripting, diagnostics.

Everything is honest by design: **only configurations the hardware can
actually represent are offered.** The keyboard firmware applies ONE global
effect — the four zones are independent only in their colours. Per-zone
effect mixing (zone 1 breathing while zone 2 is static) is physically
impossible on this controller and is clearly marked as unsupported rather
than faked.

---

## Does my laptop work?

The app needs the ITE 4-zone RGB keyboard controller on the USB bus. Check in
one command:

```sh
loq-rgb-cli detect
```

If it prints something like `Lenovo LOQ 15IRX9 · Lenovo LOQ (2024) controller
048d:c993`, you are supported.

**Verified on:** LOQ 15IRX9 / 83DV with controller `048d:c993`.

**Likely supported** (same ITE 8295 controller family, same protocol, not yet
verified by the project): `048d:c983` (LOQ 2023), and the Legion /
IdeaPad Gaming 4-zone controllers `048d:c955 … c995`. The full list is in
`src/devices.rs`.

**Not supported:** laptops with a plain white backlight, per-key RGB
keyboards (Legion 7-style spectrum boards), and anything without a
`048d:c9xx` controller. The app says "no supported controller found" and
explains what to check — it never guesses.

---

## Install on Arch Linux

### Dependencies

Building requires: `rust`, `cargo`, `pkg-config`, `libusb`, and the usual
GUI libraries used by egui/eframe (`libxkbcommon`, `wayland`, and X11 libs if
you run X11). On Arch:

```sh
sudo pacman -S --needed rust pkg-config libusb
```

### Build and install

```sh
git clone https://github.com/Aryansjc/loq-rgb
cd loq-rgb
cargo build --release

sudo install -Dm755 target/release/loq-rgb      /usr/local/bin/loq-rgb
sudo install -Dm755 target/release/loq-rgb-cli   /usr/local/bin/loq-rgb-cli
```

Or install the desktop files and udev rule from `install/`:

```sh
sudo install -Dm644 install/loq-rgb.desktop /usr/share/applications/loq-rgb.desktop
sudo install -Dm644 install/60-loq-rgb.rules /usr/lib/udev/rules.d/60-loq-rgb.rules
sudo udevadm control --reload-rules && sudo udevadm trigger
```

A PKGBUILD is provided in `install/` if you prefer `makepkg`.

### One-time device permission (required)

The lighting controller's hidraw node is root-only by default. Grant your
user access once:

```sh
sudo loq-rgb-cli udev          # writes /etc/udev/rules.d/60-loq-rgb.rules
sudo udevadm control --reload-rules
sudo udevadm trigger
```

(`loq-rgb-cli udev --print` shows the rule so you can install it by hand if
you prefer.) The rule also grants read access to the Fn+Space key node.

### Update

```sh
git pull            # or re-download the release
cargo build --release
sudo install -Dm755 target/release/loq-rgb      /usr/local/bin/loq-rgb
sudo install -Dm755 target/release/loq-rgb-cli   /usr/local/bin/loq-rgb-cli
sudo loq-rgb-cli udev     # refresh the udev rule, then reload/trigger udev
```

### Uninstall

```sh
sudo rm /usr/local/bin/loq-rgb /usr/local/bin/loq-rgb-cli
sudo rm /usr/lib/udev/rules.d/60-loq-rgb.rules /etc/udev/rules.d/60-loq-rgb.rules
sudo udevadm control --reload-rules
rm -f ~/.config/autostart/loq-rgb-autostart.desktop
rm -rf ~/.config/loq-rgb
```

---

## Using the GUI

```sh
loq-rgb
```

- **Effect** (centre column): one global effect drives the whole keyboard —
  a hardware fact, stated in the UI. Choose from Off, Static, Breathing,
  **Colour wave ←/→** (the continuous multicolour wave) and Smooth flow.
- **Zones** (left panel): four swatches map to the four keyboard zones, left →
  right. Click one to edit it. When the effect uses zone colours (Static,
  Breathing, Colour wave) you get the full colour editor: hue strip +
  saturation/value square + hex field, and "Save to palette" for colours you
  want to reuse. Editing one zone never resets the others.
- **Colour wave palette note:** the wave uses your four zone colours as its
  moving colour blocks. If all four zone colours are identical (the untouched
  default), it automatically shows the full colour spectrum instead — pick
  different zone colours to make your own palette wave.
- **Profiles** (centre, below the effect list): a profile stores the complete
  keyboard configuration (effect + speed + brightness + four zone colours).
  Use the dropdown to switch profiles, "Save current as this profile",
  "Save as…", **"Rename…"** and **"Delete profile"** (with confirmation;
  deleting the active profile moves the active selection to another profile
  without changing the lighting). **Edits are saved to the active profile
  automatically**, so what you see is what the CLI and the next start use.
- **Status bar** (bottom): shows when the last configuration was sent to the
  hardware, an "Apply now" button, a "Read state from keyboard" attempt, and
  any errors with hints.
- Missing controller? The top bar says why and what to do (usually: install
  the udev rule). The app retries automatically.

### Keeping the colour wave running

The colour wave is rendered by software (no firmware effect produces it), so
something must keep writing frames. The first time you select a colour wave,
the app **starts a small background animator** — it keeps the wave moving
even after you close the window, and also gives you Fn+Space profile cycling.
The status bar tells you when the animator is in charge. To stop it:

```sh
pkill -f 'loq-rgb-cli listen-hotkeys'
```

Only one program may write to the keyboard at a time; the app enforces this
with a lock, so you cannot accidentally run two animators.

### Colour picker

Any colour is selectable: drag in the hue strip and the saturation/value
square, or type a hex value. Saved colours appear under the picker;
right-click one to remove it.

### What you can't do (hardware limits, shown honestly)

- Per-zone effect mixing (e.g. breathing on zone 1, static on zone 2) — one
  global effect only.
- Per-key RGB.
- Zone colours for Smooth flow — that picker is disabled with the reason
  shown (Smooth flow drives its own single colour).
- The firmware's stepped wave effect is not exposed: it shows one colour at a
  time, which is exactly what the Colour wave replaces. Saved profiles that
  used it load as the Colour wave.

---

## Using the CLI

```sh
# Detection and diagnostics
loq-rgb-cli detect            # is my controller supported?
loq-rgb-cli diagnose          # full report for support tickets
loq-rgb-cli status            # what the controller reports (see limitations)

# Set lighting
loq-rgb-cli apply --effect static --colors ff0000,00ff00,0000ff,ffffff
loq-rgb-cli apply --effect breath --speed 1 --brightness 2 --colors "#1a2b3c,#4d5e6f"
loq-rgb-cli apply --effect flow-right --speed 3     # continuous colour wave →
loq-rgb-cli apply --effect flow-left  --speed 2     # continuous colour wave ←
loq-rgb-cli apply --effect smooth                   # one colour shifting
loq-rgb-cli apply --effect off

# Profiles
loq-rgb-cli apply --effect static --colors ff0000 --save "Gaming"   # save + activate
loq-rgb-cli apply --profile "Gaming"                                # switch + apply
loq-rgb-cli profiles                                                 # list profiles
loq-rgb-cli rename-profile "Gaming" "Gaming RGB" --yes
loq-rgb-cli delete-profile "Gaming" --yes
loq-rgb-cli apply                                                    # re-apply active profile

# System
loq-rgb-cli udev                # install the device permission rule
loq-rgb-cli udev --print        # print the rule without installing
loq-rgb-cli listen-hotkeys      # background animator + Fn+Space cycling
```

A single colour fills all zones; several colours assign zone 1, zone 2, …
left to right (fewer than four repeats the last colour). The old names
`wave-left`, `wave-right`, `rainbow-left`, `rainbow-right` are still accepted
and map onto the colour wave, so existing scripts keep working.

### Effect and colour semantics per effect

| Effect (`--effect`) | Zone colours | Direction | Speed | Notes |
| --- | --- | --- | --- | --- |
| `off` | — | — | — | backlight off |
| `static` | per-zone colours | — | — | solid |
| `breath` | per-zone colours | — | yes | all zones pulse in sync |
| `flow-left` / `flow-right` | the wave's colour blocks, or the full spectrum when all four zone colours are identical | selectable | yes | **continuous colour wave** (host-rendered, see below) |
| `smooth` | not used | — | yes | whole keyboard, one colour shifting |

Speed is `1` (slowest) .. `4` — it sets the wave's tempo. Brightness is `1`
(Low) / `2` (High) — on the verified hardware the brightness *byte* is
ignored, so Low is emulated by dimming the colour bytes the app controls
(static/breathing/colour wave); Smooth flow cannot be dimmed and the UI says
so.

### The Colour wave is host-rendered (read this)

No firmware effect produces a continuous moving multi-colour wave (verified
by probing every undocumented effect code). The Colour wave therefore renders
in software: the app writes real frames (~25 per second) to the controller.
Consequences:

- Something must be running to write those frames, so the app keeps a small
  **background animator** alive. Selecting a colour wave starts it
  automatically, and it continues after you close the GUI window. If it is
  not running (e.g. you stopped it), the wave holds its last frame instead of
  moving — start it again with `loq-rgb-cli listen-hotkeys`.
- The four zones are four solid colour regions — a smooth *spatial* gradient
  inside a zone is a hardware limit. Motion between blocks is continuous
  (no delay, no visible loop restart).
- **Only ONE lighting writer may run at a time.** A single-writer lock is
  built in: a second `listen-hotkeys` refuses to start, and the GUI detects
  the animator and leaves frame-writing to it (it prints who is in control).
  Running two writers was the cause of every "flickering/glitching" report.

---

## Fn+Space profile cycling (and apply at login)

On the verified hardware, Fn+Space reaches the OS as a key event and does
*not* cycle natively while software control is active, so a small daemon can
own the key. Start it once:

```sh
loq-rgb-cli listen-hotkeys
```

It applies the active profile at startup and then **Fn+Space cycles through
your saved profiles in alphabetical order, ends with a full Off step, and
wraps back to the first profile**.

To run it automatically at login:

```sh
cp install/loq-rgb-autostart.desktop ~/.config/autostart/
```

Notes:

- "Off" is a reserved profile name (the final cycle step, added
  automatically). It cannot be deleted or renamed.
- The Fn+Space key node only carries Fn-combo keys — never typed characters —
  and the udev rule grants read access to it.
- Lenovo's EC also reacts to some Fn combos from inside the firmware
  (verified: toggling the camera kill-switch changes the keyboard effect).
  The colour wave re-writes frames every ~40 ms and recovers by itself;
  firmware effects may need a re-apply after such a toggle.

---

## Configuration file

Everything is stored in `$XDG_CONFIG_HOME/loq-rgb/config.json`
(`~/.config/loq-rgb/config.json` by default): named profiles (whole keyboard
configurations), the active profile, and your saved colour palette. Writes
are atomic; a corrupt file is backed up (`config.json.corrupt-<ts>`) and
defaults restored — never silently destroyed.

```json
{
  "version": 1,
  "active_profile": "Gaming",
  "profiles": {
    "Gaming": {
      "effect": "breath",
      "speed": 3,
      "brightness": 2,
      "zones": ["#ff0000", "#00ff00", "#0000ff", "#ffff00"]
    }
  },
  "palette": ["#ff0000", "#00aaff"]
}
```

Effect names: `off`, `static`, `breath`, `flow-left`, `flow-right`, `smooth`
(legacy `wave-*` / `rainbow-*` names are also accepted and map onto the
colour wave).

---

## Troubleshooting

- **"No supported controller found"** → run `loq-rgb-cli detect`. If it is
  empty, your laptop does not expose a supported `048d:c9xx` controller; a
  white-only backlight is not supported.
- **"Permission denied opening the controller"** → run the one-time udev step
  above, then reconnect the keyboard or `sudo udevadm trigger`.
- **Colour wave is not moving** → the background animator is not running.
  Selecting a colour wave in the GUI starts it automatically; if you stopped
  it, start it again with `loq-rgb-cli listen-hotkeys` (autostart available),
  or reopen the GUI and pick the wave again.
- **Colour wave stopped when I closed the GUI** → that happens only if the
  background animator could not be started (the status bar says so). Run
  `loq-rgb-cli listen-hotkeys` manually and check `$XDG_RUNTIME_DIR/loq-rgb/animator.log`.
- **Lighting flickers or glitches** → exactly one writer must run. Check with
  `ps -ef | grep loq-rgb`; stop extras with
  `pkill -f 'loq-rgb-cli listen-hotkeys'`. The single-writer lock normally
  prevents this.
- **Fn+Space does nothing** → is the daemon running? Is the profile list
  non-empty? The key node needs the udev rule (see above).
- **Keyboard changes by itself** → Lenovo EC Fn-combos (e.g. the camera
  kill-switch) override lighting from inside the firmware. Re-apply the
  profile (`loq-rgb-cli apply`) or rely on the colour wave's automatic
  recovery.
- **Help / bug reports** → run `loq-rgb-cli diagnose` and include its output
  together with your laptop model and `loq-rgb-cli detect` output.

---

## Development

```sh
cargo test                      # unit + integration tests (mock hardware)
cargo clippy --all-targets      # must be warning-free
cargo fmt --check
```

Hardware validation is an interactive, opt-in suite (visual confirmation is
the only proof a keyboard actually changed):

```sh
cargo test --test hardware -- --ignored --nocapture
```

---

## Hardware protocol (summary)

The controller is an ITE Tech USB HID device (VID `048d`, "ITE
Device(8295)") exposing a vendor interface (usage page `0xff89`, usage
`0x00cc`). A single 33-byte feature report (Report ID `0xCC`) holds the whole
keyboard state:

```
CC 16 <effect> <speed> <brightness> <zone1 RGB> <zone2 RGB> <zone3 RGB>
<zone4 RGB> 00 <wave-right> <wave-left> 0…
```

Effect codes: `0x00` off, `0x01` static, `0x03` breath, `0x04` the firmware's
stepped wave (present in the protocol but not exposed by this app), `0x06`
smooth flow. The colour wave is sent as a stream of `0x01` (static) frames
whose colours advance every ~40 ms. State readback is NOT available on the
verified controller (the CC GET answers with an 11-byte identity report); the
app tracks the last-applied configuration instead. Full evidence in
`docs/hardware-report.md`.

## License

MIT — see `LICENSE`. This project is independent and not affiliated with
Lenovo.
