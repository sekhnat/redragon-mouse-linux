# REDRAGON Gaming Mouse — Reverse-Engineering Report

**Target:** `"REDRAGON Gaming Mouse"` Windows configuration application + HID "drivers" shipped in this repo (705 files: `RDCfg.exe`, `HIDApi.dll`, `HidDevice.dll`, Qt5 runtime, per-model `Config.ini` / `.prof` trees)
**Goal:** Understand the application, its driver stack and its configuration protocol well enough to (a) document how it works and (b) configure the mouse on **this Linux system**.
**Date:** 2026-10-24 · **Host:** CachyOS x86_64, kernel 7.3.0-3-cachyos
**Method:** Static analysis (Ghidra 12.1.2 headless decompilation, objdump, pefile, strings), USB/sysfs live observation (read-only), primary-source research (upstream FOSS projects, kernel sources). **No writes were sent to the hardware** (no root hidraw access on this host; nothing was modified on the mouse).

> Reproduction: decompiled sources produced for this report are at `/tmp/ghidra_redragon/{RDCfg,HidDevice,HIDApi}.c` (generated with Ghidra headless + a decompile-all script; not committed). Ghidra's headless runs initially crashed on a stale script cache (`~/.config/ghidra/ghidra_12.1.2_DEV/osgi/compiled-bundles/…/WindowsResourceReference.class`, compiled by a Java-27 GUI session) — worked around by deleting the cache entry and disabling the `WindowsResourceReference` analyzer via pre-script.

---

## 1. Executive summary

1. **The app is a 32-bit MinGW/Qt5 GUI (`RDCfg.exe`, built 2022-07-27) that talks to the mouse directly through the open-source `hidapi` library** — renamed `HIDApi.dll` (built 2020-05-27; licenses `LICENSE-hidapi.txt`/`LICENSE-bsd.txt` confirm Signal11 hidapi provenance). It performs **no** privileged driver operations: all configuration is ordinary HID **feature reports** on the mouse's vendor interface (`mi_02`).
2. **`HidDevice.dll` (2019-04-22, debug-CRT MSVC build) is vestigial** — RDCfg.exe contains no import or string reference to it. Its exports (`HidDevice_Open/Close/SetMultiFeature/GetMultiFeature/MultiWrite/SetReadCallBack/SetMultiCaptureDevice`) implement an older, richer HID stack (SetupAPI + `HID.DLL` + hidden window message pump + worker threads), which was analyzed anyway and is documented below.
3. **Device support is data-driven, not compiled in.** The app reads a `Mouse_List` whitelist (up to 200 entries: VID, PID, two usage-page/usage pairs, a bcdDevice version range, product name) from `Config.ini` via QSettings, and matches each `hid_enumerate()` device whose path contains `…&mi_02&col01` (usage page `0x0C`, consumer) and `…&mi_02&col02` (usage page `0xFF00`, vendor-defined).
4. **The wire protocol is a memory-mapped config-file protocol over HID feature reports** with three report sizes:
   - Report ID **2** (16-byte frames): command frames `{02, cmd, addr_lo, addr_hi, len, 00 00 00, payload[8]}` with `cmd = 0xF5` (begin upload), `0xF3` (write register), `0xF1` (commit/apply).
   - Report ID **3** (64-byte frame): the scroll-speed block at address `0x20`.
   - Report ID **4** (1024-byte frame): macro bodies at address `0x478 + 200 × button`.
   The register space covers DPI stages, LED mode/color/speed/brightness, polling rate, button keymaps and macros — full map in §5.
