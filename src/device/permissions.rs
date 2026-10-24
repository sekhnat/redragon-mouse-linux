//! Access diagnostics and interface validation.
//!
//! When the vendor node cannot be opened the application must tell the user
//! exactly which node, which permission, and which udev rule is needed — and
//! must never try to escalate.

use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use crate::models::Descriptor;

use super::enumerate::HidrawDevice;
use super::report_desc::ReportDescriptor;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeAccess {
    pub node: PathBuf,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub readable: bool,
    pub writable: bool,
}

impl NodeAccess {
    pub fn mode_string(&self) -> String {
        let mut out = String::new();
        for shift in [6, 3, 0] {
            let bits = (self.mode >> shift) & 0o7;
            out.push(if bits & 0o4 != 0 { 'r' } else { '-' });
            out.push(if bits & 0o2 != 0 { 'w' } else { '-' });
            out.push(if bits & 0o1 != 0 { 'x' } else { '-' });
        }
        out
    }
}

/// Inspect a node without changing anything.
pub fn inspect(node: &Path) -> io::Result<NodeAccess> {
    let metadata = fs::metadata(node)?;
    let readable = fs::OpenOptions::new().read(true).open(node).is_ok();
    let writable = fs::OpenOptions::new().write(true).open(node).is_ok();
    Ok(NodeAccess {
        node: node.to_path_buf(),
        mode: metadata.mode() & 0o777,
        uid: metadata.uid(),
        gid: metadata.gid(),
        readable,
        writable,
    })
}

/// The rule that grants the current user access to the vendor interface.
pub fn udev_rule(vendor_id: u16, product_id: u16) -> String {
    format!(
        "KERNEL==\"hidraw*\", ATTRS{{idVendor}}==\"{vendor_id:04x}\", ATTRS{{idProduct}}==\"{product_id:04x}\", MODE=\"0660\", TAG+=\"uaccess\""
    )
}

/// Full guidance block for the diagnostics screen and CLI output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessGuidance {
    pub node: PathBuf,
    pub detail: String,
    pub rule: String,
    pub file: String,
}

pub fn guidance(
    device: &HidrawDevice,
    access: Option<&NodeAccess>,
    reason: &str,
) -> AccessGuidance {
    let rule = udev_rule(device.vendor_id, device.product_id);
    let detail = match access {
        Some(access) => format!(
            "{} (mode {}, owner {}:{}, readable {}, writable {}): {reason}",
            access.node.display(),
            access.mode_string(),
            access.uid,
            access.gid,
            access.readable,
            access.writable
        ),
        None => format!("{}: {reason}", device.node.display()),
    };
    let file = format!(
        "99-redragon-gui-{:04x}-{:04x}.rules",
        device.vendor_id, device.product_id
    );
    AccessGuidance {
        node: device.node.clone(),
        detail,
        rule,
        file,
    }
}

impl AccessGuidance {
    /// Suggested commands for the user to run themselves.
    pub fn instructions(&self) -> String {
        format!(
            r#"Install the rule and reload udev:
  sudo tee /etc/udev/rules.d/{file} >/dev/null <<'EOF'
{rule}
EOF
  sudo udevadm control --reload-rules && sudo udevadm trigger
Then reconnect the mouse.

If access is still denied after the reload (check `getfacl {node}`), add the group
grant instead -- your account must be in that group:
  sudo sed -i 's/TAG+="uaccess"/GROUP="users", TAG+="uaccess"/' /etc/udev/rules.d/{file}
  sudo udevadm control --reload-rules && sudo udevadm trigger
or grant a session-only ACL:
  sudo setfacl -m u:$USER:rw {node}"#,
            file = self.file,
            rule = self.rule,
            node = self.node.display()
        )
    }
}

/// A device whose interface has been verified against its descriptor.
#[derive(Debug, Clone)]
pub struct ValidatedInterface {
    pub report: ReportDescriptor,
}

#[derive(Debug, thiserror::Error)]
pub enum InterfaceError {
    #[error("cannot open {node}: {source}")]
    Open {
        node: String,
        #[source]
        source: io::Error,
    },
    #[error("cannot read the report descriptor from {node}: {source}")]
    DescriptorUnavailable {
        node: String,
        #[source]
        source: io::Error,
    },
    #[error("report descriptor is invalid: {0}")]
    DescriptorInvalid(String),
    #[error("interface does not expose the required collection {usage_page:#06x}/{usage:#04x}")]
    MissingCollection { usage_page: u16, usage: u16 },
    #[error(
        "feature report {report_id} is {actual} bytes on the device but the model declares {expected}"
    )]
    SizeMismatch {
        report_id: u8,
        expected: usize,
        actual: usize,
    },
    #[error("the device does not expose feature report {report_id} (the model declares {expected} bytes)")]
    MissingFeatureReport { report_id: u8, expected: usize },
}

