# loq-rgb — usage guide

## Quick start

```sh
# 1. install the udev rule (one-time, root)
sudo loq-rgb-cli udev && sudo udevadm control --reload-rules && sudo udevadm trigger

# 2. check the controller is seen
loq-rgb-cli detect          # → LENOVO LOQ 15IRX9 (83DV) · Lenovo LOQ (2024) controller 048d:c993

# 3. launch the GUI
loq-rgb
```

## Fn+Space profile cycling

`loq-rgb-cli listen-hotkeys` watches for Fn+Space and cycles through your
saved profiles:

1. Run it once (`loq-rgb-cli listen-hotkeys`), or make it permanent at login:
   `cp install/loq-rgb-autostart.desktop ~/.config/autostart/`.
2. Save some profiles first (GUI "Save as…" or `loq-rgb-cli apply --save NAME`).
3. Press **Fn+Space** to advance: profiles in alphabetical order, then a full
   **Off** step, then back to the first profile.

The listener also applies the active profile once when it starts (so it
doubles as apply-at-login). "Off" is a reserved profile name that is added
automatically — it appears in the profile list and is always the final cycle
step. The input node it reads ("Ideapad extra buttons") carries only Fn-combo
keys, never typed characters.

## The GUI

**Zones (left panel).** Four swatches map to the four keyboard zones,
left → right. Click a swatch (or a band on the keyboard preview) to edit that
zone. When the effect is Static or Breathing you get a full colour editor:
hue strip + saturation/value square + hex field, and a "Save to palette"
button for custom colours. Editing one zone never resets the others — every
change re-sends the full config built from what you currently see.

**Effects (centre).** One global effect drives the whole keyboard; this is a
hardware fact, stated in the UI. When you pick Wave ←/→ or Smooth flow the
zone colour editors disable with the reason: those effects use fixed
firmware palettes and ignore zone colours. Speed (1–4) applies to animated
effects; brightness is Low/High.

**Profiles.** A profile stores the entire keyboard configuration (effect +
speed + brightness + four zone colours). Switch profiles from the dropdown;
"Save current as this profile" overwrites the active profile; "Save as…"
creates a new one.

**Status bar (bottom).** Shows when the last config was sent to hardware,
an "Apply now" button, and "Read state from keyboard" — where the controller
supports it, prints what it currently reports and marks ✓/✗ against your
applied config (✗ means something else changed it, e.g. an Fn-key cycle).
On the LOQ 15IRX9 the controller does not expose state readback (verified);
the app says so and tracks the last-applied configuration instead.

**Brightness.** Low/High. Verified: this firmware ignores the brightness
byte, so Low is emulated by dimming the zone colours (×0.6) — which affects
static and breathing only. The GUI shows a note when Low is selected.

If the controller is missing the top bar shows why, with the fix hint
(usually: install the udev rule). The app retries automatically.

## CLI examples

```sh
# whole-keyboard static: zone colours left→right
loq-rgb-cli apply --effect static --colors ff0000,00ff00,0000ff,ffffff

# fewer colours repeat the last one across remaining zones
loq-rgb-cli apply --colors ff8000

# breathing, slow, high brightness
loq-rgb-cli apply --effect breath --speed 1 --brightness 2 --colors "#1a2b3c,#4d5e6f"

# waves are multicolour: the four zone colours ARE the wave's palette
loq-rgb-cli apply --effect wave-right --speed 4 --colors ff0000,ff8800,ffee00,00ffcc

# rainbow waves use an automatic spectrum palette
loq-rgb-cli apply --effect rainbow-left --speed 3

# smooth flow
loq-rgb-cli apply --effect smooth

# HOST-RENDERED colour flow: the full colour wheel flows continuously.
# Animates while the GUI or `listen-hotkeys` runs.
loq-rgb-cli apply --effect flow-right --speed 3
loq-rgb-cli apply --effect flow-left

loq-rgb-cli apply --effect off

# profiles
loq-rgb-cli apply --effect static --colors ff0000 --save Gaming   # save + activate
loq-rgb-cli apply --profile Gaming                                # switch + apply
loq-rgb-cli profiles                                               # list
loq-rgb-cli delete-profile Gaming --yes                            # delete (permanent)
loq-rgb-cli apply                                                  # re-apply active profile

# state & diagnostics
loq-rgb-cli status
loq-rgb-cli dump-packet --effect static --colors ff0000   # raw 33-byte packet, hex
```

## Configuration file

`~/.config/loq-rgb/config.json` (or `$XDG_CONFIG_HOME/loq-rgb/config.json`).
Human-editable:

```json
{
  "version": 1,
  "active_profile": "Gaming",
  "profiles": {
    "Default": {
      "effect": "static",
      "speed": 2,
      "brightness": 2,
      "zones": ["#ffffff", "#ffffff", "#ffffff", "#ffffff"]
    },
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

Effect names: `off`, `static`, `breath`, `wave-left`, `wave-right`,
`rainbow-left`, `rainbow-right`, `flow-left`, `flow-right`, `smooth`.
Invalid values are clamped on load; a corrupt file is backed up
(`config.json.corrupt-<ts>`) and defaults restored — never silently deleted.
Writes are atomic (temp file + rename). The reserved `Off` profile ends the
Fn+Space cycle and cannot be deleted; deleting the active profile moves the
active selection to another profile without changing the lighting.

## Apply at login

The Fn+Space listener (`loq-rgb-cli listen-hotkeys`) already applies the
active profile when it starts:

```sh
cp /usr/share/loq-rgb/loq-rgb-autostart.desktop ~/.config/autostart/
```

or call `loq-rgb-cli apply` from your WM/DE autostart if you only want the
one-shot apply. The CLI exits cleanly if the controller is absent, so a
desktop without the hardware is unaffected.

## Uninstalling

```sh
sudo rm /usr/lib/udev/rules.d/60-loq-rgb.rules   # or /etc/udev/rules.d/
sudo udevadm control --reload-rules
rm -rf ~/.config/loq-rgb
```
