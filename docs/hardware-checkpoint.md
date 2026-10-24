# Hardware checkpoint — Redragon M721-Pro (`04d9:fc5c`)

Verified on a real mouse, 2026-10-24, bcdDevice `0x0120`, vendor interface 2 (`/dev/hidraw3`).
Access: `/etc/udev/rules.d/99-redragon-gui.rules` with `GROUP="users", MODE="0660"`
(`TAG+="uaccess"` alone was not applied by logind to hidraw on this host).

## What went wrong before

The first build "confirmed" every write by read-back while nothing on the mouse changed. A read-back
only shows that the firmware **stored** the bytes. Three protocol mistakes meant it never **applied**
them:

1. **Refresh frames were mis-encoded.** The vendor sends `{02 F1 02 <flag>}`: the two bytes after
   the command are arguments (mask `02`, then a single-bit flag). The old code treated them as a
   little-endian address, so `0x0208` went out as `{02 F1 08 02}`. The firmware accepts that frame
   and ignores it.
2. **`F5` is a session bracket, not a profile selector.** `F5 00` opens a configuration session and
   `F5 01` closes it. The old code sent `F5 <profile>` per write group and never closed the session
   after a write.
3. **Several registers were guessed wrong** (below). The read path looked plausible because the
   values it decoded were wrong in ways that still looked reasonable.

## Frame rules

- Feature reports on the vendor collection (usage page `0xFFA0`): ID 2 = 16 B, 3 = 64 B, 4 = 256 B,
  5 = 1024 B, 6 = 2048 B. Report descriptor (106 bytes) is unchanged from the first checkpoint.
- Write: `{02 F3 addr_lo addr_hi len 00 00 00 payload...}`. Read: the same header with `F2`, then
  `GET_FEATURE`; the device answers with a short report (8 + len bytes).
- **Every read and write sits between `F5 00` and `F5 01`.** The closing frame must always be sent,
  or the mouse stops reporting pointer input. The engine sends it even when a step fails.
- After writes, the vendor sends refresh frames `{02 F1 02 flag}` for flags `04 01 02 08 10`. Writes
  are accepted at any time (a probe of 0 to 4 s between refreshes dropped none), but only take
  effect after a refresh.
- Switching the active profile: write `0x2C` (`{profile, 0}`) and send the full refresh sequence.
  Even so, the lighting of the new profile was not always reloaded (switching to profile 1 often
  left the old effect running). Rewriting that profile's own LED record inside the same session and
  refreshing again made it reliable, which is what `activate_profile` does.

## Register map (profile `p` = 0..4)

| Function | Address | Layout |
|---|---|---|
| Active profile | `0x002C` (len 2) | `{profile, 0}` |
| Polling rate | `0x32 + 2p` (p = 0..2), `0x38 + 2(p−3)` (p = 3..4), 1 byte | interval in ms: `1` = 1000 Hz, `2` = 500, `4` = 250, `8` = 125 |
| LED record | `0x0449 + 8p` (len 7) | `R G B mode speed mode2 brightness` |
| DPI page | `0x42, 0x102, 0x1B2, 0x262, 0x312` | byte 0 = active stage (0-based), then stage records |
| DPI stage record | `page + 2 + 6s`, len 5 | `enabled code flag 0 0` (`flag` = high bit for >16000 DPI) |

Polling codes are the opposite of what the first report assumed. The LED speed byte is inverted
(vendor writes `9 − level`) and is `0` for effects without a speed.

### LED effects

An effect is the pair `(mode, mode2)`:

| Effect | `(mode, mode2)` | Colour | Speed |
|---|---|---|---|
| Breathing | `(1, 4)` | yes | yes |
| Rainbow | `(1, 8)` | no | yes |
| Full lighted | `(1, 2)` | yes | no |
| Wave | `(2, 0)` | no | yes |
| Go without trace | `(6, 0)` | yes | yes |
| Reactive | `(7, 0)` | yes | yes |
| Flash | `(1, 0x10)` | yes | yes |
| Off | `(0, 0)` | — | no |

Brightness is `0..3`. The vendor's shipped default is Wave, red, speed 4, brightness 2.

### DPI

For this sensor (vendor id `0xD07`) the vendor UI works in indices `1..240`: DPI = `100·i` up to
index 160 (16000 DPI), then `16000 + 200·(i−160)` up to 32000. Each index maps to a code through two
tables in `RDCfg.exe` (`0x4bae1c` for flag 0, `0x4bad7c` for flag 1). `models/holtek-04d9-fc5c.toml`
holds the generated list (238 entries; the vendor maps 15900 and 16000 to the same code, so the
higher one is kept). The mouse's five default stages read back as `0b 17 2f 5f bc` = 1000, 2000,
4000, 8000, 16000 DPI.

## Verification on the mouse

Each item was checked by observing the effect, with the original value restored afterwards.

| Feature | How it was checked | Result |
|---|---|---|
| LED colour and effect | A solid-green write, then each of the eight effects on profile 1 | Seen on the mouse; labels match what the LED does |
| Profile switch | Switched between profiles with different LED effects, in both directions | LED follows the profile after the fix above; the pointer keeps working |
| DPI stage value | Active stage set to 400 DPI (was 8000) | Cursor became very slow |
| Active DPI stage | Stage 5 (16000 DPI) selected | Cursor became very fast |
| Polling rate | Counted input reports per second on `/dev/hidraw1` while moving the mouse | 125 → 125/s, 500 → 500/s, 1000 → 999/s |
| Write path | `cargo test --test hardware -- --ignored` (brightness round trip) | Passes |

## Not done

- **Buttons.** The keymap is at `0x82 + 4·slot` for profile 1 (other profiles add the page offset).
  The vendor function table is known (`81` left, `82` right, `83` middle, `84`/`85` side buttons,
  `99` DPI, `8f 01 xx 00` Ctrl+key shortcuts, `8e 01 xx xx` media keys, ...), but the physical slot
  order, the labels and the keyboard/combo encodings are not decoded.
- **Macros.** The record layout at `0x478 + 200·slot` is not decoded; reads return zeros. The
  descriptor declares no macro section so nothing guessed is written.
- **Global settings** (angle snap, lift-off, scroll speed, double-click) are not pinned.

## State left on the device

Profile 1 was restored to its original LED (steady, black), DPI stages and polling. Two leftovers
from the first, broken build were left alone because they are the user's configuration to change:
profile 4's polling register holds `8` (125 Hz) and its LED record is stored as Off.

## Reproduction

```sh
redragon-gui --list-devices
redragon-gui --dump-raw        # raw register windows (identical across runs)
redragon-gui --dump-state      # decoded settings

# scripted writes (back up, apply, read back); needs REDRAGON_GUI_ALLOW_WRITE=1
REDRAGON_GUI_ALLOW_WRITE=1 redragon-gui --set led.1.mode=breathing --set led.1.color=00ff00
REDRAGON_GUI_ALLOW_WRITE=1 redragon-gui --set dpi.1.4=1600 --set dpi.1.active=4
REDRAGON_GUI_ALLOW_WRITE=1 redragon-gui --set poll.1=1000 --set profile=2
```

Access rule used on this host:

```
KERNEL=="hidraw*", ATTRS{idVendor}=="04d9", ATTRS{idProduct}=="fc5c", GROUP="users", MODE="0660", TAG+="uaccess"
```
