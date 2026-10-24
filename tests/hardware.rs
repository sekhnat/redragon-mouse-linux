//! Physical-hardware verification (ignored by default).
//!
//! Run explicitly, with the mouse connected and access granted:
//!
//! ```sh
//! REDRAGON_GUI_ALLOW_WRITE=1 cargo test --test hardware -- --ignored --nocapture
//! ```
//!
//! The test performs one benign change (LED brightness on a non-active profile),
//! verifies it by read-back, then restores the pre-change values from the
//! backup the apply engine wrote. It refuses to run without the environment
//! variable so a plain `cargo test` never touches the hardware.

use std::path::Path;

use redragon_gui::device;
use redragon_gui::models::Registry;
use redragon_gui::paths::Paths;
use redragon_gui::settings::{
    apply, apply_backup, backup, read_state_consistent, ApplySettings, DeviceIds,
};
use redragon_gui::transport::hidraw::HidrawTransport;
use redragon_gui::transport::Pacer;

#[test]
#[ignore = "writes to the physical mouse; run explicitly with REDRAGON_GUI_ALLOW_WRITE=1"]
fn led_brightness_round_trip_on_hardware() {
    if std::env::var("REDRAGON_GUI_ALLOW_WRITE").as_deref() != Ok("1") {
        panic!("refusing to write: set REDRAGON_GUI_ALLOW_WRITE=1 to run this hardware test");
    }

    let models = Path::new(env!("CARGO_MANIFEST_DIR")).join("models");
    let registry = Registry::load(&models);
    let base = device::enumerate::default_base();
    let detected = device::scan_and_identify(&registry, &base).expect("scan");
    let det = detected
        .iter()
        .find(|det| det.is_supported())
        .expect("a supported device must be connected");
    let descriptor_index = match det.identification {
        device::Identification::Supported {
            descriptor_index, ..
        } => descriptor_index,
        _ => unreachable!(),
    };
    let desc = registry.descriptors[descriptor_index].clone();
    device::validate_interface(&det.device, &desc).expect("interface matches the descriptor");

    let paths = Paths::from_env();
    let ids = DeviceIds {
        vendor_id: det.device.vendor_id,
        product_id: det.device.product_id,
    };
    let settings = ApplySettings {
        pacer: Pacer::production(),
        force_stale: false,
        active_profile: 0,
        label: "hardware LED brightness verification".into(),
    };

    let mut transport = HidrawTransport::open(&det.device.node).expect("open vendor node");
    let baseline = read_state_consistent(&mut transport, &desc, &Pacer::production())
        .expect("consistent baseline read");

    let slot = (0..desc.profile_count())
        .find(|profile| *profile != baseline.active_profile)
        .expect("a non-active profile");
    let original = baseline.profiles[slot as usize]
        .lighting
        .as_ref()
        .expect("lighting was read")
        .brightness;
    let levels = desc
        .lighting
        .as_ref()
        .map(|lighting| lighting.brightness_levels)
        .expect("lighting declared");
    let target = if original == 0 { 1 } else { original - 1 };
    assert!(target < levels, "target brightness is in range");

    let mut draft = baseline.clone();
    draft.profiles[slot as usize]
        .lighting
        .as_mut()
        .expect("lighting")
        .brightness = target;
    let mut apply_settings = settings.clone();
    apply_settings.active_profile = baseline.active_profile;

    let report = apply(
        &mut transport,
        &desc,
        &baseline,
        &draft,
        ids,
        &paths,
        &apply_settings,
    )
    .expect("apply one brightness change");
    assert!(report.fully_confirmed(), "{report:?}");
    let backup_path = report.backup.clone().expect("backup path");
    assert!(backup_path.is_file(), "backup must exist before writes");

    let after = read_state_consistent(&mut transport, &desc, &Pacer::production())
        .expect("read after apply");
    let applied = after.profiles[slot as usize]
        .lighting
        .as_ref()
        .expect("lighting")
        .brightness;
    assert_eq!(applied, target, "read-back must show the new brightness");
    println!(
        "profile {} LED brightness {} -> {} confirmed; backup {}",
        slot + 1,
        original,
        target,
        backup_path.display()
    );

    let saved = backup::load(&backup_path).expect("load backup");
    let restored = apply_backup(
        &mut transport,
        &desc,
        &after,
        &saved,
        ids,
        &paths,
        &apply_settings,
    )
    .expect("restore backup");
    assert!(restored.fully_confirmed(), "{restored:?}");

    let final_state = read_state_consistent(&mut transport, &desc, &Pacer::production())
        .expect("read after restore");
    assert_eq!(
        final_state.profiles[slot as usize]
            .lighting
            .as_ref()
            .expect("lighting")
            .brightness,
        original,
        "restore must return the original brightness"
    );
    println!("profile {} LED brightness restored to {original}", slot + 1);
}
