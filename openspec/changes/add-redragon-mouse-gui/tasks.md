# Tasks

## 1. Project foundation

- [x] 1.1 Create the Rust binary crate (`Cargo.toml`, `src/main.rs`, `src/lib.rs`) with modules per design D10 and the dependency set from design D1/D4/D8; verify `cargo build` and `cargo test` both succeed on an empty test suite
- [x] 1.2 Add `LICENSE-MIT`, `LICENSE-APACHE`, and a provenance note stating that no `mouse_m908` tables are copied (design D9); verify both license files exist and `README.md` names the dual license
- [x] 1.3 Implement XDG state/config path resolution and `tracing` file logging under `$XDG_STATE_HOME/redragon-gui/logs`; verify a test setting `XDG_STATE_HOME` to a temp dir produces a log file containing a startup record
- [x] 1.4 Establish the quality gate: `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`; verify both commands exit 0 on the skeleton

## 2. Transport layer

- [x] 2.1 Implement frame construction/parsing for report 2 (16 B `0xF5`/`0xF3`/`0xF1`), report 3 (64 B scroll block at `0x20`), and report 4 (1024 B macro frame at `0x478 + 200×button`) with the length/payload rules from the report §5.2; verify golden-vector unit tests cover each frame class, including a non-zero address and a full macro payload
- [x] 2.2 Define the two-method `Transport` trait and implement the hidraw backend using `HIDIOCGFEATURE`/`HIDIOCSFEATURE` with an injectable pacer (default 10 ms, 100 ms after the report-3 frame); verify unit tests exercise buffer sizing, report-ID handling, and error mapping without touching hardware
- [x] 2.3 Implement `SimTransport`: an in-memory register image that records frames and can inject failures at chosen indices; verify a test applies a frame set and reads the same bytes back, and another test injects a mid-transaction failure and observes it reported at the expected index

## 3. Device discovery

- [x] 3.1 Implement sysfs enumeration of `/sys/class/hidraw/*` reading vendor/product ID, `bcdDevice`, `bInterfaceNumber`, and node path; verify `cargo test` covers a fixture sysfs tree and `redragon-gui --list-devices` on this host reports `04d9:fc5c` interface 2 with its node
- [x] 3.2 Implement report-descriptor retrieval (`HIDIOCGRDESC`/`HIDIOCGRDESCSIZE`) and parse the first top-level collection's usage page/usage plus feature report IDs and sizes; verify tests parse a captured descriptor fixture and that a size mismatch against the descriptor marks the device unsupported
- [x] 3.3 Implement access diagnostics: on open failure, report node path, error kind, mode/owner from metadata, and a generated udev rule for the device IDs; verify a test with a mocked root-owned `0600` node produces the rule text and that `--dump-state` without access exits with the access-failure code
- [x] 3.4 Implement registry resolution with narrowest-`bcdDevice` match, explicit conflict refusal, and unsupported-device reporting that lists observed IDs; verify tests cover range-match, unrestricted-match, and two-overlapping-ranges cases
- [x] 3.5 Implement hotplug polling (2 s sysfs scan plus re-check after transport errors) with arrival/removal events that retain pending edits; verify a test driving simulated presence changes shows arrival, removal, and edit retention without restart

## 4. Model data

- [x] 4.1 Define descriptor types with `serde` (schema version, provenance, verified flag, register map with per-profile strides, DPI/keycode tables, LED modes and zones, button inventory, limits) and reject unknown fields; verify round-trip tests plus a test that an unknown field fails parsing with the field named
- [x] 4.2 Implement semantic validation: required sections per declared capability, address/stride consistency, no overlapping ranges, tables able to express every declared value; verify tests for a missing DPI stride, an overlapping address, and an empty table all fail with specific messages
- [x] 4.3 Author `models/00-registry.toml` and an `04d9:fc5c` descriptor from the report §5.3 register map and the vendored profile/Config files, with provenance `capture` and `verified = false`; verify `--list-devices` resolves the model name and `--show-descriptor` prints its capabilities
- [x] 4.4 Make descriptor loading resilient: one malformed file is reported and skipped while others load; verify a test with one broken and one valid descriptor loads the valid one and reports the broken file path
- [x] 4.5 Add the unverified-descriptor warning and the extra write confirmation path; verify a test that writing through an unverified descriptor requires the extra acknowledgement and a verified one does not

