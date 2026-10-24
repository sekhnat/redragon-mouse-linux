//! Model registry: maps observed device IDs to descriptors, and loads them
//! resiliently so one broken file never takes the others down.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::descriptor::{Descriptor, DescriptorError};

pub const INDEX_FILE: &str = "00-registry.toml";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryIndex {
    pub schema: u32,
    pub devices: Vec<RegistryDevice>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryDevice {
    /// Descriptor file name, relative to the registry directory.
    pub descriptor: String,
    pub vendor_id: u16,
    pub product_id: u16,
    /// Inclusive `bcdDevice` bounds. Both zero means "any version".
    #[serde(default)]
    pub bcd_min: u16,
    #[serde(default)]
    pub bcd_max: u16,
}

#[derive(Debug, Clone)]
pub struct RegistryEntry {
    pub descriptor_index: Option<usize>,
    pub descriptor_file: PathBuf,
    pub vendor_id: u16,
    pub product_id: u16,
    pub bcd_min: u16,
    pub bcd_max: u16,
}

impl RegistryEntry {
    pub fn matches(&self, vendor_id: u16, product_id: u16, bcd: u16) -> bool {
        if self.vendor_id != vendor_id || self.product_id != product_id {
            return false;
        }
        if self.bcd_max == 0 {
            return true;
        }
        bcd >= self.bcd_min && bcd <= self.bcd_max
    }

    /// Narrower ranges win; an unrestricted entry scores worst.
    fn specificity(&self) -> u32 {
        if self.bcd_max == 0 {
            u32::MAX
        } else {
            self.bcd_max as u32 - self.bcd_min as u32
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    Model {
        descriptor_index: usize,
        entry_index: usize,
    },
    Unsupported,
    Conflict(Vec<String>),
}

#[derive(Debug, Default)]
pub struct Registry {
    pub root: PathBuf,
    pub descriptors: Vec<Descriptor>,
    pub entries: Vec<RegistryEntry>,
    /// Files that failed to load, with the reason. Surfaced in diagnostics.
    pub errors: Vec<LoadError>,
}

#[derive(Debug, Clone)]
pub struct LoadError {
    pub file: PathBuf,
    pub message: String,
}

impl Registry {
    /// Load every descriptor named by the index, plus any `*.toml` descriptor
    /// not listed there (an index omission should not hide a file).
    pub fn load(dir: &Path) -> Self {
        let mut registry = Registry {
            root: dir.to_path_buf(),
            ..Default::default()
        };
        let index_path = dir.join(INDEX_FILE);
        let index_text = match std::fs::read_to_string(&index_path) {
            Ok(text) => text,
            Err(err) => {
                registry.errors.push(LoadError {
                    file: index_path,
                    message: format!("cannot read registry index: {err}"),
                });
                return registry;
            }
        };
        let index: RegistryIndex = match toml::from_str(&index_text) {
            Ok(index) => index,
            Err(err) => {
                registry.errors.push(LoadError {
                    file: index_path,
                    message: format!("cannot parse registry index: {err}"),
                });
                return registry;
            }
        };

        let mut by_file: BTreeMap<String, usize> = BTreeMap::new();
        for device in index.devices {
            let file = dir.join(&device.descriptor);
            let descriptor_index =
                *by_file
                    .entry(device.descriptor.clone())
                    .or_insert_with(|| match load_descriptor(&file, &mut registry.errors) {
                        Some(desc) => {
                            registry.descriptors.push(desc);
                            registry.descriptors.len() - 1
                        }
                        None => usize::MAX,
                    });
            registry.entries.push(RegistryEntry {
                descriptor_index: (descriptor_index != usize::MAX).then_some(descriptor_index),
                descriptor_file: file,
                vendor_id: device.vendor_id,
                product_id: device.product_id,
                bcd_min: device.bcd_min,
                bcd_max: device.bcd_max,
            });
        }

        // Pick up unlisted descriptor files so a stray file is reported rather
        // than silently ignored.
        if let Ok(read_dir) = std::fs::read_dir(dir) {
            let mut paths: Vec<PathBuf> = read_dir
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    path.extension().map(|e| e == "toml").unwrap_or(false)
                        && path.file_name().map(|n| n != INDEX_FILE).unwrap_or(false)
                })
                .collect();
            paths.sort();
            for path in paths {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                if by_file.contains_key(&name) {
                    continue;
                }
                if load_descriptor(&path, &mut registry.errors).is_some() {
                    registry.errors.push(LoadError {
                        file: path,
                        message: "loaded but not listed in the registry index".into(),
                    });
                }
            }
        }
        registry
    }

    pub fn descriptor(&self, index: usize) -> Option<&Descriptor> {
        self.descriptors.get(index)
    }

    pub fn by_id(&self, id: &str) -> Option<&Descriptor> {
        self.descriptors.iter().find(|d| d.id == id)
    }

    pub fn resolve(&self, vendor_id: u16, product_id: u16, bcd: u16) -> Resolution {
        let mut matches: Vec<(usize, &RegistryEntry)> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.descriptor_index.is_some() && e.matches(vendor_id, product_id, bcd))
            .collect();
        if matches.is_empty() {
            return Resolution::Unsupported;
        }
        matches.sort_by_key(|(_, e)| e.specificity());
        let best = matches[0].1.specificity();
        let tied: Vec<&(usize, &RegistryEntry)> = matches
            .iter()
            .filter(|(_, e)| e.specificity() == best)
            .collect();
        if tied.len() > 1 {
            return Resolution::Conflict(
                tied.iter()
                    .filter_map(|(_, e)| {
                        e.descriptor_index
                            .and_then(|i| self.descriptors.get(i))
                            .map(|d| d.id.clone())
                            .or_else(|| Some(e.descriptor_file.display().to_string()))
                    })
                    .collect(),
            );
        }
        let (entry_index, entry) = matches[0];
        Resolution::Model {
            descriptor_index: entry.descriptor_index.expect("filtered above"),
            entry_index,
        }
    }

    pub fn entry(&self, index: usize) -> Option<&RegistryEntry> {
        self.entries.get(index)
    }
}

