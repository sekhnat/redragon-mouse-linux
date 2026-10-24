# Redragon Mouse (Linux)

A native Linux configuration tool for Redragon/Holtek USB gaming mice. It speaks the mouse's
HID feature-report protocol directly through `/dev/hidraw`, reads the current settings before
showing anything editable, and applies changes as a reviewed, backed-up, verified transaction.

The protocol it implements is documented in
[`docs/redragon-gaming-mouse-re.md`](docs/redragon-gaming-mouse-re.md), produced by reverse
engineering the vendor's Windows application that ships in this repository.

## Status

- **Platform:** Linux only. The device stack is hidraw; there is no libusb dependency, no kernel
  module, and no attempt to run the Windows application under Wine.
- **Model support is data-driven.** `models/00-registry.toml` maps USB IDs to a TOML descriptor.
  The bundled `04d9:fc5c` (M721-Pro "Lonewolf2") descriptor is marked **unverified**: every write
  through it requires an extra acknowledgement, and a few capabilities are deliberately absent
  because the report does not pin their registers:
  - button keymaps are not declared (the documented keymap bases collide with the DPI page), so
    the Buttons screen shows an explicit read-only explanation instead of guessing;
  - only the documented LED static/off codes are present;
  - macro action encoding is a documented hypothesis;
  - angle snap, lift-off, scroll, acceleration, and double-click registers are not declared.
- **Read-back is enforced.** If the device cannot provide a consistent register image, the tool
  stays read-only and says why. There is no "write anyway" escape hatch.
- **Hardware checkpoint (`04d9:fc5c`):** the interface's real report descriptor uses vendor page
  `0xFFA0` (not `0xFF00`) and feature reports ID 2 = 16 B, 3 = 64 B, 4 = 256 B, 5 = 1024 B,
  6 = 2048 B. Reads are consistent across runs, and one LED-brightness write was applied,
  confirmed by read-back, and restored from its backup. The DPI and LED-mode tables are still
  incomplete for this PID, so those values display as unknown and are preserved; the descriptor
  therefore stays unverified. Details, raw dumps and the commit sequence:
  [`docs/hardware-checkpoint.md`](docs/hardware-checkpoint.md).

## Build

```sh
cargo build --release
```

Requirements: a Rust toolchain (1.82 or newer), the usual X11/Wayland development packages for
`winit`, and an OpenGL-capable driver (the `glow` renderer is used). The release build is a single
binary, `target/release/redragon-gui`.

## Install

```sh
sudo install -Dm755 target/release/redragon-gui /usr/local/bin/redragon-gui
sudo install -Dm644 models/*.toml -t /usr/local/share/redragon-gui/models/
sudo install -Dm644 assets/99-redragon-gui.rules /etc/udev/rules.d/
sudo install -Dm644 assets/redragon-gui.desktop /usr/local/share/applications/
sudo install -Dm644 assets/redragon-gui.metainfo.xml /usr/local/share/metainfo/
sudo install -Dm644 assets/redragon-gui.svg /usr/local/share/icons/hicolor/scalable/apps/
sudo udevadm control --reload-rules && sudo udevadm trigger
```

Reconnect the mouse after installing the udev rule, then run `redragon-gui`.

The rule only grants access to the vendor interface of `04d9:fc5c`; add another rule (or edit the
existing one) for a different model. The application prints the exact rule it needs — including
the node path, its mode, and its owner — when access is missing.

If `TAG+="uaccess"` does not produce an ACL on the hidraw node on your system (verify with
`getfacl /dev/hidrawN`), add `GROUP="users"` to the rule (and make sure your user is in that
group), or grant a session ACL with `sudo setfacl -m u:$USER:rw /dev/hidrawN`. This host needed
the group grant; see the hardware checkpoint.

## Use

```sh
redragon-gui                    # graphical interface
redragon-gui --list-devices     # list HID interfaces and mark supported ones
redragon-gui --dump-state       # read and print the current settings
redragon-gui --show-descriptor  # print a model descriptor
redragon-gui --report-descriptor  # dump and parse the interface's HID report descriptor
redragon-gui --dump-raw         # print the raw register windows read from the device
redragon-gui --simulate holtek-04d9-fc5c   # explore the UI without hardware
```

