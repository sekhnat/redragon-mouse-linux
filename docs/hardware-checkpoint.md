# Hardware checkpoint — Redragon M721-Pro (`04d9:fc5c`)

Read-only verification of the vendor interface on this host, performed after the udev access fix.
No configuration writes were sent to the mouse during this checkpoint.

- **Device:** `04d9:fc5c`, bcdDevice `0x0120`, interface 2, `/dev/hidraw3`
- **Model descriptor:** `holtek-04d9-fc5c` (schema 1)
- **Date:** 2026-10-24
- **Access:** `/etc/udev/rules.d/99-redragon-gui.rules` with `GROUP="users", MODE="0660"`.
  `TAG+="uaccess"` was present but logind did not apply the ACL to the hidraw node on this host
  (it did for `/dev/dri/card0`), so the group grant is what makes the node readable.

## Report descriptor (106 bytes, read with `HIDIOCGRDESC`)

```
05 0c 09 01 a1 01 85 01 05 0c 19 00 2a ff 0f 15
00 26 ff 0f 75 10 95 02 81 00 75 08 95 03 81 01
c0 06 a0 ff 09 01 a1 01 85 02 09 02 15 00 26 ff
00 75 08 95 0f b1 02 85 03 09 03 95 3f b1 02 85
04 09 04 95 ff b1 02 85 05 09 05 96 ff 03 b1 02
85 06 09 06 96 ff 07 b1 02 85 07 09 07 95 07 81
00 85 08 09 08 95 07 81 00 c0
```

Parsed:

| Top-level collection | Usage page | Usage |
|---|---|---|
| Consumer | `0x000C` | `0x01` |
| Vendor | **`0xFFA0`** | `0x01` |

| Feature report | Payload | Total frame |
|---|---|---|
| ID 2 | 15 B | 16 B |
| ID 3 | 63 B | 64 B |
| ID 4 | 255 B | 256 B |
| ID 5 | 1023 B | 1024 B |
| ID 6 | 2047 B | 2048 B |

IDs 7 and 8 are 7-byte input reports.

Two assumptions in `docs/redragon-gaming-mouse-re.md` are wrong for this PID and were corrected
in the descriptor:

1. The vendor collection is `0xFFA0`, not `0xFF00` (the same newer-generation page OpenRGB
   reports for `fc62`).
2. The 1024-byte report is ID 5; report 4 is 256 bytes. The descriptor's frame table is now a
   `[[frames]]` list (ID + length) rather than three fixed classes, and macro records use the
   256-byte report 4 with a 200-byte payload.

## Read protocol

- A read is `SET_REPORT` with `{report, 0xF2, addr_lo, addr_hi, len, 0,0,0}` followed by
  `GET_REPORT` on the same report. The response repeats an 8-byte header and carries the
  requested bytes from offset 8.
- The device returns a *short* report sized to the request: 8 + len bytes (including the report-ID
  byte), not the full declared report. `HIDIOCGFEATURE` returns that length (e.g. 15 for a 7-byte
  window on report 2, 40 for a 32-byte window on report 3). The transport now accepts and
  truncates to the returned length instead of rejecting it.
- `0xF5 <profile>` selects the page for the following read or write. Apply uses the vendor's
  commit sequence `{02 F1 02 02}` then `{02 F1 02 08}`; profile activation writes
  `{02 F3 2C 00 02 <profile> 00 …}` and then commits `{02 F1 02 01}`, `{02 F1 02 08}`,
  `{02 F1 02 04}`, `{02 F1 02 02}`, `{02 F1 02 08}`. An earlier build used
  `{02 F1 01 00}`, which the device rejects with `EPIPE`.
- **The read sequence must end with `{02 F5 01}`.** The vendor tool sends this final frame after
  every read pass; without it the mouse stays in its read/configuration state and stops reporting
  pointer input. The first checkpoint reads omitted it and the pointer stopped until the frame
  was sent (and, if needed, the mouse was replugged). `read_image` now always sends the exit frame,
  including when a read fails.

## Consistency check

`redragon-gui --dump-raw` was run twice. The two dumps are byte-identical (31 lines: 5 LED windows,
15 macro slots, 5 polling windows, 5 profile pages, plus a header line). The only difference between
two `--dump-state` runs was the session log filename.

Raw snapshot (macro slots are all zero and omitted here):

```
led[0] report 2 addr 0x0449 len 7: 00 00 00 01 00 02 02
led[1] report 2 addr 0x0451 len 7: ff 00 00 02 04 00 02
led[2] report 2 addr 0x0459 len 7: ff 00 00 02 04 00 02
led[3] report 2 addr 0x0461 len 7: ff 00 00 02 04 00 02
led[4] report 2 addr 0x0469 len 7: ff 00 00 02 04 00 02
polling_hi[3] report 2 addr 0x0038 len 3: 02 00 02
polling_hi[4] report 2 addr 0x003a len 3: 02 00 1f
polling_lo[0] report 2 addr 0x0032 len 5: 01 00 02 00 02
polling_lo[1] report 2 addr 0x0034 len 5: 02 00 02 00 02
polling_lo[2] report 2 addr 0x0036 len 5: 02 00 02 00 02
profile_page[0] report 3 addr 0x0042 len 32: 03 00 01 0b 00 00 00 00 01 17 00 00 00 00 01 2f 00 00 00 00 01 5f 00 00 00 00 01 bc 00 00 00 00
profile_page[1] report 3 addr 0x0102 len 32: 01 00 01 0b 00 00 00 00 01 17 00 00 00 00 01 2f 00 00 00 00 01 5f 00 00 00 00 01 bc 00 00 00 00
profile_page[2] report 3 addr 0x01b2 len 32: 02 00 01 0b 00 00 00 00 01 17 00 00 00 00 01 2f 00 00 00 00 01 5f 00 00 00 00 01 bc 00 00 00 00
profile_page[3] report 3 addr 0x0262 len 32: 03 00 01 0b 00 00 00 00 01 17 00 00 00 00 01 2f 00 00 00 00 01 5f 00 00 00 00 01 bc 00 00 00 00
profile_page[4] report 3 addr 0x0312 len 32: 04 00 01 0b 00 00 00 00 01 17 00 00 00 00 01 2f 00 00 00 00 01 5f 00 00 00 00 01 bc 00 00 00 00
```