## 5. Settings core

- [x] 5.1 Implement `RegisterImage` with typed decode/encode views for DPI stages, polling rate, lighting, scroll/accel/double-click, keymap slots, and macros; verify round-trip tests decode a fixed register image into expected typed values and encode them back byte-identically
- [x] 5.2 Implement baseline/draft diffing that yields only changed registers; verify a test changing one lighting byte produces exactly one operations entry plus begin/commit framing
- [x] 5.3 Implement the validation pass producing per-field errors (enumerated values, ranges, descriptor bounds) that blocks apply; verify tests for an invalid polling rate, an out-of-range double-click speed, and an invalid keycode all block with the field identified
- [x] 5.4 Implement the apply engine (validate → backup → stale re-read → begin → write → commit → verify → rollback) returning structured outcomes; verify simulation tests for confirmed apply, read-back mismatch, mid-transaction failure with successful rollback, and rollback not fully restorable
- [x] 5.5 Implement backup write and restore: timestamped JSON with register frames, descriptor ID/version, device IDs, profile; verify a test applies, edits the backup file, restores it, and observes the pre-apply values replayed through the engine
- [x] 5.6 Implement the read-consistency gate (all declared frames readable, no all-zero/all-`0xFF` bulk image, second read identical) that downgrades the device to read-only; verify tests for a clean double read, an all-zero image, and a divergent second read

## 6. Read-only hardware checkpoint

- [x] 6.1 Install the udev rule for `04d9:fc5c`, reload rules, and run `--dump-raw` against the attached M721-Pro; verify the dump completes and a second run is byte-identical (both outputs recorded in `docs/hardware-checkpoint.md`)
- [x] 6.2 Confirm the parsed vendor-interface report descriptor: usage page `0xFFA0`/usage `0x01`, feature reports ID 2 = 16 B, 3 = 64 B, 4 = 256 B, 5 = 1024 B, 6 = 2048 B; update the descriptor's collection and frame table to match the hardware rather than the report's earlier assumption
- [x] 6.3 Decide the write path from 6.1/6.2: reads are consistent, but the DPI and LED-mode tables are still incomplete and no write encoding has been confirmed on this PID, so the descriptor stays `verified = false` (writes need the extra confirmation) and the decision is recorded in `docs/hardware-checkpoint.md`; `--show-descriptor` reflects the unverified state

## 7. UI shell

- [x] 7.1 Build the single-window shell: persistent device status header, section navigation (device, pointer, lighting, profiles, buttons, macros, backups, diagnostics), and a no-device state that keeps retrying; verify `cargo run -- --simulate fc5c` shows the header and every supported section
- [x] 7.2 Implement the I/O worker thread and command queue with progress reporting and repaint-on-completion; verify a simulated slow apply keeps the window responsive and shows progress, and that no I/O call runs on the UI thread
- [x] 7.3 Implement dirty-state tracking, change-set summary dialog with before/after values, apply/revert, and the quit-with-pending-edits prompt; verify `--simulate` scenarios exercise apply enabled/disabled, summary contents, revert, and the quit prompt in each of its three choices
- [x] 7.4 Implement error surfacing with cause and next step, plus the diagnostics section showing the log path and a copyable udev rule; verify a simulated device error appears in both the interface state and the log file
- [x] 7.5 Implement keyboard navigation and visible focus, theme following the desktop preference, and scaling without clipping; verify a manual pass at 100% and 150% in both light and dark, and a keyboard-only apply
- [x] 7.6 Wire `--simulate <model>`, `--list-devices`, `--dump-state`, and `--show-descriptor` through the same code paths as the UI, with exit codes 0/1/2/3; verify each flag's success and failure exit codes in tests

