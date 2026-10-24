//! Golden checks for the M721-Pro wire protocol.
//!
//! The expected bytes come from the vendor application's writers (decompiled
//! `RDCfg.exe`) and from a raw dump of a real mouse. The earlier build passed
//! every simulated test while the real mouse ignored its writes, because the
//! tests only compared our own encoder with our own decoder. These tests pin
//! the frames themselves.

use std::path::{Path, PathBuf};

use redragon_gui::models::Descriptor;
use redragon_gui::paths::Paths;
use redragon_gui::settings::{apply, read_state_consistent, ApplySettings, DeviceIds};
use redragon_gui::transport::frames::{CMD_BEGIN, CMD_COMMIT, CMD_WRITE};
use redragon_gui::transport::sim::SimTransport;
use redragon_gui::transport::Pacer;

/// `--dump-raw` from a real M721-Pro (profile 1 active, LED steady black).
const REAL_DUMP: &str = "\
active_profile[0] report 2 addr 0x002c len 2: 00 00
led[0] report 2 addr 0x0449 len 7: 00 00 00 01 00 02 02
led[1] report 2 addr 0x0451 len 7: ff 00 00 02 04 00 02
led[2] report 2 addr 0x0459 len 7: ff 00 00 02 04 00 02
led[3] report 2 addr 0x0461 len 7: b8 ff 00 00 04 00 02
led[4] report 2 addr 0x0469 len 7: ff 00 00 02 04 00 02
polling_hi[3] report 2 addr 0x0038 len 3: 08 00 02
polling_hi[4] report 2 addr 0x003a len 3: 02 00 1f
polling_lo[0] report 2 addr 0x0032 len 5: 01 00 02 00 02
profile_page[0] report 3 addr 0x0042 len 32: 03 00 01 0b 00 00 00 00 01 17 00 00 00 00 01 2f 00 00 00 00 01 5f 00 00 00 00 01 bc 00 00 00 00
profile_page[1] report 3 addr 0x0102 len 32: 01 00 01 0b 00 00 00 00 01 17 00 00 00 00 01 2f 00 00 00 00 01 5f 00 00 00 00 01 bc 00 00 00 00
profile_page[2] report 3 addr 0x01b2 len 32: 02 00 01 0b 00 00 00 00 01 17 00 00 00 00 01 2f 00 00 00 00 01 5f 00 00 00 00 01 bc 00 00 00 00
profile_page[3] report 3 addr 0x0262 len 32: 03 00 01 0b 00 00 00 00 01 17 00 00 00 00 01 2f 00 00 00 00 01 5f 00 00 00 00 01 bc 00 00 00 00
profile_page[4] report 3 addr 0x0312 len 32: 04 00 01 0b 00 00 00 00 01 17 00 00 00 00 01 2f 00 00 00 00 01 5f 00 00 00 00 01 bc 00 00 00 00
";

fn desc() -> Descriptor {
    let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("models/holtek-04d9-fc5c.toml");
    Descriptor::from_toml(&std::fs::read_to_string(path).expect("descriptor")).expect("valid")
}

fn sim_from_real_dump(desc: &Descriptor) -> SimTransport {
    let mut sim = SimTransport::new(desc.frame_sizes());
    for line in REAL_DUMP.lines() {
        let (head, bytes) = line.split_once(": ").expect("dump line");
        let addr = head
            .split_whitespace()
            .skip_while(|word| *word != "addr")
            .nth(1)
            .and_then(|word| u16::from_str_radix(word.trim_start_matches("0x"), 16).ok())
            .expect("address");
        let bytes: Vec<u8> = bytes
            .split_whitespace()
            .map(|b| u8::from_str_radix(b, 16).expect("hex"))
            .collect();
        sim.set_bytes(addr, &bytes);
    }
    sim
}

fn run_apply(
    sim: &mut SimTransport,
    desc: &Descriptor,
    edit: impl FnOnce(&mut redragon_gui::settings::DeviceState),
) -> redragon_gui::settings::ApplyReport {
    let tmp = tempfile::tempdir().expect("tempdir");
    let paths = Paths {
        state_dir: tmp.path().to_path_buf(),
        config_dir: tmp.path().join("config"),
        runtime_dir: tmp.path().join("run"),
    };
    let baseline = read_state_consistent(sim, desc, &Pacer::instant()).expect("read");
    let mut draft = baseline.clone();
    edit(&mut draft);
    let settings = ApplySettings {
        pacer: Pacer::instant(),
        force_stale: false,
        active_profile: baseline.active_profile,
        label: "golden".into(),
    };
    let ids = DeviceIds {
        vendor_id: 0x04d9,
        product_id: 0xfc5c,
    };
    apply(sim, desc, &baseline, &draft, ids, &paths, &settings).expect("apply")
}