impl InterfaceError {
    /// True when the failure is about reaching the node (permissions, missing
    /// node, bus error), not about the model not matching the hardware.
    pub fn is_access(&self) -> bool {
        matches!(self, InterfaceError::Open { .. })
    }
}

/// Open the node, parse its report descriptor, and check it against the model.
pub fn validate_interface(
    device: &HidrawDevice,
    descriptor: &Descriptor,
) -> Result<ValidatedInterface, InterfaceError> {
    let node_label = device.node.display().to_string();
    let bytes = crate::transport::hidraw::HidrawTransport::report_descriptor(&device.node)
        .map_err(|source| {
            if source.kind() == io::ErrorKind::PermissionDenied {
                InterfaceError::Open {
                    node: node_label.clone(),
                    source,
                }
            } else {
                InterfaceError::DescriptorUnavailable {
                    node: node_label.clone(),
                    source,
                }
            }
        })?;
    let report = ReportDescriptor::parse(&bytes)
        .map_err(|err| InterfaceError::DescriptorInvalid(err.to_string()))?;
    if !report.has_collection(
        descriptor.collections.usage_page,
        descriptor.collections.usage,
    ) {
        return Err(InterfaceError::MissingCollection {
            usage_page: descriptor.collections.usage_page,
            usage: descriptor.collections.usage,
        });
    }
    for (report_id, expected) in descriptor.frame_sizes().iter().map(|(id, len)| (*id, *len)) {
        // The HID report excludes the report-ID byte that hidraw prepends.
        let declared = expected.saturating_sub(1);
        match report.feature_size_bytes(report_id) {
            Some(actual) if actual == declared => {}
            Some(actual) => {
                return Err(InterfaceError::SizeMismatch {
                    report_id,
                    expected,
                    actual: actual + 1,
                });
            }
            None => {
                return Err(InterfaceError::MissingFeatureReport {
                    report_id,
                    expected,
                });
            }
        }
    }
    Ok(ValidatedInterface { report })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device() -> HidrawDevice {
        HidrawDevice {
            node: PathBuf::from("/dev/hidraw3"),
            sysfs: PathBuf::from("/sys/devices/5-1/5-1:1.2/0003:04D9:FC5C.0005"),
            vendor_id: 0x04d9,
            product_id: 0xfc5c,
            bcd_device: 0x0120,
            interface_number: 2,
            interface_class: 3,
            interface_subclass: 0,
            interface_protocol: 0,
            name: "USB Gaming Mouse".into(),
            bus: 3,
        }
    }

    #[test]
    fn rule_targets_the_observed_ids() {
        let rule = udev_rule(0x04d9, 0xfc5c);
        assert!(rule.contains("idVendor}==\"04d9\""), "{rule}");
        assert!(rule.contains("idProduct}==\"fc5c\""), "{rule}");
        assert!(rule.contains("TAG+=\"uaccess\""), "{rule}");
    }

    #[test]
    fn guidance_reports_mode_owner_and_rule_for_a_root_only_node() {
        let access = NodeAccess {
            node: PathBuf::from("/dev/hidraw3"),
            mode: 0o600,
            uid: 0,
            gid: 0,
            readable: false,
            writable: false,
        };
        let guidance = guidance(&device(), Some(&access), "permission denied");
        assert!(guidance.detail.contains("/dev/hidraw3"), "{guidance:?}");
        assert!(guidance.detail.contains("rw-------"), "{guidance:?}");
        assert!(guidance.detail.contains("owner 0:0"), "{guidance:?}");
        assert!(guidance.rule.contains("04d9"), "{guidance:?}");
        assert!(
            guidance.instructions().contains("/etc/udev/rules.d/"),
            "{guidance:?}"
        );
        assert!(
            guidance
                .instructions()
                .contains("udevadm control --reload-rules"),
            "{guidance:?}"
        );
    }

    #[test]
    fn instructions_include_a_group_and_acl_fallback() {
        let guidance = guidance(&device(), None, "permission denied");
        let text = guidance.instructions();
        assert!(text.contains("GROUP=\"users\""), "{text}");
        assert!(text.contains("setfacl"), "{text}");
        assert!(text.contains("getfacl"), "{text}");
    }

    #[test]
    fn only_open_failures_count_as_access_problems() {
        let open = InterfaceError::Open {
            node: "/dev/hidraw3".into(),
            source: io::Error::from(io::ErrorKind::PermissionDenied),
        };
        assert!(open.is_access());
        let mismatch = InterfaceError::MissingCollection {
            usage_page: 0xffa0,
            usage: 1,
        };
        assert!(!mismatch.is_access());
        let size = InterfaceError::SizeMismatch {
            report_id: 4,
            expected: 256,
            actual: 1024,
        };
        assert!(!size.is_access());
    }

    #[test]
    fn mode_string_matches_posix_notation() {
        let access = NodeAccess {
            node: PathBuf::from("/dev/null"),
            mode: 0o660,
            uid: 0,
            gid: 0,
            readable: true,
            writable: true,
        };
        assert_eq!(access.mode_string(), "rw-rw----");
    }
}