## 8. Feature screens

- [x] 8.1 Build the device-information screen showing model, IDs, node, descriptor ID, provenance, verification state, and capability list; verify `--simulate` renders a verified and an unverified descriptor with different banners
- [x] 8.2 Build the pointer-tuning screen for the five DPI stages (enable, X/Y link, per-axis values, active stage, sniper target) and polling rate; verify simulated tests cover linked/unlinked axes, the last-enabled-stage refusal, active-stage relocation on disable, and an unsupported DPI value rejection
- [x] 8.3 Add angle snap, lift-off distance, scroll speed/in-page, acceleration, and double-click controls with device-level vs profile-level application; verify a simulated apply changing only lift-off produces no profile registers, and an out-of-range double-click blocks apply
- [x] 8.4 Build the lighting screen with declared modes, RGB/HSV/hex colour in sync, brightness, effect speed, and zones; verify simulated tests cover colour round-trip, colour persistence through a colourless effect, disabled controls for undeclared effects/speeds, and the unsupported-model state
- [x] 8.5 Build the profiles screen with five slots, active-slot marking, activation with read-back confirmation, copy, and reset; verify simulated tests cover confirmed activation, unconfirmed activation, activation blocked by pending edits, and slot isolation after apply
- [x] 8.6 Implement profile import/export in the vendor INI shape accepting `KeyInfo_i_`/`KeyInfo_i_L` and `acee`/`aceel`; verify tests round-trip a profile, import a vendored `.prof` fixture from `REDRAGON Gaming Mouse/`, and reject a file with an unsupported polling rate as a whole
- [x] 8.7 Build the button screen with the declared inventory, action classes, multi-key combos, keycode table validation, current-mapping display including unrecognised encodings, left-click confirmation, and per-button/profile reset; verify simulated tests cover combo limit refusal, invalid keycode blocking, left-click confirmation, and reset-all
- [x] 8.8 Build the macro library and editor: create/rename/duplicate/delete within declared slots, ordered actions with delays, size-limit enforcement, and the burst-fire editor; verify simulated tests cover slot-limit refusal, oversized-macro blocking, burst-fire round-trip, and deletion of a bound macro
- [x] 8.9 Implement verified macro writes through the report-4 layout and macro import/export; verify a simulated mismatch is reported as not confirmed with the bound button named, and an exported macro re-imports with identical actions

## 9. Packaging and documentation

- [x] 9.1 Add `assets/99-redragon-gui.rules`, the desktop entry, app icon, and AppStream metainfo; verify `desktop-file-validate` passes and the rule matches the device IDs used in diagnostics
- [x] 9.2 Write `README.md` covering installation, the udev rule, the read-only fallback, backup location, and the recovery path (vendor app/reflash); verify the documented commands work verbatim on this host
- [x] 9.3 Record the hardware checkpoint findings (report descriptor, read behaviour, and the keep-unverified decision) in `docs/hardware-checkpoint.md`; the document names the tested PID and the observed raw data

## 10. Verification

- [x] 10.1 Run the full suite plus `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`; verify all pass and the release build produces a single binary
- [x] 10.2 Run an end-to-end simulated session covering detect → read → edit → change-set summary → apply → verify → backup → restore; verify every step reports confirmed state and no hardware access occurs
- [x] 10.3 On hardware, apply one benign change (LED brightness) to a non-active profile, verify read-back matches, then restore from the generated backup and verify the original values return (recorded in `docs/hardware-checkpoint.md`; reproducible via `tests/hardware.rs` with `REDRAGON_GUI_ALLOW_WRITE=1`)
- [x] 10.4 Run `--list-devices` and `--dump-state` from the release binary with the mouse detached and reattached; no-device exits 2 for both, the access-failure path exits 3, and reattaching restores discovery, consistent reads and identical raw dumps (recorded in `docs/hardware-checkpoint.md`)