/// Frames sent after the read phase, as hex strings of their first `n` bytes.
fn frames_since(sim: &SimTransport, first: usize, n: usize) -> Vec<String> {
    sim.frames[first..]
        .iter()
        .map(|frame| {
            frame
                .bytes
                .iter()
                .take(n)
                .map(|b| format!("{b:02x}"))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

#[test]
fn the_real_dump_decodes_to_the_values_seen_on_the_device() {
    let desc = desc();
    let mut sim = sim_from_real_dump(&desc);
    let state = read_state_consistent(&mut sim, &desc, &Pacer::instant()).expect("read");
    assert_eq!(state.active_profile, 0, "active profile is register 0x2C");

    let first = &state.profiles[0];
    let dpi = first.dpi.as_ref().expect("dpi");
    let values: Vec<u16> = dpi.stages.iter().map(|s| s.x).collect();
    assert_eq!(values, vec![1000, 2000, 4000, 8000, 16000]);
    assert_eq!(dpi.active, 3, "page byte 0 is the active stage (0-based)");
    assert_eq!(first.polling_hz, Some(1000), "code 1 is 1000 Hz");

    let led = first.lighting.as_ref().expect("lighting");
    assert_eq!(led.mode, "steady");
    assert_eq!(led.mode_code, 0x0102);
    assert_eq!(led.color, [0, 0, 0]);
    assert_eq!(led.brightness, 2);

    let second = state.profiles[1].lighting.as_ref().expect("lighting");
    assert_eq!(second.mode, "wave");
    assert_eq!(second.color, [255, 0, 0]);
    assert_eq!(second.speed, 4);

    // Profile 4's polling register (0x38) holds 8, which is 125 Hz.
    assert_eq!(state.profiles[3].polling_hz, Some(125));
    assert_eq!(state.profiles[3].lighting.as_ref().unwrap().mode, "off");
}

#[test]
fn an_led_change_sends_the_vendor_frames_in_a_closed_session() {
    let desc = desc();
    let mut sim = sim_from_real_dump(&desc);
    let report = run_apply(&mut sim, &desc, |draft| {
        let led = draft.profiles[0].lighting.as_mut().unwrap();
        led.set_mode(desc.lighting.as_ref().unwrap(), "breathing");
        led.color = [0x00, 0xff, 0x00];
        led.speed = 4;
        led.brightness = 3;
    });
    assert!(report.fully_confirmed(), "{report:?}");

    // Find the write session: it starts at the second F5 00 of the apply.
    let opens: Vec<usize> = sim
        .frames
        .iter()
        .enumerate()
        .filter(|(_, f)| f.bytes[1] == CMD_BEGIN && f.bytes[2] == 0)
        .map(|(i, _)| i)
        .collect();
    let write_session = opens[opens.len() - 2]; // stale check, write, verify
    let sent = frames_since(&sim, write_session, 9 + 7);
    assert_eq!(sent[0], "02 f5 00 00 00 00 00 00 00 00 00 00 00 00 00 00");
    // One record: R G B mode speed mode2 brightness at 0x0449.
    assert_eq!(sent[1], "02 f3 49 04 07 00 00 00 00 ff 00 01 04 04 03 00");
    // The refresh sequence is {02 F1 02 flag}, flag in byte 3.
    let refresh: Vec<&String> = sent.iter().filter(|s| s.starts_with("02 f1")).collect();
    let flags: Vec<&str> = refresh.iter().map(|s| &s[6..11]).collect();
    assert_eq!(flags, vec!["02 04", "02 01", "02 02", "02 08", "02 10"]);
    // Writes are followed by a refresh, then the session closes.
    assert_eq!(
        sent[sent.len() - 1],
        "02 f5 01 00 00 00 00 00 00 00 00 00 00 00 00 00"
    );
    assert_eq!(sim.lost_writes, 0);
    assert!(!sim.in_session());
}

#[test]
fn a_dpi_stage_is_one_five_byte_record_and_the_active_stage_is_one_byte() {
    let desc = desc();
    let mut sim = sim_from_real_dump(&desc);
    run_apply(&mut sim, &desc, |draft| {
        let dpi = draft.profiles[0].dpi.as_mut().unwrap();
        dpi.stages[1].x = 1600;
        dpi.stages[1].y = 1600;
        dpi.active = 1;
    });
    // Stage 2 of profile 1 lives at page 0x42 + 2 + 6.
    let record = sim.bytes_at(0x004a, 5);
    assert_eq!(record[0], 1, "enabled");
    assert_eq!(record[1..3], [0x12, 0x00], "vendor code for 1600 DPI");
    assert_eq!(record[3..5], [0, 0]);
    assert_eq!(sim.bytes_at(0x0042, 1), vec![1], "active stage byte");
    let writes: Vec<_> = sim
        .frames
        .iter()
        .filter(|f| f.bytes[1] == CMD_WRITE)
        .map(|f| (f.bytes[2], f.bytes[3], f.bytes[4]))
        .collect();
    assert!(writes.contains(&(0x4a, 0x00, 5)), "{writes:?}");
    assert!(writes.contains(&(0x42, 0x00, 1)), "{writes:?}");
}

#[test]
fn polling_codes_are_the_interval_in_milliseconds() {
    let desc = desc();
    for (hz, code) in [(125u16, 8u8), (250, 4), (500, 2), (1000, 1)] {
        let mut sim = sim_from_real_dump(&desc);
        // Profile 2's register is 0x34 and currently holds 500 Hz (code 2).
        let before = sim.bytes_at(0x34, 1)[0];
        if before == code {
            continue;
        }
        run_apply(&mut sim, &desc, |draft| {
            draft.profiles[1].polling_hz = Some(hz);
        });
        assert_eq!(sim.bytes_at(0x34, 1), vec![code], "{hz} Hz");
    }
}

#[test]
fn every_refresh_frame_the_engine_sends_has_the_vendor_layout() {
    let desc = desc();
    let mut sim = sim_from_real_dump(&desc);
    run_apply(&mut sim, &desc, |draft| {
        draft.profiles[2].polling_hz = Some(250);
    });
    let refreshes: Vec<_> = sim
        .frames
        .iter()
        .filter(|f| f.bytes[1] == CMD_COMMIT)
        .collect();
    assert_eq!(refreshes.len(), 5);
    for frame in refreshes {
        assert_eq!(frame.bytes[2], 0x02, "mask byte");
        assert!(frame.bytes[3].is_power_of_two(), "flag byte");
        assert!(frame.bytes[4..].iter().all(|b| *b == 0));
    }
}
