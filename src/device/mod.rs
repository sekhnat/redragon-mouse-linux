//! Device discovery: sysfs enumeration, model resolution, access diagnostics.

pub mod enumerate;
pub mod permissions;
pub mod report_desc;

use std::io;
use std::path::Path;

pub use enumerate::{scan, HidrawDevice};
pub use permissions::{
    guidance, inspect, udev_rule, validate_interface, AccessGuidance, InterfaceError, NodeAccess,
    ValidatedInterface,
};
pub use report_desc::ReportDescriptor;

use crate::models::{Registry, Resolution};

/// Result of matching a scanned node against the model registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Identification {
    Supported {
        descriptor_index: usize,
        entry_index: usize,
    },
    Unsupported,
    Conflict(Vec<String>),
}

#[derive(Debug, Clone)]
pub struct DetectedDevice {
    pub device: HidrawDevice,
    pub identification: Identification,
}

impl DetectedDevice {
    pub fn is_supported(&self) -> bool {
        matches!(self.identification, Identification::Supported { .. })
    }
}

pub fn identify(registry: &Registry, device: &HidrawDevice) -> Identification {
    match registry.resolve(device.vendor_id, device.product_id, device.bcd_device) {
        Resolution::Model {
            descriptor_index,
            entry_index,
        } => Identification::Supported {
            descriptor_index,
            entry_index,
        },
        Resolution::Unsupported => Identification::Unsupported,
        Resolution::Conflict(ids) => Identification::Conflict(ids),
    }
}

pub fn scan_and_identify(registry: &Registry, base: &Path) -> io::Result<Vec<DetectedDevice>> {
    let devices = enumerate::scan(base)?;
    Ok(devices
        .into_iter()
        .filter(|device| device.is_vendor_interface())
        .map(|device| {
            let identification = identify(registry, &device);
            DetectedDevice {
                device,
                identification,
            }
        })
        .collect())
}

/// Arrivals and removals seen by a hotplug poll.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HotplugEvent {
    pub arrived: Vec<HidrawDevice>,
    pub removed: Vec<HidrawDevice>,
}

impl HotplugEvent {
    pub fn is_empty(&self) -> bool {
        self.arrived.is_empty() && self.removed.is_empty()
    }
}

/// Polls sysfs for hidraw arrivals and removals without reopening anything.
pub struct HotplugMonitor {
    base: std::path::PathBuf,
    known: Vec<HidrawDevice>,
}

impl HotplugMonitor {
    pub fn new(base: std::path::PathBuf) -> io::Result<Self> {
        let known = enumerate::scan(&base)?;
        Ok(HotplugMonitor { base, known })
    }

    pub fn devices(&self) -> &[HidrawDevice] {
        &self.known
    }

    /// Compare a fresh scan with the previous one.
    pub fn poll(&mut self) -> io::Result<HotplugEvent> {
        let current = enumerate::scan(&self.base)?;
        let arrived = current
            .iter()
            .filter(|device| !self.known.iter().any(|known| known.node == device.node))
            .cloned()
            .collect();
        let removed = self
            .known
            .iter()
            .filter(|device| !current.iter().any(|now| now.node == device.node))
            .cloned()
            .collect();
        self.known = current;
        Ok(HotplugEvent { arrived, removed })
    }
}

/// Human-readable reason why a supported device is not usable.
pub fn unsupported_reason(identification: &Identification) -> String {
    match identification {
        Identification::Supported { .. } => "supported".into(),
        Identification::Unsupported => "this vendor/product ID has no model descriptor".into(),
        Identification::Conflict(ids) => format!(
            "several descriptors claim this device release: {}",
            ids.join(", ")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Registry;

    fn registry() -> Registry {
        Registry::load(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("models")
                .as_path(),
        )
    }

    #[test]
    fn hotplug_monitor_reports_arrival_and_removal() {
        use std::os::unix::fs::symlink;
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let class = root.join("class");
        std::fs::create_dir_all(&class).expect("class dir");

        let add = |index: u8| {
            let usb = root.join("5-1");
            let iface = usb.join(format!("5-1:1.{index}"));
            let hid = iface.join(format!("0003:04D9:FC5C.{index:04}"));
            std::fs::create_dir_all(&hid).expect("hid dir");
            std::fs::write(
                hid.join("uevent"),
                "HID_ID=0003:000004D9:0000FC5C\nHID_NAME=Mouse\n",
            )
            .expect("uevent");
            std::fs::write(iface.join("bInterfaceNumber"), "02\n").expect("iface");
            std::fs::write(usb.join("bcdDevice"), "0120\n").expect("bcd");
            let entry = class.join(format!("hidraw{index}"));
            std::fs::create_dir_all(&entry).expect("class entry");
            symlink(&hid, entry.join("device")).expect("symlink");
        };

        add(0);
        let mut monitor = HotplugMonitor::new(class.clone()).expect("monitor");
        assert_eq!(monitor.devices().len(), 1);
        assert!(monitor.poll().expect("poll").is_empty());

        add(1);
        let event = monitor.poll().expect("poll");
        assert_eq!(event.arrived.len(), 1);
        assert_eq!(
            event.arrived[0].node,
            std::path::PathBuf::from("/dev/hidraw1")
        );
        assert!(event.removed.is_empty());

        std::fs::remove_dir_all(class.join("hidraw0")).expect("remove node");
        let event = monitor.poll().expect("poll");
        assert!(event.arrived.is_empty());
        assert_eq!(event.removed.len(), 1);
        assert_eq!(
            event.removed[0].node,
            std::path::PathBuf::from("/dev/hidraw0")
        );
    }

    #[test]
    fn bundled_registry_resolves_the_attached_mouse() {
        let registry = registry();
        assert!(registry.errors.is_empty(), "{:?}", registry.errors);
        match registry.resolve(0x04d9, 0xfc5c, 0x0120) {
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
}
