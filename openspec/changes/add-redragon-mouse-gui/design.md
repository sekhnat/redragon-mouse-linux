# Design

## Context

See `proposal.md` — Why. The constraints that shape this design:

- The protocol is documented in `docs/redragon-gaming-mouse-re.md` §5 and independently corroborated by `mouse_m908`; it is a memory-mapped register file carried in HID **feature** reports on the vendor interface, with `0xF5` begin / `0xF3` write / `0xF2` read-select / `0xF1` commit framing and roughly 10 ms pacing between frames.
- The read-only hardware checkpoint corrected two of the report's assumptions for `04d9:fc5c`: the vendor collection is `0xFFA0`/`0x01`, and the feature-report table is ID 2 = 16 B, 3 = 64 B, 4 = 256 B, 5 = 1024 B, 6 = 2048 B (not 2/3/4 with a 1024-byte report 4). Descriptors therefore declare a report-size table and their own vendor collection rather than assuming either.
- The attached device (`04d9:fc5c`, M721-Pro) binds to generic `usbhid`; its vendor interface is `/dev/hidraw3` at `0600 root:root`. No kernel module is involved, so the tool is a userspace HID client that needs a udev rule and nothing else.
- Read-back support on this PID is documented but marked untested upstream, and the shipped vendor app never reads general settings back. Reading is therefore the riskiest assumption in the whole design and must be proven before any write path is exposed.
- The vendored Windows package is a 705-file data tree that is already in the repo. It is reference material for descriptor fields, not part of the product.
- This repo has no application code yet; `rustc 1.99.1` / `cargo 1.99.1` are available.

## Goals / Non-Goals

**Goals:**

- One narrow transport interface that every device operation goes through, so the protocol can be exercised against a simulation in tests and in the UI without hardware.
- A whole-register-space baseline as the single source of truth for diffing, backup, restore, and verification, with typed views layered on top rather than an ad-hoc struct per screen.
- Descriptors as reviewed data with provenance, so model coverage grows without protocol code changes.
- Every write path provably preceded by a validated, backed-up, re-read baseline.

**Non-Goals:**

- Cross-platform transport abstraction. The `Transport` trait is internal; a Windows/macOS backend is not modelled.
- Macro playback or a macro interpreter in the tool — the device plays macros, the tool only authors and stores them.
- Re-implementing the vendor app's OS integration (pointer speed via `SystemParametersInfo`) or its INI-as-state-of-record model. Our state of record is the device.
- Automatic model discovery by fingerprinting unknown devices. Unknown PIDs are reported, not guessed.
- Packaging/distribution automation (AUR, Flatpak, AppImage) beyond committing the metadata files.

## Decisions

### D1 — Transport: direct hidraw ioctl, not hidapi or libusb

`HIDIOCGFEATURE` / `HIDIOCSFEATURE` on the vendor `/dev/hidraw` node, behind a two-method trait (`get_feature(report_id, len)`, `set_feature(&buf)`).

- *Why*: the vendor interface is already exposed by `usbhid`; hidraw needs no C dependency, no kernel-driver detach, and gives us the node path and its permission bits, which the access-diagnostics requirement depends on.
- *Alternatives*: the `hidapi` crate would add a C build and hide node details behind a device path; `libusb` would let us claim the interface, but detaching/re-attaching the kernel driver on a mouse that is simultaneously the active pointer is a self-inflicted outage. Both are overkill for feature reports on an interface the kernel already published.
- Frames are built and parsed in one module (`frames`) so no other module knows about `0xF3` or byte 4 being a length marker.

### D2 — Enumerate from sysfs, and parse the report descriptor instead of trusting the report

Detection walks `/sys/class/hidraw/hidraw*`, reads `HID_ID`/`uevent`, `bcdDevice`, and `bInterfaceNumber` from the parent USB interface, then pulls the report descriptor with `HIDIOCGRDESC` and parses the first top-level collection's usage page/usage.

- *Why*: sysfs gives vendor/product/version/interface without opening the node, so unsupported devices can be reported without touching them; the descriptor parse closes the report's open question about the 106-byte vendor descriptor and lets us assert the real feature-report sizes before writing.
- *Alternatives*: assuming the documented sizes (silently wrong on a different model) or `hid_enumerate` via hidapi (needs the C dependency we avoided).
- If the parsed sizes disagree with the descriptor's declared frame sizes, or a declared feature report is missing, the device is treated as unsupported rather than written.
- The declared report table is data (`[[frames]]` with ID and length), because feature-report layouts vary by model; the code keys everything by report ID instead of assuming three fixed classes.

