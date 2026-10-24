//! Sysfs enumeration of hidraw nodes.
//!
//! Everything here works without opening the node, so an unsupported or
//! unreachable device can still be reported with its IDs, interface and path.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// One hidraw node as described by sysfs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HidrawDevice {
    pub node: PathBuf,
    /// Canonical sysfs directory of the HID device (`.../0003:04D9:FC5C.0005`).
    pub sysfs: PathBuf,
    pub vendor_id: u16,
    pub product_id: u16,
    pub bcd_device: u16,
    pub interface_number: u8,
    pub interface_class: u8,
    pub interface_subclass: u8,
    pub interface_protocol: u8,
    pub name: String,
    pub bus: u16,
}

impl HidrawDevice {
    /// HID class 3 with no subclass is the vendor-defined configuration
    /// interface; boot mouse/keyboard interfaces have a subclass.
    pub fn is_vendor_interface(&self) -> bool {
        self.interface_class == 3 && self.interface_subclass == 0
    }

    pub fn id_string(&self) -> String {
        format!("{:04x}:{:04x}", self.vendor_id, self.product_id)
    }
}

/// Default sysfs class directory.
pub fn default_base() -> PathBuf {
    PathBuf::from("/sys/class/hidraw")
}

/// Scan a hidraw class directory. Unreadable nodes are skipped, not fatal.
pub fn scan(base: &Path) -> io::Result<Vec<HidrawDevice>> {
    let mut devices = Vec::new();
    for entry in fs::read_dir(base)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("hidraw") {
            continue;
        }
        let node = PathBuf::from("/dev").join(&name);
        match read_device(&entry.path(), node) {
            Ok(device) => devices.push(device),
            Err(err) => {
                tracing::debug!(node = %name, error = %err, "skipping unreadable hidraw entry");
            }
        }
    }
    devices.sort_by(|a, b| natural_cmp(&a.node, &b.node));
    Ok(devices)
}

fn natural_cmp(a: &Path, b: &Path) -> std::cmp::Ordering {
    let num = |p: &Path| {
        p.file_name()
            .and_then(|n| {
                n.to_string_lossy()
                    .trim_start_matches("hidraw")
                    .parse::<u32>()
                    .ok()
            })
            .unwrap_or(0)
    };
    num(a).cmp(&num(b))
}

fn read_device(class_entry: &Path, node: PathBuf) -> io::Result<HidrawDevice> {
    let sysfs = fs::canonicalize(class_entry.join("device"))?;
    let uevent = read_key_values(&sysfs.join("uevent"));

    let (mut vendor_id, mut product_id, mut bus) = (0u16, 0u16, 0u16);
    if let Some(id) = uevent.get("HID_ID") {
        let parts: Vec<&str> = id.split(':').collect();
        if parts.len() == 3 {
            bus = u16::from_str_radix(parts[0], 16).unwrap_or(0);
            vendor_id = truncate_id(parts[1]);
            product_id = truncate_id(parts[2]);
        }
    }

    let interface_dir = sysfs.parent().unwrap_or(&sysfs);
    let usb_dir = interface_dir.parent().unwrap_or(interface_dir);

    if vendor_id == 0 {
        vendor_id = read_hex_u16(&usb_dir.join("idVendor")).unwrap_or(0);
    }
    if product_id == 0 {
        product_id = read_hex_u16(&usb_dir.join("idProduct")).unwrap_or(0);
    }
    let bcd_device = read_hex_u16(&usb_dir.join("bcdDevice")).unwrap_or(0);
    let interface_number = read_hex_u16(&interface_dir.join("bInterfaceNumber")).unwrap_or(0) as u8;
    let interface_class = read_hex_u16(&interface_dir.join("bInterfaceClass"))
        .map(|value| value as u8)
        .unwrap_or(3);
    let interface_subclass = read_hex_u16(&interface_dir.join("bInterfaceSubClass"))
        .map(|value| value as u8)
        .unwrap_or(0);
    let interface_protocol = read_hex_u16(&interface_dir.join("bInterfaceProtocol"))
        .map(|value| value as u8)
        .unwrap_or(0);
    let name = uevent
        .get("HID_NAME")
        .cloned()
        .or_else(|| fs::read_to_string(usb_dir.join("product")).ok())
        .unwrap_or_default()
        .trim()
        .to_string();

    Ok(HidrawDevice {
        node,
        sysfs,
        vendor_id,
        product_id,
        bcd_device,
        interface_number,
        interface_class,
        interface_subclass,
        interface_protocol,
        name,
        bus,
    })
}

