# Spectrum RGB zones — Legion Pro 7 Gen 10 (83RU)

Verified live on hardware (`048d:c197`, ITE 8258) via
[legion-spectrum-control](https://github.com/alstergee/legion-spectrum-control)
protocol (960-byte HID feature reports, report ID `0x07`).

Lighting profiles store **per-zone** static/effects. Multi-effect packets work:
send several `build_effect` blobs in one `0xCB` EFFECT_CHANGE on the **active**
profile.

## Zones (verified 2026-07-21)

| Zone | Keycodes | Notes |
|------|----------|--------|
| Keyboard | `KEYBOARD_KEYS` (101 keys, see spectrum-ctl) | Per-key matrix |
| Front bar | `0x01f5`–`0x01fe` (10 LEDs) | Side + front chassis accents |
| Back / rear bar | `0x03e9`–`0x03fa` (18 LEDs) | Rear accent strip |
| Lid logo | `0x05dd` | LEGION lid LED (also `0xA6` logo on/off) |
| All lights | `0x0065` | Special “everything” code — good for global effects |

`PERIMETER` in spectrum-ctl = rear **plus** front. For independent front/back
colors, split those lists (do not use a single perimeter blob).

## Ops that matter

- `0xCE` / `0xCD` — brightness set/get (0–9). **0 = visually off.**
- `0xCB` — apply effect description to a profile
- `0xCA` / `0xC8` — get/set active profile (0–6)
- `0xA6` / `0xA5` — logo LED on/off (independent of colour effect)

## Per-key (verified on German / QWERTZ 83RU)

Do **not** trust legion-spectrum-control ANSI `KEY_NAMES` blindly on this DE board.
Probe with max-saturation colours on a **dark-grey** keyboard base (never red-on-red).

### Confirmed (code → physical DE)

| Code | Physical (DE) | ANSI map was |
|------|---------------|--------------|
| `0x0042` | Q | q |
| `0x0004` | F3 | f3 |
| `0x001e` | 8 | 8 |
| `0x0012` | End | end |
| `0x0028` | Keypad \* | nummul |
| `0x0082` | **Y** | x (QWERTZ) |
| `0x0083` | **X** | c |
| `0x0021` | **ß / ?** | minus |
| `0x009d` | **Up arrow** | down |
| `0x008e` | **Numpad 1** | up |
| `0x000e` | **Einfügen (Insert)** | prtsc *(swapped)* |
| `0x000f` | **Druck (PrtSc)** | insert *(swapped)* |
| `0x0043` | **W** | w |
| `0x0044` | **E** | e |
| `0x0045` | **R** | r |
| `0x006d` | **A** | a |
| `0x006e` | **S** | s |
| `0x0058` | **D** | d |
| `0x006a` | **Shift** (left?) | z *(wrong — not Z)* |
| `0x0098` | **Space** | space |
| `0x0077` | **Enter** | enter |
| `0x0001` | **Esc** | esc |
| `0x0046` | **T** | t |
| `0x0047` | **Z** | y *(QWERTZ — ANSI y is DE Z)* |
| `0x0048` | **U** | u |
| `0x0049` | **I** | i |
| `0x004a` | **O** | o |
| `0x004b` | **P** | p |
| `0x0059` | **F** | f |
| `0x005a` | **G** | g |
| `0x0071` | **H** | h |
| `0x0072` | **J** | j |
| `0x005b` | **K** | k |
| `0x005c` | **L** | l |
| `0x009c` | **Left arrow** | left |
| `0x0075` | **.** (period) | slash *(DE legend)* |
| `0x006f` | **C** | v *(row shifted vs ANSI)* |
| `0x0070` | **V** | b |
| `0x0087` | **M** | n |
| `0x0088` | **N** | m |
| `0x0074` | **,** (comma) | period *(not B)* |
| `0x0073` | **M** | comma *(overrides earlier M=`0x0087`)* |
| `0x0011` | **Pos1 (Home)** | home |
| `0x0040` | **Tab** | tab |
| `0x0055` | **Caps** *(user said “shift” — confirm)* | caps |
| `0x005d` | **Ö** | semicolon |
| `0x005f` | **Ä** | quote |
| `0x008d` | **Right Shift** | rshift |
| `0x009f` | **Down arrow** *(“bottom”)* | right |
| `0x0038` | **Backspace** | backspace |
| `0x0087` | **B** | n *(was wrongly listed as M earlier)* |
| `0x0022` | **´ / \`** (DE accent key) | equals *(US =)* |
| `0x0019` | **3** | 3 |
| `0x001a` | **4** | 4 |
| `0x001b` | **5** | 5 |
| `0x001c` | **6** | 6 |
| `0x001d` | **7** | 7 |
| `0x001f` | **9** | 9 |
| `0x0020` | **0** | 0 |
| `0x0017` | **1** | 1 |
| `0x0018` | **2** | 2 |
| `0x0016` | **^** (dead circumflex) | tilde |
| `0x0002` | **F1** | f1 |
| `0x0003` | **F2** | f2 |
| `0x0005` | **F4** | f4 |
| `0x0006` | **F5** | f5 |
| `0x0007` | **F6** | f6 |
| `0x0008` | **F7** | f7 |
| `0x0009` | **F8** | f8 |
| `0x000a` | **F9** | f9 |
| `0x000b` | **F10** | f10 |
| `0x000c` | **F11** | f11 |
| `0x000d` | **F12** | f12 |
| `0x0010` | **Entf (Delete)** | delete |
| `0x0013` | **Bild↑ (PgUp)** | pgup |
| `0x0014` | **Bild↓ (PgDn)** | pgdn |
| `0x0026` | **NumLock** | numlock |
| `0x0027` | **Num /** | numdiv |
| `0x0029` | **Num −** | numsub |
| `0x004c` | **Ü** | lbracket |
| `0x004d` | **+** | rbracket |
| `0x004e` | **<>\|** (ISO, next to Y) | backslash *(not #)* |
| `0x007f` | **Left Ctrl (Strg)** | fn *(ANSI wrong — not Copilot)* |
| `0x0096` | **Windows** | lalt *(ANSI win was wrong code)* |
| `0x009a` | **AltGr** | ralt |
| `0x004f` | **Num 7** | num7 |
| `0x0050` | **Num 8** | num8 |
| `0x0080` | **Fn** | win *(ANSI wrong)* |
| `0x009b` | **Copilot** | rctrl |
| `0x00a3` | **Num 0** | num0 |
| `0x0051` | **Num 9** | num9 |
| `0x007c` | **Num 6** | num6 |
| `0x007b` | **Num 5** | num5 |
| `0x0079` | **Num 4** | num4 |
| `0x0092` | **Num 3** | num2 *(shifted)* |
| `0x0068` | **Num 2** | numadd *(not +)* |
| `0x0090` | **Num +** *(violet; confirm)* | num1 |
| `0x00a5` | **Num , / Num-Entf** (decimal) | numdot |
| `0x00a7` | **Num Enter** | numenter |
| `0x0076` | **- _** (near RShift) | *(unknown in ANSI map)* |
| `0x0097` | **Left Alt** | *(unknown in ANSI map)* |
| `0x00a1` | **Right arrow** | *(unknown in ANSI map)* |

### Modifier summary (DE)

| Key | Code |
|-----|------|
| Esc | `0x0001` |
| Fn | `0x0080` |
| Left Ctrl (Strg) | `0x007f` *(was mis-tagged Copilot in an early probe)* |
| Copilot | `0x009b` |
| Win | `0x0096` |
| Left Alt | `0x0097` |
| Left Shift | `0x006a` |
| Right Shift | `0x008d` |
| AltGr | `0x009a` |
| Space | `0x0098` |
| Tab | `0x0040` |
| Caps | `0x0055` |
| Enter | `0x0077` |
| Backspace | `0x0038` |

### Arrow cluster

| Key | Code |
|-----|------|
| Up | `0x009d` |
| Down | `0x009f` |
| Left | `0x009c` |
| Right | `0x00a1` |

**KEYBOARD_KEYS list is fully identified** for this 83RU DE board (see table above + earlier rows).
Machine-readable copy: `research/spectrum-keymap-de.json`.

## Effects on one zone (verified)

- **Keyboard-only rainbow-wave** works when `KEYBOARD_KEYS` is the effect’s key list
  (bars/logo set to static black in the same `0xCB` packet).
- Observed direction with `direction=0`, `speed=2`: **right → left**.
- **Keyboard-only color-pulse** (deep red, `speed=2`) works. Startup quirk:
  first second or so looks like hard **0↔100%** jumps, then settles into a
  smooth pulse/blink. Likely firmware settling after `0xCB` + profile reselect.
- **Keyboard-only rain** (cyan, `speed=2`) works well — clear droplet animation.
- So full-key lists are fine for animations on this firmware (not only `0x0065`).
  Use `0x0065` for “everything”; use zone lists to isolate keyboard / front / rear / logo.

### Still unknown (not probed yet)

Down arrow, Left/Right, C, Z, and most letter row — continue live probes as needed.
ANSI `KEY_NAMES` from spectrum-ctl is a starting point only; maintain `KEY_NAMES_DE`
in-tree once the GUI/keymap lands.

## Working multi-zone example (static)

- Keyboard: deep red, except **Q** blue (`0x0042`)
- Front bar: green (`0x01f5`–`0x01fe`)
- Back bar: blue (`0x03e9`–`0x03fa`)
- Logo: yellow (`0x05dd`)
- Brightness: 9, profile: 1

## TODO — Per-key RGB editor (UI)

Design a dedicated Lighting sub-page with an interactive **DE QWERTZ** keyboard
map (use `research/spectrum-keymap-de.json`), click-to-paint RGB, and group
presets (WASD / arrows / numpad). Backend keycodes are ready; **do not implement
the editor yet** — zone + firmware effects ship first.

## Power-mode / “turn-on” LED (NOT Spectrum)

The LED that tracks Quiet/Balanced/Performance (usually the **power button**) is
**not** on `048d:c197` Spectrum and not on `048d:c193` Lenovo Lighting.

Live probe 2026-07-21 (`research/mode-led-probe.json`):

1. Cycled `platform_profile` via daemon: low-power → balanced → performance →
   max-power → custom → balanced.
2. Snapshotted Spectrum ops `0xA5/A7/A9/AC/CF/C7/CD` and c193 report `0x5A`
   after each switch — **all constant** (no mode correlation).
3. LenovoLegionToolkit only exposes Lighting_IDs **0** (white KB), **3** (panel
   logo), **5** (ports) — **no power-button LED ID**.
4. Kernel docs: LED colour is driven by the EC with the thermal mode
   (blue/white/red/purple). Off only when the machine is off/hibernate.
5. Gen 10 users report no software dim/off; some BIOS builds may have a
   “Power Button LED” knob (not exposed under firmware-attributes on 83RU).

Undocumented Spectrum pair found (unrelated to mode LED so far):

- **GET `0xAC` / SET `0xAD`** — writable status 0–7 (value sticks). Purpose
  unknown; did not track `platform_profile`. Needs visual confirmation.

**Next (needs root):** EC dump (`ec_sys`) across profiles to find the mode-LED
register; check BIOS for an LED disable option.

## Pitfalls found in this project

1. Writing only profile `0` while the laptop is on another profile → silent no-op.
2. Animated effects need key `0x0065` (or proper zone lists), not a broken packet.
3. Spamming HID / aurora experiments leaves zones in mixed leftover colours —
   always send a **full** multi-effect replace for all zones you care about.
4. GUI must not block the main thread on HID; coalesce jobs; never double-`remove`
   glib `SourceId` (Drop already removes → abort).
5. System `/usr/bin/legion-*` can be an old build — prefer `~/.local/bin` until
   `sudo ./scripts/enable-root-daemon.sh` is run.