Exit codes: `0` success, `1` error, `2` no supported device, `3` the configuration interface
cannot be opened (the access guidance is printed).

The interface has these sections:

| Section | What it does |
|---|---|
| Device | Model, IDs, node, descriptor provenance, capabilities, access guidance, last apply report |
| Pointer | Five DPI stages (enable, value, X/Y link where supported, active stage), polling rate, global pointer settings |
| Lighting | Effect, colour (RGB/HSV/hex), brightness, effect speed, zones where declared |
| Profiles | Five onboard slots: activate, copy, reset, import/export the vendor `.prof` layout |
| Buttons | Per-button action, key, macro slot, multi-key combos, primary-click guard |
| Macros | Library, ordered actions with delays, burst fire, portable macro files |
| Backups | Restore a recorded register set through the same verified path |
| Diagnostics | Log path, descriptor load problems, access guidance, apply report |

## How a change reaches the mouse

1. On connect, every register window declared by the descriptor is read twice; the two reads must
   match and must not be all-zero/all-`0xFF` filler.
2. Edits stay in a draft. The Apply button is enabled only when at least one encoded register
   differs from the baseline and the draft validates against the descriptor.
3. Apply shows the exact change set (register, before, after) and asks for confirmation. An
   unverified descriptor adds a risk acknowledgement.
4. The targeted registers are re-read; if they changed since the baseline read, the apply is
   refused until you explicitly choose "write anyway".
5. A timestamped backup of the pre-apply values is written **before** the first frame.
6. Frames are written per profile with begin/commit framing and the documented pacing. A failure
   triggers a rollback attempt of the registers already written.
7. Every written register is read back and compared. Mismatches are reported as "not confirmed";
   they are never presented as success.

## Files

- Backups: `$XDG_STATE_HOME/redragon-gui/backups/<timestamp>_<vid>_<pid>_p<profile>.json`
  (defaults to `~/.local/state/redragon-gui/backups`). Never pruned automatically.
- Logs: `$XDG_STATE_HOME/redragon-gui/logs/session-*.log`.
- Macro names: `$XDG_CONFIG_HOME/redragon-gui/macro-names.json` (the device stores records, not
  names).
- Exported profiles use the vendor `.prof` field names; exported macros are JSON documents.

## Recovery

A wrong write can leave the mouse in a state that is awkward to use (for example a reassigned
primary click). Before writing anything, the tool stores the previous register values; restore
that backup from the Backups screen. If the mouse is unresponsive, reconnect it — the mouse keeps
its configuration across power cycles — and use the vendor application or a firmware reflash as
the last resort. The primary-click reassignment is guarded by a confirmation that spells this out.

## Model descriptors

Adding a model means adding a TOML descriptor and a registry entry — no recompilation. A descriptor
declares the register windows used for reading, the write regions for each feature, the encoding
tables (DPI codes, keycodes, LED modes/zones, button inventory), and its provenance. See
`models/holtek-04d9-fc5c.toml` for a commented example.

Descriptors are validated at load: missing sections, overlapping write regions, addresses outside
the frame capacity, and tables that cannot express a declared value are rejected. One broken file
does not prevent the others from loading; the problem is shown in Diagnostics.

## Tests

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

The suite covers frame golden vectors, report-descriptor parsing, registry resolution (including
version-range conflicts and malformed files), descriptor validation, state round-trips, the
transaction engine (stale refusal, verification mismatch, mid-transaction rollback, backup replay),
profile and macro file round-trips, hotplug events, and a headless render of every screen.

## License and provenance

Dual-licensed under MIT or Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`).

The protocol facts in the descriptors come from the repository's own reverse-engineering report
and from public HID documentation. No tables or source code from `mouse_m908` (GPL-3.0) were
copied; the descriptor schema records provenance per model so that distinction stays visible.