### D3 — State of record is the raw register image

A connected device holds one `RegisterImage`: the frame IDs and bytes read from the device, keyed by report ID and address. Typed settings (`DpiStage`, `LightingEffect`, `ButtonMapping`, …) are decoded views over that image, and edits write back into the image.

- *Why*: the device is a memory-mapped register file, so byte-level diffing gives change-set minimization, complete backup/restore, and exact verification for free. A per-screen struct model would need a bespoke backup, diff, and verify path per feature — the class of bug that bricks a mouse.
- *Trade-off*: the image must be read in full for a profile before typed edits are trustworthy; that is a few hundred small transfers, shown as a progress state rather than hidden.
- Baseline (`on_dev`) and draft (`pending`) images are separate; the UI renders both and apply diffs them.

### D4 — Descriptor format: TOML files plus a registry, versioned, validated in two passes

`models/index.toml` maps `vid`, `pid`, optional `bcdDevice` range, and `descriptor`. Each descriptor declares `schema_version`, provenance (`capture` / `upstream` / `readback`), `verified` flag, register map with per-profile strides, and encoding tables (DPI codes, keycodes, LED modes/zones, button inventory, limits).

- *Why TOML*: reviewable diffs, comments for provenance notes, and no dependency on the vendor INI parser.
- *Why two passes*: `serde` catches shape errors, then semantic validation catches missing strides, overlapping address ranges, and tables that cannot express a declared capability. Parse failure of one file must not take the others down (spec: malformed descriptor present).
- *Alternative*: deriving tables from the vendored `Config.ini`/`.prof` tree at runtime — clever, but it couples the product to a 236 KB vendor artifact whose schema drifts between generations.

### D5 — Apply is one engine, not per-feature code

`settings::apply` builds an ordered operation list from the draft/baseline diff, then: validate → backup → re-read targeted registers (stale check) → begin → write changed frames → commit → verify by read-back → rollback on failure.

- *Why*: one code path means one place to get pacing, framing, rollback, and verification right; features contribute only address/encoding knowledge.
- The engine returns a structured outcome (`Confirmed`, `NotConfirmed { registers }`, `RolledBack { restorable, not_restorable }`, `ValidationFailed { fields }`) that the UI renders directly, so error detail cannot be lost between layers.

### D6 — Read-back gate before exposing any write path

A descriptor is only allowed to drive writes when a read session produced a consistent image: every declared frame readable, no all-zero or all-`0xFF` bulk response, and a second read returning identical bytes.

- *Why*: upstream marks `fc5c` experimental and the vendor app never reads settings; if GET_REPORT is unusable on this PID, the whole safe-apply design collapses into guessing. Failing closed keeps the first release useful for diagnostics and honest about it.
- Consequence: on a model where reads are broken, the tool is read-only and says so. There is deliberately no "write anyway" toggle — that would negate D3/D5.

### D7 — Threading: one I/O worker, UI stays immediate-mode

A worker thread owns the open node and a command queue (`ReadProfile`, `Apply`, `Verify`, `Dump`); the UI sends requests and polls results, calling `request_repaint` on completion. Hotplug is a sysfs poll every 2 s plus an immediate re-check after any transport error.

- *Why*: egui repaints on demand, and a blocked `ioctl` inside a repaint would freeze the window; a second process-level lock would still not stop a foreign writer, which the stale check already covers.
- Exclusivity is advisory: our own instances take a lock file under `$XDG_RUNTIME_DIR/redragon-gui/`, and the pre-apply re-read is what actually protects against `mouse_m908` or the vendor app writing concurrently.

### D8 — Backups and logs in XDG state

`$XDG_STATE_HOME/redragon-gui/backups/<timestamp>_<vid>_<pid>_p<profile>.json` (register frames as hex + descriptor id/version + device IDs) and `.../logs/session-*.log` via `tracing`. Backups are never pruned automatically.

- *Why JSON for backups*: diffable by eye, trivially restorable, and independent of the descriptor schema so an old backup stays restorable after descriptor edits.
- Restore replays through the same apply engine, so a restore is verified like any other write.

### D9 — License: dual MIT/Apache-2.0, descriptors from vendor data and read-back only

- *Why*: `mouse_m908` is GPL-3.0. Copying its captured frame tables or DPI code tables into descriptors would relicense the project. Descriptors are instead derived from the vendored vendor data (facts about field layout) and from live read-back, with provenance recorded per descriptor.
- *Alternative*: adopt GPL-3.0 outright and import upstream tables for speed. Rejected because it constrains reuse of the transport/settings core and the provenance model already records what a descriptor owes to whom.