5. **The mouse plugged into this system is `04d9:fc5c` = Redragon M721-Pro "Lonewolf2"** (identification via dokutan/mouse_m908's support table). It presents the classic 3-interface Holtek layout (boot mouse, consumer/multimedia keyboard, vendor config interface). **Linux needs no kernel quirk for it**; the vendor interface is exposed as `/dev/hidraw3` and is fully usable by userspace tools.
6. **On this Linux system you can already configure this mouse today** with FOSS tools: `mouse_m908` (C++, *experimental* support for `0x04d9:0xfc5c`), optionally with the `mouse-config-gui` GTK front end. The protocol used by those tools is byte-compatible with what this Windows app uses (§5.4), independently confirming this analysis. The app build in this repo, however, would **not** detect this particular mouse: its shipped `Config.ini` whitelists only PIDs `0xFCA9`, `0xFCB6`, `0xFC9F`.

---

## 2. Hardware found on this system (live, read-only)

```
Bus 005 Device 002: ID 04d9:fc5c Holtek Semiconductor, Inc. USB Gaming Mouse   (lsusb)
bcdDevice 0120, USB 2.00 Full-Speed, 100 mA, iProduct "USB Gaming Mouse"      (sysfs, lsusb -v)
```

| Interface | Class/Sub/Proto | Report descriptor | Role |
|---|---|---|---|
| `5-1:1.0` | HID boot **mouse** (03/01/02), EP 0x81 IN | 67 B | actual pointer input |
| `5-1:1.1` | HID boot **keyboard** (03/01/01), EP 0x82 IN | 47 B | multimedia/consumer keys |
| `5-1:1.2` | HID **vendor** (03/00/00), EP 0x83 IN, 8-byte, bInterval 4 | 106 B | **configuration channel** (`mi_02`) |

- Sources: `lsusb`, `lsusb -v -d 04d9:fc5c` output (this host, 2026-10-24); `/sys/bus/usb/devices/5-1/*`.
- `/dev/hidraw1..3` map to the three interfaces (via `/dev/input/by-id/` symlinks); all are `root:root 0600` on this host — userspace access requires root or a udev rule (§7.3).
- The 106-byte report descriptor of interface 2 could not be dumped without root; its structure is instead inferred from the application code (report IDs 2/3/4, sizes 16/64/1024 — §5.2) and is consistent with `hid-holtek-mouse.c`-era descriptors.
- `04d9:fc5c` has **no entry in the kernel's `hid-ids.h`** (checked `drivers/hid/hid-ids.h` @ master: only Holtek ALT PIDs `a04a/a067/a070/a072/a081/a0c2` (older gaming mice whose oversized report descriptors get fixed up by `drivers/hid/hid-holtek-mouse.c`) and keyboards `a055/a096/a292/a293` appear). The mouse therefore binds to the generic USB HID driver without quirks.
- Model identification: mouse_m908's supported-mice table lists `Redragon M721-Pro Lonewolf2 | experimental | 0x04d9:0xfc5c` (README.md). The `ernesto-elsaesser/usb-mouse-config` PID table lists `fc5c` as "M721".

---

## 3. Package inventory & provenance

```
REDRAGON Gaming Mouse/
├── RDCfg.exe                1.2 MB  PE32 i386 GUI, MinGW-GCC (imports msvcrt.dll, libgcc_s_dw2-1.dll,
│                                   libstdc++-6.dll; .eh_frame section; PE timestamp 0x62e13b75 = 2022-07-27)
│                                   Version resource: ProductName "RDCfg", all version fields 0.0.0.0
├── HIDApi.dll                12.5 KB PE32 DLL "hidapi.dll", timestamp 0x5ece20af = 2020-05-27
│                                   exports: hid_init/hid_exit/hid_enumerate/hid_free_enumeration/hid_open_path/
│                                   hid_close/hid_read/hid_read_timeout/hid_write/hid_set_nonblocking/
│                                   hid_get_feature_report/hid_send_feature_report/hid_get_*_string/…
│                                   imports: SETUPAPI.dll, KERNEL32.dll, MSVCR90.dll  → stock Signal11 hidapi
├── HidDevice.dll             55 KB  PE32 DLL, timestamp 0x5cbd61d0 = 2019-04-22, MSVC 9.0 **debug** build
│                                   (imports MSVCR90D.dll, RTC stack checks, OutputDebugStringA "error N" traces)
│                                   exports: HidDevice_Open/Close/GetMultiFeature/MultiWrite/SetMultiCaptureDevice/
│                                   SetMultiFeature/SetReadCallBack; imports HID.DLL, SETUPAPI.dll
├── Qt5{Core,Gui,Widgets,Svg}.dll, D3Dcompiler_47.dll, platforms/styles/imageformats/iconengines/…
├── Config.ini (root, 236 KB) — app-level QSettings state: [Config] LED defaults, [FONT_LIST],
│                                   [Mouse_List] supported-mouse whitelist, [PROFILE_0..4] last profiles
├── LICENSE.txt (Qt via LGPL dynamic linking notice), LICENSE-hidapi.txt, LICENSE-bsd.txt (hidapi, Alan Ott)
├── lang/{en,sc,tc}.qm      — Qt compiled translations (UI strings; source of feature list in §6)
├── skin/                    — UI images incl. skin/mouse/<MODEL>.png for ~58 models
└── REDRAGON Gaming Mouse/    — per-model data folders, one per supported model:
    ├── <MODEL>/Config.ini   — per-model profile list + defaults (54 models, 2 structural variants)
    ├── <MODEL>/Profile/{DefaultProfile,Profile 1..5}.prof — per-profile settings (INI format)
    ├── MacroDB/             — empty; populated when macros are created
    ├── Profile/            — generic default profiles
    └── 2710-G … TITANOBOA4, incl. sensor-suffixed dirs (2713 3325, 2903 3389/3395/3399 = PixArt PMW33xx sensors),
        M711(-FPS-1/-2), M715, M716, M720-RGB, M721-PRO, M721-RGB(2), M808-RGB, M912/990/998-RGB, …
        (leftover vendor IDE metadata: several `<MODEL>/.idea/*.iml` PYTHON_MODULE files)
```

Evidence: `file`, `objdump -p` (import/export tables), `pefile` (timestamps/version resources), `strings` (export names, UI strings), `ls -R` of the tree. hidapi provenance: `LICENSE-hidapi.txt` ("HIDAPI can be used under one of three licenses…"), `LICENSE-bsd.txt` ("Copyright (c) 2010, Alan Ott, Signal 11 Software"). Qt provenance: `LICENSE.txt` ("The Software used open-source licensed QT (under the GNU Lesser General Public License (LGPL) version 3) by dynamic link library").

---

## 4. Application architecture

### 4.1 RDCfg.exe (the app)

Qt5 Widgets application (class names from strings/symbols: `MousecfgWidget`, `WillMessageBox`, `WillGunEditDialog`, `WillGunsetDialog`, `WillMacroSelect`, `SVColorArea`, …). Single-instance mutex `"REDRAGON Gaming Mouse Mutex"`. The entire device stack is:

```
RDCfg.exe ── hidapi (HIDApi.dll, SetupAPI) ──> Windows HID (hid.dll / kernel)
```

Imported and *actually used* hidapi functions (call sites verified in disassembly, through MinGW import thunks at `0x497a00-0x497a50`): `hid_init`, `hid_exit`, `hid_enumerate`, `hid_free_enumeration`, `hid_open_path`, `hid_close`, `hid_read`, `hid_set_nonblocking`, `hid_send_feature_report`, `hid_get_feature_report`, `hid_get_release_number`. `hid_write` is exported by HIDApi.dll but not referenced by the app — **all configuration traffic is feature reports**.

### 4.2 Device detection (`FUN_00456c50`, `FUN_00456a50`, `FUN_004577e0` in RDCfg.c)

1. On startup the app loads the whitelist into a `200 × 808-byte` record array from QSettings keys `Mouse_List/{Count,VID_%d,PID_%d,USAGE_0_%d,USAGEPAGE_0_%d,USAGE_1_%d,USAGEPAGE_1_%d,VER_MIN_%d,VER_MAX_%d,PRODUCT_NAME_%d}` (string constants at `0x4ba620+`; loader `FUN_00456c50` at RDCfg.c:33846-33928).
2. `hid_enumerate(0,0)` walks every HID device; for each, the record with matching VID/PID is found and **two collection criteria** are checked against the device's top-level-collection usage/usage-page (RDCfg.c:33701 `FUN_00456a50`):
   - `usage_page == USAGEPAGE_0 && usage == USAGE_0` → marks the **col01** path matched (consumer page `0x0C`, usage `0x01` per the shipped INI),
   - `usage_page == USAGEPAGE_1 && usage == USAGE_1` → **col02** (vendor page `0xFF00`, usage `0x01`),
   - plus a **bcdDevice version-range check**: `VER_MAX == 0 || (VER_MIN ≤ release ≤ VER_MAX)` (release = `hid_device_info.release_number`).
   The companion regexes on device paths, `HID#VID_(?<vid>\w{4})&PID_(?<pid>\w{4})&mi_02&col01` and `…col02`, appear in the binary's strings — on Windows, hidapi device paths contain exactly these `mi_XX&colNN` segments, so the app only ever considers the vendor interface (`mi_02`) of matching mice.
3. On connect (`FUN_004577e0`), it opens **both** collections: `DAT_004f9b7c = hid_open_path(col01_path)`, `DAT_004f9b78 = hid_open_path(col02_path)`. **All configuration writes go to col02** (the vendor collection); a worker thread polls **col01** with `hid_read` in a ~100 ms loop (`call 0x497a20` = `hid_set_nonblocking`, `call 0x497a28` = `hid_read` at `0x485b05/0x485bc3`) to capture live button events for the UI.

The shipped root `Config.ini` whitelist (`[Mouse_List]`, `Count=3`) — decimal values as stored, hex in parentheses:

| idx | VID | PID | col01 usage | col02 usage | VER range |
|---|---|---|---|---|---|
| 0 | 1241 (`0x04D9`) | 64585 (`0xFCA9`) | 12/1 (`0x0C/0x01`) | 65440/1 (`0xFF00/0x01`) | 768–1023 (`0x300–0x3FF`) |
| 1 | 1241 | 64598 (`0xFCB6`) | 12/1 | 65440/1 | 256–511 (`0x100–0x1FF`) |
| 2 | 1241 | 64575 (`0xFC9F`) | 12/1 | 65440/1 | 0–0 (**unchecked** — `VER_MAX=0` disables the test) |

These three PIDs do not appear in public USB-ID databases (DeviceHunt lists only up to `FC62`); they are most plausibly newer-generation models (the app build is from 2022 and the tree contains PixArt-3399-era model folders), but their model identities are **unconfirmed**. Notably, the attached mouse `0xFC5C` (M721-Pro) is *not* in this shipped list, and no VID/PID whitelist table exists inside the binary (verified: no `d9 04`/`fc??` VID-PID pair patterns in the data sections; the only `0xfc??` immediates are unrelated small constants) — **the list lives entirely in `Config.ini`**, so a stale/trimmed INI shrinks the app's device support even though the binary could speak the protocol. (A newer official app build's `Config.ini` with a `0xFC5C` entry would presumably enable this binary against the attached mouse — plausible but untested.)

### 4.3 HIDApi.dll

Stock [hidapi](https://github.com/libusb/hidapi) (Signal11 lineage): Windows implementation over SetupAPI + `hid.dll`, exporting the classic `hid_*` C API. Nothing Redragon-specific was found (compared against the export list and import set: `SETUPAPI.dll`, `KERNEL32.dll`, `MSVCR90.dll`). It is a *library*, not a driver — the "driver" is Windows' own HID class stack.

### 4.4 HidDevice.dll (legacy, unused by RDCfg.exe)

An older, more elaborate device layer, linked against `MSVCR90D.dll` (debug CRT). Analyzed for completeness:

- **`HidDevice_Open` → `FUN_10012990`:** records callback window/context, `GetVersionExA`, spawns a hidden-window worker thread (`FUN_100116d0`: window class `"Thread In DLL"`), then initial enumeration (`FUN_10013d30`).
- **Hot-plug:** the window procedure (`FUN_100119f0`) handles `WM_DEVICECHANGE` (`0x219` arrival / `0x8004` remove); on arrival it `CreateFileA(path, GENERIC_READ|GENERIC_WRITE, …, FILE_FLAG_OVERLAPPED)`s the interface, does `HidD_GetPreparsedData`/`HidD_GetAttributes`/`HidP_GetCaps`, and matches against a **capture table** previously installed by the app via `HidDevice_SetMultiCaptureDevice` (`FUN_10012b90`: memcpy of `0x23F4` bytes = `DWORD count + up to 100 entries × 92 bytes`, entry = `{vid, pid, pair_count, {usage, usage_page} × 10}`) — compare logic in `FUN_10013a80`. The GUI is notified via `SendMessage` with a 16-byte struct `{event, usage, usagepage, vid, pid, version}` (`FUN_100120c0`) — the **version-range filtering is the caller's job**, same as in the newer app.
- **Transport (`HidDevice_SetMultiFeature` → `FUN_10013370`, `GetMultiFeature` → `FUN_10013640`, `MultiWrite` → `FUN_10012e60`):** `HidD_SetFeature` / `HidD_GetFeature` for feature reports; `MultiWrite` additionally writes **output** reports via overlapped `WriteFile` (report-ID-prefixed buffer, zero-filled to the collection's report length).
- **Read path:** per-device worker thread (`FUN_10012270` registers a handle-notification, starts reader thread, links the device into a global list); full `HidP_GetSpecificButtonCaps`/`HidP_GetSpecificValueCaps` parsing (`FUN_10015b70`) builds button/value tables used to decode input reports for the `HidDevice_SetReadCallBack` callback.

The design mirrors what RDCfg.exe now does inline through hidapi — evidence that the vendor replaced this DLL with direct hidapi usage between 2019 and 2022. No reference to `HidDevice.dll` exists anywhere in RDCfg.exe (checked imports **and** strings).

---

## 5. The configuration wire protocol

### 5.1 Transport

All configuration is **HID SET_REPORT (Feature)** traffic on **interface `mi_02`, collection 2 (usage page `0xFF00`)**. In hidapi terms: `hid_send_feature_report(handle_col02, buf, len)` with `buf[0]` = report ID. In raw USB terms (as used by mouse_m908 on Linux): `bmRequestType 0x21, bRequest 0x09 (SET_REPORT), wValue 0x03<report-id> (Feature), wIndex 0x0002 (interface 2)`.

### 5.2 Frame formats (from RDCfg.exe decompilation, cross-checked against mouse_m908 captures)

**Report ID 2 — 16-byte register frames.** Layout (payload after the report-ID byte; `·` = zero):

```
byte:  0     1      2        3        4     5 6 7   8 … 15
     +------+----+--------+--------+-----+-------+-----------+
     | 0x02 | cmd| addr_lo| addr_hi| len | 00 00 |  data[8]  |
     +------+----+--------+--------+-----+-------+-----------+
cmd:  0xF5 = begin settings upload (byte 2 = profile index; frame {02 F5 <p> 0…})   [FUN_004578d0]
      0xF3 = write register at 16-bit address `addr`                                  [many]
      0xF1 = commit / switch active profile ({02 F1 01 00 00} style)                  [e.g. RDCfg.c:35848+]
len:  payload length marker (0/1/2/4/6/… varies per frame class)
```
`Sleep(10)` between frames. Verified examples in the app: `{02 F5 <profile>}` (FUN_004578d0), `{02 F3 3E 00 02 …}` "SendKeyMapMode" (FUN_00457970), `{02 F3 46 00 04 02 …}` LED-mode frame per profile (FUN_00457a20), `{02 F3 <lo> <hi> 01 00 00 00 <keybyte>}` keymap frames (FUN_00458670 / FUN_0045be50, addresses `0x42 + offset_table[profile] + 4×slot`, per-profile base offsets table at `0x4bb830` = `{64, 256, 432, 608, 784}`).

**Report ID 3 — 64-byte block.** `{03 F3 20 00 0A 00 00 00 <64-byte body>}` — "SendScrollSpeed" (FUN_00458580): scroll speed per profile at body offsets `8, 10, 12, 14, …` (one byte per profile, spaced by 2). Sent with `Sleep(100)` after. Identical to mouse_m908's `_c_data_settings_2` (wValue `0x0303`).

**Report ID 4 — 1024-byte macro frames.** `SendButtonRefresh` (FUN_0045a9f0): `{04 F3 <addr_lo> <addr_hi> C8 00 00 00 <macro body>}` with address `0x478 + 200 × button_index`; macro body = 15-byte records (delay/key entries, e.g. `{06 01 …}`), sent as one 1024-byte feature report, then **read back with `hid_get_feature_report` for verification** (6 read sites in the same function).

**Read path.** This app build reads feature reports only for macro verification; the GUI never reads general settings back (state is kept in the INI files). mouse_m908 demonstrates that these mice also support **GET_REPORT** (`bmRequestType 0xA1, bRequest 0x01, wValue 0x03<id>, wIndex 0x0002`) to read any register back — the basis of its `read_settings()`.

### 5.3 Register map (as used by this protocol family)

Reconstructed from mouse_m908's `include/m721/data.cpp` register captures (authoritative for `0x04d9:0xfc5c`, the attached mouse's PID) and consistent with the app's frame writers:

| Function | Address / frame | Encoding |
|---|---|---|
| Profile switch | `0xF5` begin + `{02 F1 01…}` commit; profile row `{02 F3 42 00 02 … payload[0]=profile}` | profile index 0-4 |
| Scroll speed | report 3 @ `0x20`, body bytes `8+2i` | 1 byte per profile (1..?) |
| LED mode+color+speed | report 2, per profile: `0x49, 0x51, 0x59, 0x61, …` (len 6): payload `{R,G,B, speed, mode1, mode2}` | e.g. static white = `FF FF FF 02 08 01` |
| LED brightness | paired rows `0x4F, 0x57, 0x5F, …` (len 1) | brightness level |
| Polling rate | rows 13/14 of settings_1: profiles 0-2 at payload `{0,2,4}` of row 13, profiles 3-4 at `{2+2i}` of row 14 | `0x01/0x02/0x04/0x08` = 125/250/500/1000 Hz |
| DPI stages | per stage `s`, profile `p`: address `0x0044 + 0xC0·p + 0x06·s`, len 4: `{enabled, dpi_code_lo, dpi_code_hi, 0}` | dpi codes are a non-linear lookup (e.g. 1000→`0x16 0x00`, 5000→`0x73 0x00`; full table in mouse_m908 `data.cpp`) |
| Button keymap | per profile bases `0x0082 / 0x0042 / 0x00F2 / 0x00A2 / 0x0052` (model-dependent), 4 bytes per button region entry, plus multi-key combo frames (`{02 F3 lo hi 01 00 00 00 <key>}`) | key-code tables in app data `0x4bb324` |
| Macros | report 4 @ `0x478 + 200×button`, 200-byte records, 15-byte per-action entries | `{type, …}` action entries |

### 5.4 Independent confirmation

mouse_m908's frames for the M721 (`0xfc5c`) are byte-identical in structure to the frames this app emits — e.g. mouse_m908 `include/m721/writers.cpp` sends `libusb_control_transfer(_i_handle, 0x21, 0x09, 0x0302, 0x0002, buffer[i], 16, 1000)` for each of the same 16-byte frames, patches the same payload offsets (`buffer2[8+(2*i)]` scroll speeds, `buffer1[3+(2*i)][8..13]` LED fields, `buffer3[7+(5*i)+j][8..10]` DPI, `buffer3[35+(10*i)+j][8..11]` keymap), and `include/m721/data.cpp` stores the captured default frames `{02 F5 …}`, `{02 F3 3E 00 02 …}`, `{02 F3 46 04 02 …}` etc. Those `_c_data_*` arrays were captured from the official software's USB traffic — i.e., **from the same protocol this app speaks**. The one difference: mouse_m908's M721 writer addresses keymap rows as full 16-byte frames at fixed row indices rather than the app's per-key addressed frames; the register *locations* agree.

---

## 6. On-disk configuration formats (plain INI)

- **Root `Config.ini`** — QSettings (INI, UTF-8, no BOM): `[Config]` global UI state + 8 LED-mode presets (HSV/brightness/breath), `[FONT_LIST]` UI fonts, `[Mouse_List]` the device whitelist (§4.2), `[PROFILE_0..4]` the last-applied per-profile state for the connected model.
- **Per-model `REDRAGON Gaming Mouse/<MODEL>/Config.ini`** — `[Config]` `ProfileID`, `AngleValue` (angle-snapping), `LiftHigh` (lift-off distance), `LedMode` (RGB models only) + `[MACRO_LIST]` (16 name slots; present in 53 of 54 models — the non-RGB M2761-F variant omits it) + `[PROFILE_LIST]` (5 profiles).
- **`<MODEL>/Profile/*.prof`** — INI (CRLF), one file per profile: `pollrate` (125/250/500/1000), `dpi` (active stage), `DPI_INFO_0..4_{used,link,xy,x,y}` (5 stages, X/Y-linkable), `mousespeed`, `dbclickspeed`, `scrollispage`, `scrollspeed`, `accelenable`, and `BUTTON_INFO_0..N_{ID,Fun,LoopNumber,KeyNumber,Res1..4,macroname}` with up to 62 `KeyInfo_i_{map,L,H}` slots per button (multi-key combos). Schema drift between app generations is visible in the files themselves (`KeyInfo_i_` vs `KeyInfo_i_L`, `acee` vs `aceel`).
- **UI feature set** (from `lang/en.qm` strings): profiles (5+default, load/save/export `.prof`), macros (`*.MSMACRO`, 20 max, import/export), "gun" burst-fire editor (bullets/delay), DPI ±/switch/sniper, polling rate, LED modes (7-color breathing, rainbow, reactive, wave, flash, full-lighted, LED switch, brightness, front/side/bottom zones), lift-off distance (1/2/3 mm), angle snap, multimedia/browser/Desktop button functions, combo keys.
- The app also touches the **OS**: `SystemParametersInfoW(SPI_GETMOUSESPEED/SPI_SETMOUSESPEED)` (`FUN_0045be00`/`FUN_0045bd70`) — the "mouse speed" setting is applied to *Windows* pointer speed, not just the device (irrelevant on Linux; libinput owns pointer speed).

---

## 7. Linux status & recommendations for this system

### 7.1 Kernel

No kernel quirk or driver needed for `04d9:fc5c`: generic `usbhid` binds the mouse/keyboard interfaces, and the vendor interface (`5-1:1.2`) is exposed as a hidraw node usable by any userspace tool. (Older Holtek PIDs `a067/a04a/a081/a072/a0c2` do have descriptor fixups in `drivers/hid/hid-holtek-mouse.c`; `fc5c` is not among them — `hid-ids.h` checked.)

### 7.2 Existing FOSS tools (primary sources)

- **[dokutan/mouse_m908](https://github.com/dokutan/mouse_m908)** — C++ CLI, GPL-3.0, drives Redragon/Holtek mice over raw USB control transfers (libusb). **Supports `0x04d9:0xfc5c` (M721-Pro) experimentally** — the exact mouse on this host. Its `include/m721/*` backend implements profile/LED/DPI/report-rate/keymap/macro read+write for this PID. Linux/BSD/Haiku; has a `--kernel-driver` note and ships a udev rule (`mouse_m908.rules`).
- **[robinsiebler/mouse-config-gui](https://github.com/robinsiebler/mouse-config-gui)** — GTK4/libadwaita GUI front end over mouse_m908 (LED, DPI, report rate, profiles, button mapping, macro editor). Capability data for the other models was derived from mouse_m908's source, not verified on hardware.
- **[ernesto-elsaesser/usb-mouse-config](https://github.com/ernesto-elsaesser/usb-mouse-config)** — Python, similar protocol implementation, large PID table.
- **OpenRGB** — LED-only control for several Redragon Holtek mice (`RedragonMouseController`; newer M711-FPS-1 `fc62` uses usage page `0xFFA0` on interface 2 — a later-generation vendor page than this app's `0xFF00`).

### 7.3 Practical recipe on this machine (M721-Pro, `04d9:fc5c`)

1. Install `mouse_m908` (Arch: AUR PKGBUILD exists upstream; repo includes it) or build via CMake.
2. Grant hidraw access (mouse_m908 needs the vendor interface node — `/dev/hidraw3` here, recheck after replug since numbering changes):
   ```
   # udev rule (mouse_m908 ships one):
   KERNEL=="hidraw*", ATTRS{idVendor}=="04d9", ATTRS{idProduct}=="fc5c", MODE="0660", TAG+="uaccess"
   ```
   (Without root, `/dev/hidraw*` is `0600 root:root` on this host — verified.)
3. Read current state first (`mouse_m908 -R`), save a backup, then apply changes (`mouse_m908 -c file.ini`). Beware the upstream safety note: bad writes can make the mouse unusable (recovery = reflash/official app).
4. Optional GUI: `mouse-config-gui` on top of mouse_m908.
5. If instead you want to talk to it directly: `ioctl(HIDIOCSFEATURE)` with the §5.2 frames on the vendor-interface hidraw node; `HIDIOCGFEATURE` to read registers back (this app build never does the latter for settings, but the hardware supports it per mouse_m908).

### 7.4 Running the original Windows app on Linux

The package is a portable Qt5/MinGW app with no installer and no kernel driver component — it runs under Wine, but its device layer enumerates via Windows SetupAPI device paths (`\\?\hid#vid_…&mi_02&col01#…`), which Wine cannot map to Linux hidraw without a hidapi-over-hidraw shim (wine's hidapi is USB-level and generally cannot reach raw HID feature SET_REPORT on arbitrary interfaces reliably). The FOSS route above is the practical path; the app's value here is as the *protocol oracle* (this report).

---

## 8. Unknowns / open questions

1. **Which models are PIDs `0xFCA9`, `0xFCB6`, `0xFC9F`?** Not in public USB databases; likely 2022-generation models (possibly the PixArt-3399 folder models), unconfirmed. The shipped `[Mouse_List]` also may simply be a stale testing leftover rather than this build's intended whitelist.
2. **Whether this exact RDCfg.exe build would accept the attached M721-Pro if given a `Mouse_List` entry for `0xFC5C`** — likely (matching is data-driven; the 2022 build speaks the same F1/F3/F5 register protocol per §5), but untested (would require adding an INI entry and running the Windows app under Wine with real HID access, or live writes on Linux).
3. **Vendor interface report descriptor bytes** (106 B) — needs one privileged `usbhid-dump`/`hid-recorder` run to confirm the exact report-ID/size table; currently inferred from app code + mouse_m908.
4. **`HidDevice.dll`'s `SetMultiCaptureDevice` entry semantics** (92-byte entries incl. 10 usage pairs) were derived from code; whether the app ever shipped a 100-entry table is unknown (the DLL is unused by this app build).
5. **Live GET_REPORT read-back behavior of the M721-Pro for settings registers** (mouse_m908 marks fc5c "experimental: nothing is tested").

---

## 9. Source index

**Local artifacts** (relative to repo root): `REDRAGON Gaming Mouse/RDCfg.exe` (imports/thunks/strings; PE timestamp 0x62e13b75), `…/HIDApi.dll` (export table = hidapi C API; LICENSE-hidapi.txt, LICENSE-bsd.txt), `…/HidDevice.dll` (exports; imports MSVCR90D.dll), `…/Config.ini` ([Mouse_List], [Config], [PROFILE_*]), `…/REDRAGON Gaming Mouse/*/Config.ini` + `Profile/*.prof`, `…/lang/en.qm` (UI strings), `…/skin/mouse/` (per-model images).

**Decompilation evidence** (generated during this analysis, Ghidra 12.1.2 headless): `/tmp/ghidra_redragon/RDCfg.c` (function addresses cited in §4-§5: `FUN_00456a50` enumeration/matching, `FUN_00456c50` whitelist loader, `FUN_004577e0/00457870` open/close, `FUN_004578d0` F5 frame, `FUN_00457970` keymap-mode, `FUN_00457a20` LED frames, `FUN_00458580` report-3 scroll speed, `FUN_00458670`/`FUN_0045be50` keymap frames, `FUN_0045a9f0` report-4 macros + read-back verify, `FUN_0045be00/0045bd70` SPI pointer speed), `/tmp/ghidra_redragon/HidDevice.c` (§4.4 functions), raw disassembly `/tmp/rdcfg_dis.txt` (hid_read thread at `0x485b05/0x485bc3`).

**Live system** (this host, 2026-10-24): `lsusb`; `lsusb -v -d 04d9:fc5c`; `/sys/bus/usb/devices/5-1/**`; `/dev/hidraw*` permissions; `uname -a`.

**Upstream FOSS / kernel (primary sources):**
- dokutan/mouse_m908 — README (supported-mice table: `0x04d9:0xfc5c` = M721-Pro, experimental); `include/m721/{data.cpp,writers.cpp,readers.cpp,constructor.cpp}` (frame tables, transfer parameters, DPI/key-code tables).
- Linux kernel `drivers/hid/hid-ids.h` (`USB_VENDOR_ID_HOLTEK_ALT 0x04d9`, device list; `USB_VENDOR_ID_HOLTEK 0x1241`) and `drivers/hid/hid-holtek-mouse.c` (older-PID descriptor fixups).
- ernesto-elsaesser/usb-mouse-config README (Redragon PID↔model table).
- robinsiebler/mouse-config-gui (GUI over mouse_m908; capability-data caveat).
- OpenRGB issue 1669 / MR 3459 (M808 Storm `fc5f`, M711-FPS-1 `fc62`, usage page `0xFFA0`, interface 2 — later-generation detection pattern).
- libusb/hidapi (hidapi API semantics; Windows device-path format).
- DeviceHunt USB vendor 04D9 database (public PID↔model names).