fn load_descriptor(file: &Path, errors: &mut Vec<LoadError>) -> Option<Descriptor> {
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(err) => {
            errors.push(LoadError {
                file: file.to_path_buf(),
                message: format!("cannot read descriptor: {err}"),
            });
            return None;
        }
    };
    match Descriptor::from_toml(&text) {
        Ok(desc) => Some(desc),
        Err(err) => {
            errors.push(LoadError {
                file: file.to_path_buf(),
                message: describe(err),
            });
            None
        }
    }
}

fn describe(err: DescriptorError) -> String {
    err.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(
        descriptor_index: usize,
        descriptor: &str,
        bcd_min: u16,
        bcd_max: u16,
    ) -> RegistryEntry {
        RegistryEntry {
            descriptor_index: Some(descriptor_index),
            descriptor_file: PathBuf::from(descriptor),
            vendor_id: 0x04d9,
            product_id: 0xfc5c,
            bcd_min,
            bcd_max,
        }
    }

    #[test]
    fn narrowest_version_range_wins() {
        let registry = Registry {
            entries: vec![
                entry(0, "unrestricted.toml", 0, 0),
                entry(1, "versioned.toml", 0x0100, 0x01ff),
            ],
            descriptors: Vec::new(),
            ..Default::default()
        };
        match registry.resolve(0x04d9, 0xfc5c, 0x0120) {
            Resolution::Model {
                descriptor_index, ..
            } => assert_eq!(descriptor_index, 1),
            other => panic!("unexpected resolution: {other:?}"),
        }
        match registry.resolve(0x04d9, 0xfc5c, 0x0200) {
            Resolution::Model {
                descriptor_index, ..
            } => assert_eq!(descriptor_index, 0),
            other => panic!("unexpected resolution: {other:?}"),
        }
    }

    #[test]
    fn unknown_product_is_unsupported() {
        let registry = Registry {
            entries: vec![entry(0, "known.toml", 0, 0)],
            ..Default::default()
        };
        assert_eq!(
            registry.resolve(0x04d9, 0x1234, 0x0100),
            Resolution::Unsupported
        );
    }

    #[test]
    fn equal_ranges_are_a_conflict() {
        let registry = Registry {
            entries: vec![
                entry(0, "a.toml", 0x0100, 0x01ff),
                entry(1, "b.toml", 0x0100, 0x01ff),
            ],
            descriptors: Vec::new(),
            ..Default::default()
        };
        match registry.resolve(0x04d9, 0xfc5c, 0x0120) {
            Resolution::Conflict(ids) => {
                assert_eq!(ids.len(), 2);
                assert!(ids[0].ends_with(".toml"));
            }
            other => panic!("unexpected resolution: {other:?}"),
        }
    }

    #[test]
    fn malformed_descriptor_does_not_block_valid_ones() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join(INDEX_FILE),
            r#"
schema = 1
[[devices]]
descriptor = "broken.toml"
vendor_id = 0x04d9
product_id = 0x0001
[[devices]]
descriptor = "valid.toml"
vendor_id = 0x04d9
product_id = 0xfc5c
"#,
        )
        .expect("write index");
        std::fs::write(dir.path().join("broken.toml"), "schema = 1\nid = 42\n")
            .expect("write broken");
        std::fs::write(
            dir.path().join("valid.toml"),
            include_str!("../../models/holtek-04d9-fc5c.toml"),
        )
        .expect("write valid");

        let registry = Registry::load(dir.path());
        assert_eq!(registry.descriptors.len(), 1);
        assert!(registry
            .errors
            .iter()
            .any(|e| e.file.ends_with("broken.toml")));
        match registry.resolve(0x04d9, 0xfc5c, 0x0120) {
            Resolution::Model { .. } => {}
            other => panic!("valid descriptor not resolved: {other:?}"),
        }
    }
}