### D10 — Repository layout

```
Cargo.toml, src/            single binary crate `redragon-gui`
  transport/{frames,hidraw,sim}.rs
  device/{enumerate,permissions}.rs
  models/{descriptor,registry,validate}.rs
  settings/{state,diff,apply,backup}.rs
  ui/{app,device,pointer,lighting,profiles,buttons,macros,backups,diagnostics}.rs
models/index.toml, models/*.toml
assets/{redragon-gui.desktop, udev/99-redragon-gui.rules, icon, metainfo}
tests/{frames_golden, descriptor_validation, apply_simulation}.rs
```

- *Why*: `transport` and `settings` are the depth of the project and are UI-free; `ui` is a thin rendering layer over draft/baseline images. Keeping the descriptor data outside `src/` makes a new model a data-only change, as the specs require.
- The vendored `REDRAGON Gaming Mouse/` tree stays untouched and is excluded from packaging.

### D11 — CLI surface is part of the app, not a second binary

`--list-devices`, `--dump-state`, `--simulate <model>`, and `--log-level` are handled in `main` before the window starts; exit codes are `0` success, `2` no supported device, `3` access failure, `1` other errors.

- *Why*: the headless requirements and the simulation mode are the same code paths the UI uses, so CI and users get identical behaviour with no display server.

## Risks / Trade-offs

- **GET_REPORT may not work on `fc5c`** → read-consistency gate (D6) keeps the tool read-only and explicit; first milestone is a read-only session that either confirms or kills the write path before it ships. Documented in the report as open question 5.
- **A bad write can leave the mouse unusable** → backup before the first frame, change-set minimization, read-back verification, rollback attempt, unverified-descriptor extra confirmation, and left-click reassignment confirmation. Recovery path (vendor app / reflash) is documented in diagnostics.
- **Descriptor data may be wrong for a model we cannot test** → per-descriptor provenance and `verified` state; unverified descriptors force an extra confirmation; capability gating means absent sections are absent UI instead of broken controls.
- **egui has weaker screen-reader support than GTK**, while `desktop-ui` requires full keyboard operation → keyboard nav and focus rings are implemented deliberately; assistive-technology parity is accepted as a v1 limitation and recorded rather than promised. Revisit if accessibility becomes a hard requirement.
- **Full-image reads can be slow** (hundreds of transfers at ~10 ms) → read the active profile plus global settings on connect, read other slots lazily for copy/import and for post-apply isolation checks; show progress and keep the UI responsive via the worker thread.
- **Immediate-mode UI re-renders drafts every frame** → drafts live in plain data (`RegisterImage` + typed view cache), so rendering is a pure function of state; no I/O or allocation-heavy work in the paint path.
- **Advisory locking cannot stop a foreign writer** → pre-apply stale re-read and explicit confirmation naming the differences (D7).
- **`bcdDevice` ranges can overlap across descriptors** → narrowest-match resolution plus an explicit conflict error rather than a silent pick.
- **Protocol drift in vendor `.prof` schema** (`KeyInfo_i_` vs `KeyInfo_i_L`, `acee`/`aceel`) → import accepts both documented forms and reports anything it cannot map instead of dropping it silently.

## Migration Plan

Greenfield, so there is no data or user migration. Rollout is staged within the change:

1. Land transport, sysfs enumeration, descriptor loading, and `--list-devices` — no writes exist in the binary yet.
2. Add read sessions and `--dump-state`; run one against the attached `fc5c` to confirm the read-consistency gate. If reads fail, stop and re-scope to a read-only diagnostic release rather than shipping writes.
3. Add the apply engine with simulation tests, then enable it for descriptors marked `verified` only.
4. Add the remaining editors against the same engine.
5. Ship packaging assets; document the udev rule and the recovery path.

Rollback: every stage is independently shippable; the binary never writes before stage 3, and `--simulate` plus the read-only flags remain available if a descriptor is later found wrong. A released descriptor can be demoted to `unverified` or removed, and backups restore prior device state.

## Open Questions

- Exact LED mode and zone encodings for models other than `fc5c` — answerable by capturing register images from each model; does not change the design or task order.
- Whether the device stores macro records with trailing padding or a terminator — resolved by the first read-back of a macro slot; affects encoding only.
- Whether a `bcdDevice`-based split is needed for the three PIDs the vendor build whitelists (`fca9`, `fcb6`, `fc9f`) — each gets an unverified descriptor until captured; registry shape already supports it.
