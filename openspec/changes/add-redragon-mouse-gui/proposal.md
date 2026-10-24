# Proposal

## Why

The Redragon vendor application in this repo is a 32-bit Windows-only Qt5 binary whose configuration path goes through SetupAPI device paths that cannot be mapped onto Linux hidraw, leaving the mouse attached to this system (`04d9:fc5c`, M721-Pro) configurable only through a CLI (`mouse_m908`) or fragile hand-written register writes. The protocol is fully documented (`docs/redragon-gaming-mouse-re.md` §5) and the device is a plain HID feature-report device with no kernel driver requirement, so the missing piece is a native, safe, GUI-driven Linux tool that turns that protocol into an understandable workflow with read-back, backup, and review before anything is written to the hardware.

## What Changes

- Introduce a new Rust desktop application (`redragon-gui`) built on `egui`/`eframe`, shipped as a single binary with a udev rule, that detects supported Redragon/Holtek mice and configures them over HID feature reports on the vendor interface.
- Add a data-driven device/model layer: per-model descriptor files carry register layout, DPI code table, keycode table, LED capability set, and button count, so a new model is added as data plus a validated descriptor rather than new code.
- Add a read-back-first settings engine: read the current register state with GET_REPORT, present it in the GUI, and require an explicit apply that writes a diff, verifies, and keeps a timestamped backup.
- Add editor screens for DPI stages (5, X/Y link, active stage, sniper), polling rate, angle snap, lift-off distance, scroll speed, acceleration, LED mode/colour/brightness/speed/zones, onboard profiles (5 slots incl. load/save/import/export of `.prof`-compatible files), button remapping with multi-key combos, and macros with burst-fire.
- Add hidraw permission diagnostics: when the vendor node is unreadable, the app explains the exact udev rule and the affected node instead of failing silently.
- Out of scope: Windows/macOS support, the legacy `HidDevice.dll` API surface, pointer-speed writes to the OS, running `RDCfg.exe` under Wine, and support for models whose protocol is unknown.

## Capabilities

### New Capabilities

- `mouse-discovery`: finding the supported mouse, resolving its model descriptor, following hotplug, and diagnosing hidraw access failures with actionable udev guidance.
- `model-capability-data`: the per-model descriptor format (register map, DPI code table, keycode table, LED zones/modes, button count), its provenance requirements, and how unsupported or ambiguous models are reported.
- `settings-sync`: reading device state through feature reports, building a validated in-memory model, transactional apply with diff/verify/rollback, and file-based backup and restore.
- `pointer-tuning`: DPI stages (enable, X/Y link, per-axis values, active stage, sniper), polling rate (125/250/500/1000 Hz), angle snap, lift-off distance, scroll speed, acceleration, and double-click speed.
- `lighting`: LED mode selection, colour, brightness, effect speed, and per-zone control, degraded correctly on non-RGB models.
- `profiles`: the five onboard profile slots — activation, editing, copy/reset to defaults, and import/export of `.prof`-compatible profile files.
- `button-keymap`: assigning built-in mouse/multimedia/browser functions, keyboard keys, and multi-key combos to each button, including the button inventory per model.
- `macro-editor`: authoring, editing, and replaying macro records bound to buttons, plus the burst-fire ("gun") editor, with import/export.
- `desktop-ui`: the GUI contract — device status, dirty-state tracking, explicit apply/revert, error surfacing, destructive-action confirmation, and keyboard-accessible operation.

### Modified Capabilities

None — this is the project's first spec.

## Impact

- **New code**: a Rust workspace at repo root (`Cargo.toml`, `src/`), with modules for transport (hidraw), model data, settings, and UI; descriptor data under a `models/` directory; packaging assets (desktop entry, icon, udev rule).
- **Dependencies**: `eframe`/`egui`, `serde` + `toml`/`json` for descriptors, `thiserror`, `tracing`; hidraw access through direct `/dev/hidraw` ioctl (`HIDIOCSFEATURE`/`HIDIOCGFEATURE`) — no libusb, no kernel module.
- **Systems**: reads `docs/redragon-gaming-mouse-re.md` and the shipped `REDRAGON Gaming Mouse/` data tree as the protocol and format reference (read-only reference material, not redistributed); requires a udev rule for the vendor interface; writes only to the mouse and to the user's config/backup directory.
- **Existing tooling**: remains usable in parallel; this change does not modify `mouse_m908` or the vendored Windows package.
- **Safety**: all writes are HID feature reports to vendor interface collection 2; a bad write can leave the mouse in a recoverable-but-unusable state, so backup, read-back verification, and explicit confirmation are part of the capability requirements rather than later polish.