fn truncate_id(text: &str) -> u16 {
    u32::from_str_radix(text, 16)
        .map(|value| value as u16)
        .unwrap_or(0)
}

fn read_key_values(path: &Path) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    if let Ok(text) = fs::read_to_string(path) {
        for line in text.lines() {
            if let Some((key, value)) = line.split_once('=') {
                map.insert(key.trim().to_string(), value.trim().to_string());
            }
        }
    }
    map
}

fn read_hex_u16(path: &Path) -> Option<u16> {
    let text = fs::read_to_string(path).ok()?;
    read_hex_u16_str(text.trim())
}

fn read_hex_u16_str(text: &str) -> Option<u16> {
    // USB sysfs idVendor/bcdDevice are hex without a 0x prefix.
    u32::from_str_radix(text.trim_start_matches("0x"), 16)
        .ok()
        .map(|value| value as u16)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn fixture_device(root: &Path, bus_dir: &str, iface: &str, uevent: &str, hid_device: &str) {
        let usb = root.join(bus_dir);
        let interface = usb.join(iface);
        let hid = interface.join(hid_device);
        fs::create_dir_all(&hid).expect("create hid dir");
        fs::write(hid.join("uevent"), uevent).expect("write uevent");
        fs::write(interface.join("bInterfaceNumber"), "02\n").expect("iface");
        fs::write(usb.join("bcdDevice"), "0120\n").expect("bcd");
        fs::write(usb.join("idVendor"), "04d9\n").expect("vid");
        fs::write(usb.join("idProduct"), "fc5c\n").expect("pid");
        fs::write(usb.join("product"), "USB Gaming Mouse\n").expect("product");
    }

    #[test]
    fn reads_ids_interface_and_version_from_sysfs() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        fixture_device(
            root,
            "5-1",
            "5-1:1.2",
            "HID_ID=0003:000004D9:0000FC5C\nHID_NAME=USB Gaming Mouse\n",
            "0003:04D9:FC5C.0005",
        );
        // Class directory with a symlink like /sys/class/hidraw/hidraw3/device.
        let class = root.join("class");
        fs::create_dir_all(class.join("hidraw3")).expect("class dir");
        symlink(
            root.join("5-1/5-1:1.2/0003:04D9:FC5C.0005"),
            class.join("hidraw3/device"),
        )
        .expect("symlink");

        let devices = scan(&class).expect("scan");
        assert_eq!(devices.len(), 1);
        let device = &devices[0];
        assert_eq!(device.vendor_id, 0x04d9);
        assert_eq!(device.product_id, 0xfc5c);
        assert_eq!(device.bcd_device, 0x0120);
        assert_eq!(device.interface_number, 2);
        assert_eq!(device.name, "USB Gaming Mouse");
        assert_eq!(device.node, PathBuf::from("/dev/hidraw3"));
    }

    #[test]
    fn multiple_nodes_are_ordered_numerically() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let class = root.join("class");
        for index in [10u8, 2u8] {
            fixture_device(
                root,
                "5-1",
                &format!("5-1:1.{index}"),
                &format!("HID_ID=0003:000004D9:0000FC5C\nHID_NAME=Mouse {index}\n"),
                &format!("0003:04D9:FC5C.{index:04}"),
            );
            let entry = class.join(format!("hidraw{index}"));
            fs::create_dir_all(&entry).expect("class dir");
            symlink(
                root.join(format!("5-1/5-1:1.{index}/0003:04D9:FC5C.{index:04}")),
                entry.join("device"),
            )
            .expect("symlink");
        }
        let devices = scan(&class).expect("scan");
        let names: Vec<String> = devices
            .iter()
            .map(|d| d.node.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["hidraw2", "hidraw10"]);
    }
}
