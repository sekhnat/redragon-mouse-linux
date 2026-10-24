//! End-to-end simulated session: detect, read, edit, apply, verify, restore.

use std::path::{Path, PathBuf};

use redragon_gui::device;
use redragon_gui::models::{Descriptor, Registry, Resolution};
use redragon_gui::paths::Paths;
use redragon_gui::settings::{
    apply, apply_backup, backup, encode_ops, read_state_consistent, seed_image, ApplyReport,
    ApplySettings, DeviceIds,
};
use redragon_gui::transport::sim::SimTransport;
use redragon_gui::transport::Pacer;

fn models_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("models")
}

fn paths(tmp: &tempfile::TempDir) -> Paths {
    Paths {
        state_dir: tmp.path().to_path_buf(),
        config_dir: tmp.path().join("config"),
        runtime_dir: tmp.path().join("run"),
    }
}

fn sim_for(desc: &Descriptor) -> SimTransport {
    let image = seed_image(desc);
    let mut sim = SimTransport::new(desc.frame_sizes());
    for (_, window) in image.iter() {
        sim.set_bytes(window.addr, &window.bytes);
    }
    sim
}

fn ids() -> DeviceIds {
    DeviceIds {
        vendor_id: 0xffff,
        product_id: 0,
    }
}

fn settings() -> ApplySettings {
    ApplySettings {
        pacer: Pacer::instant(),
        force_stale: false,
        active_profile: 0,
        label: "integration test".into(),
    }
}

fn walk_round_trip(desc: Descriptor, edits: impl FnOnce(&mut redragon_gui::settings::DeviceState)) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let paths = paths(&tmp);
    let mut sim = sim_for(&desc);

    let baseline = read_state_consistent(&mut sim, &desc, &Pacer::instant()).expect("read");
    let mut draft = baseline.clone();
    edits(&mut draft);
    assert!(
        !encode_ops(&desc, &draft).is_empty(),
        "the draft must produce operations"
    );

    let report = apply(
        &mut sim,
        &desc,
        &baseline,
        &draft,
        ids(),
        &paths,
        &settings(),
    )
    .expect("apply");
    assert!(report.fully_confirmed(), "{report:?}");
    assert!(report.backup.as_ref().is_some_and(|path| path.is_file()));

    let after = read_state_consistent(&mut sim, &desc, &Pacer::instant()).expect("re-read");
    let saved = backup::load(report.backup.as_ref().expect("backup")).expect("load backup");
    let restored =
        apply_backup(&mut sim, &desc, &after, &saved, ids(), &paths, &settings()).expect("restore");
    assert!(matches!(restored, ApplyReport { .. }));

    let final_state = read_state_consistent(&mut sim, &desc, &Pacer::instant()).expect("final");
    let baseline_payloads: Vec<(String, Vec<u8>)> = encode_ops(&desc, &baseline)
        .into_iter()
        .map(|op| (op.id, op.payload))
        .collect();
    let final_payloads: Vec<(String, Vec<u8>)> = encode_ops(&desc, &final_state)
        .into_iter()
        .map(|op| (op.id, op.payload))
        .collect();
    assert_eq!(
        baseline_payloads, final_payloads,
        "restore must return the baseline"
    );
}

#[test]
fn shipped_descriptor_round_trips_lighting_and_polling() {
    let desc =
        Descriptor::from_toml(include_str!("../models/holtek-04d9-fc5c.toml")).expect("descriptor");
    walk_round_trip(desc, |state| {
        state.profiles[0].lighting.as_mut().unwrap().brightness = 0;
        state.profiles[0].lighting.as_mut().unwrap().color = [0x10, 0x20, 0x30];
        state.profiles[1].polling_hz = Some(1000);
    });
}

#[test]
fn fixture_descriptor_round_trips_buttons_combos_and_burst() {
    let desc = redragon_gui::models::fixture::descriptor();
    walk_round_trip(desc, |state| {
        state.profiles[0].buttons.as_mut().unwrap()[1].combo = vec!["ctrl".into(), "c".into()];
        state.macros[0].actions = vec![
            redragon_gui::settings::MacroAction {
                kind: redragon_gui::settings::MacroActionKind::KeyDown,
                key: Some("a".into()),
                button: None,
                delay_ms: 0,
            },
            redragon_gui::settings::MacroAction {
                kind: redragon_gui::settings::MacroActionKind::Delay,
                key: None,
                button: None,
                delay_ms: 40,
            },
            redragon_gui::settings::MacroAction {
                kind: redragon_gui::settings::MacroActionKind::KeyUp,
                key: Some("a".into()),
                button: None,
                delay_ms: 0,
            },
        ];
    });
}

#[test]
fn simulated_read_is_stable_and_non_blank() {
    let desc =
        Descriptor::from_toml(include_str!("../models/holtek-04d9-fc5c.toml")).expect("descriptor");
    let mut sim = sim_for(&desc);
    let state = read_state_consistent(&mut sim, &desc, &Pacer::instant()).expect("read");
    assert!(!state.image.is_blank());
    assert_eq!(state.profiles.len(), 5);
}

#[test]
fn fixture_sysfs_tree_resolves_against_the_bundled_registry() {
    use std::os::unix::fs::symlink;

    let registry = Registry::load(&models_dir());
    assert!(registry.errors.is_empty(), "{:?}", registry.errors);

    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    let usb = root.join("5-1");
    let iface = usb.join("5-1:1.2");
    let hid = iface.join("0003:04D9:FC5C.0005");
    std::fs::create_dir_all(&hid).expect("hid dir");
    std::fs::write(
        hid.join("uevent"),
        "HID_ID=0003:000004D9:0000FC5C\nHID_NAME=USB Gaming Mouse\n",
    )
    .expect("uevent");
    std::fs::write(iface.join("bInterfaceNumber"), "02\n").expect("iface");
    std::fs::write(iface.join("bInterfaceClass"), "03\n").expect("class");
    std::fs::write(iface.join("bInterfaceSubClass"), "00\n").expect("subclass");
    std::fs::write(usb.join("bcdDevice"), "0120\n").expect("bcd");
    std::fs::write(usb.join("idVendor"), "04d9\n").expect("vid");
    std::fs::write(usb.join("idProduct"), "fc5c\n").expect("pid");
    std::fs::write(usb.join("product"), "USB Gaming Mouse\n").expect("product");
    let class = root.join("class");
    std::fs::create_dir_all(class.join("hidraw3")).expect("class dir");
    symlink(&hid, class.join("hidraw3/device")).expect("symlink");

    let detected = device::scan_and_identify(&registry, &class).expect("scan");
    assert_eq!(detected.len(), 1);
    assert!(detected[0].is_supported());
    match registry.resolve(
        detected[0].device.vendor_id,
        detected[0].device.product_id,
        detected[0].device.bcd_device,
    ) {
        Resolution::Model {
            descriptor_index, ..
        } => {
            assert_eq!(
                registry.descriptors[descriptor_index].id,
                "holtek-04d9-fc5c"
            );
        }
        other => panic!("unexpected resolution: {other:?}"),
    }
}