Decoded state: active profile 4 (index 3); all five profiles have stages 1–5 enabled with the same
DPI codes; polling 125 Hz on profile 1 and 250 Hz on the others; LED colour black on profile 1 and
red on profiles 2–5, brightness 2; all 15 macro slots empty.

The GUI reads the same values: DPI stages show `unknown code 0x000b … 0x00bc` (preserved, not
guessed), polling shows the read rate, and the change set is empty until something is edited.

## Observations and open data

- **DPI codes are unknown to the table.** The five stages report codes `0x000b`, `0x0017`,
  `0x002f`, `0x005f`, `0x00bc`. `tables.dpi_codes` only contains the two values the report
  documented (`1000 → 0x0016`, `5000 → 0x0073`), so the UI displays the read codes as unknown and
  preserves them byte-for-byte. No interpolation is performed.
- **LED mode codes are unknown.** Profiles use mode bytes `0x01` and `0x02`, while the table only
  declares the report's `0x08` (static) and `0x00` (off). Brightness at base+6 reads `0x02`, and
  the colour bytes are consistent with the visible red lighting. The offsets in the descriptor
  (`mode +3`, `speed +5`, `brightness +6`) follow the read evidence and upstream reader offsets;
  they are still unconfirmed for writes.
- **Macro slots read back as zeros** via report 4 at `0x478 + 200·slot` with a 200-byte window.
  Report 5 answers the same request, so the report class for macros is not settled by reads alone.
- **Profile pages use irregular addresses.** The vendor capture's read rows (and a live probe at
  0x1B2/0x262/0x312) show pages at `0x0042, 0x0102, 0x01B2, 0x0262, 0x0312`: the first gap is
  0xC0 and the rest are 0xB0. A uniform 0xC0 stride (an earlier assumption) read unrelated
  registers for profiles 3–5 and produced impossible stage data. The descriptor now lists the
  addresses explicitly, and DPI writes derive their target address from the same read window, so
  read and write pages cannot drift apart.

## Write verification

One benign write was performed with explicit user consent, through the app's own engine:

- Target: profile 1 (non-active; profile 4 was active), LED brightness at `0x044F`.
- Change: brightness 2 → 1.
- Sequence: `F5 00`, `F3 4F 04 01 00 00 00 01`, `F1 02 02`, `F1 02 08`, switch back to the active
  profile, then read-back.
- Result: `changed = 1`, `confirmed = 1`, no mismatches. A backup was written before the first
  frame.
- Restore: the backup replayed through the same engine, `confirmed = 1`, and a fresh read showed
  brightness back at 2.

`tests/hardware.rs` (ignored by default, requires `REDRAGON_GUI_ALLOW_WRITE=1`) reproduces this
round-trip. This confirms the write transport, the F3 brightness frame, the commit sequence, and
the backup/restore path on this PID.

## Decision

The descriptor stays **unverified** (`verified = false`), even though the write path now works.

What is proven:

- reads are consistent and decode to plausible values;
- a single-register write is accepted, confirmed by read-back, and reversible from the backup;
- the commit and read-exit sequences required by the firmware are known.

What is still missing:

- the DPI code table only contains the report's two values (`1000 → 0x0016`, `5000 → 0x0073`),
  while the device reports `0x0b/0x17/0x2f/0x5f/0xbc` for its five stages — selecting another DPI
  would write an unverified code;
- the LED mode table only contains the report's `0x08`/`0x00`, while the device currently uses
  `0x01`/`0x02`; the two-byte mode encoding is not decoded;
- macro record encoding has not been written to the device.

Promoting the descriptor would remove the extra confirmation from those unverified writes, so it
stays unverified and every write still needs the risk acknowledgement. `--show-descriptor` reports
`provenance: vendor software capture (unverified)`.

The next capture steps are a DPI-table capture (set known DPI values with the vendor application,
read the codes back) and a two-byte LED-mode capture; both need the vendor software or a set of
consented writes.

## Detach and reconnect

- With the mouse unplugged, `--list-devices` and `--dump-state` both reported no supported device
  and exited with code 2.
- Replugging re-enumerated the device (Bus 005 Device 004, new node ordering) and `--list-devices`
  found it again on `/dev/hidraw3`; `--dump-state` exited 0 with the same decoded values, and two
  `--dump-raw` runs after the replug were byte-identical.
- The access-failure path (exit 3 with the node's mode/owner and the udev rule) was observed before
  the rule was fixed and is covered by `tests/cli.rs`. The GUI's hotplug monitor is covered by a
  unit test (arrival, removal, edit retention).

## Reproduction

```sh
redragon-gui --report-descriptor   # collection + feature report table
redragon-gui --dump-raw            # raw register windows (run twice; must match)
redragon-gui --dump-state          # decoded settings
redragon-gui                       # GUI
```

Access rule used on this host:

```
KERNEL=="hidraw*", ATTRS{idVendor}=="04d9", ATTRS{idProduct}=="fc5c", GROUP="users", MODE="0660", TAG+="uaccess"
```
