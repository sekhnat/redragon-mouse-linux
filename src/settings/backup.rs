//! Backups: timestamped JSON snapshots of the values an apply is about to
//! overwrite, plus the macro-name sidecar the device itself cannot store.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::models::Descriptor;
use crate::paths::Paths;

use super::state::{DeviceState, WriteOp};

pub const BACKUP_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupFile {
    pub version: u32,
    pub descriptor_id: String,
    pub descriptor_schema: u32,
    pub vendor_id: u16,
    pub product_id: u16,
    pub created_unix: u64,
    pub active_profile: u8,
    pub label: String,
    pub values: Vec<BackupValue>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupValue {
    pub id: String,
    pub label: String,
    pub profile: Option<u8>,
    pub report: u8,
    pub addr: u16,
    /// Register payload as lowercase hex.
    pub payload: String,
}

impl BackupFile {
    pub fn ops(&self) -> Vec<WriteOp> {
        self.values
            .iter()
            .map(|value| WriteOp {
                id: value.id.clone(),
                label: value.label.clone(),
                profile: value.profile,
                report: value.report,
                addr: value.addr,
                payload: decode_hex(&value.payload),
            })
            .collect()
    }

    pub fn created_text(&self) -> String {
        format_timestamp(self.created_unix)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackupInfo {
    pub path: PathBuf,
    pub descriptor_id: String,
    pub label: String,
    pub created_unix: u64,
    pub values: usize,
}

impl BackupInfo {
    pub fn created_text(&self) -> String {
        format_timestamp(self.created_unix)
    }
}

pub fn backup_dir(paths: &Paths) -> PathBuf {
    paths.backups_dir()
}

/// Write the pre-apply values before the first frame is sent.
pub fn write_backup(
    paths: &Paths,
    desc: &Descriptor,
    vendor_id: u16,
    product_id: u16,
    active_profile: u8,
    label: &str,
    baseline_ops: &[WriteOp],
) -> io::Result<PathBuf> {
    let dir = paths.backups_dir();
    fs::create_dir_all(&dir)?;
    let created = now_unix();
    let file = BackupFile {
        version: BACKUP_VERSION,
        descriptor_id: desc.id.clone(),
        descriptor_schema: desc.schema,
        vendor_id,
        product_id,
        created_unix: created,
        active_profile,
        label: label.to_string(),
        values: baseline_ops
            .iter()
            .map(|op| BackupValue {
                id: op.id.clone(),
                label: op.label.clone(),
                profile: op.profile,
                report: op.report,
                addr: op.addr,
                payload: encode_hex(&op.payload),
            })
            .collect(),
    };
    let path = dir.join(format!(
        "{created}_{vendor_id:04x}_{product_id:04x}_p{}.json",
        active_profile + 1
    ));
    let text = serde_json::to_string_pretty(&file).map_err(io::Error::other)?;
    // Write to a temporary file first so a crash cannot leave a half backup.
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, text)?;
    fs::rename(&temp, &path)?;
    tracing::info!(
        path = %path.display(),
        descriptor = %desc.id,
        values = file.values.len(),
        "wrote backup"
    );
    Ok(path)
}

pub fn list(paths: &Paths) -> Vec<BackupInfo> {
    let dir = paths.backups_dir();
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(&dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Ok(file) = load(&path) {
            out.push(BackupInfo {
                path,
                descriptor_id: file.descriptor_id,
                label: file.label,
                created_unix: file.created_unix,
                values: file.values.len(),
            });
        }
    }
    out.sort_by_key(|info| std::cmp::Reverse(info.created_unix));
    out
}

pub fn load(path: &Path) -> Result<BackupFile, BackupError> {
    let text = fs::read_to_string(path)?;
    let file: BackupFile = serde_json::from_str(&text)?;
    if file.version != BACKUP_VERSION {
        return Err(BackupError::Version {
            found: file.version,
            supported: BACKUP_VERSION,
        });
    }
    Ok(file)
}

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("cannot read backup: {0}")]
    Io(#[from] io::Error),
    #[error("backup is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("backup version {found} is not supported (this build reads {supported})")]
    Version { found: u32, supported: u32 },
}

/// Macro names live outside the device: the vendor format stores 16 slots in
/// `Config.ini`, not in the mouse. Keyed by descriptor id.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacroNames {
    #[serde(default)]
    pub by_descriptor: BTreeMap<String, Vec<String>>,
}

impl MacroNames {
    pub fn names_for(&self, descriptor_id: &str) -> Vec<String> {
        self.by_descriptor
            .get(descriptor_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn set(&mut self, descriptor_id: &str, names: Vec<String>) {
        self.by_descriptor.insert(descriptor_id.to_string(), names);
    }
}

pub fn names_path(paths: &Paths) -> PathBuf {
    paths.app_config_dir().join("macro-names.json")
}

pub fn load_names(paths: &Paths) -> MacroNames {
    let path = names_path(paths);
    match fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => MacroNames::default(),
    }
}

pub fn save_names(paths: &Paths, names: &MacroNames) -> io::Result<()> {
    let path = names_path(paths);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(names).map_err(io::Error::other)?;
    fs::write(path, text)
}

/// Copy names from the sidecar into a decoded state.
pub fn apply_names(state: &mut DeviceState, names: &[String]) {
    for macro_state in &mut state.macros {
        if let Some(name) = names.get(macro_state.slot as usize) {
            macro_state.name = name.clone();
        }
    }
}

pub fn capture_names(state: &DeviceState) -> Vec<String> {
    let mut names = vec![String::new(); state.macros.len()];
    for macro_state in &state.macros {
        if let Some(slot) = names.get_mut(macro_state.slot as usize) {
            *slot = macro_state.name.clone();
        }
    }
    names
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn format_timestamp(unix: u64) -> String {
    // Minimal UTC formatter; avoids a chrono dependency for one line of output.
    let days = unix / 86_400;
    let seconds = unix % 86_400;
    let (year, month, day) = civil_from_days(days as i64);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC",
        seconds / 3600,
        (seconds % 3600) / 60,
        seconds % 60
    )
}

/// Howard Hinnant's days-to-civil conversion.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn decode_hex(text: &str) -> Vec<u8> {
    let cleaned: String = text.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    cleaned
        .as_bytes()
        .chunks(2)
        .filter_map(|pair| {
            std::str::from_utf8(pair)
                .ok()
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_round_trips_through_json() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = Paths {
            state_dir: tmp.path().to_path_buf(),
            config_dir: tmp.path().join("config"),
            runtime_dir: tmp.path().join("run"),
        };
        let desc = Descriptor::from_toml(include_str!("../../models/holtek-04d9-fc5c.toml"))
            .expect("descriptor");
        let ops = vec![WriteOp {
            id: "lighting.brightness:0".into(),
            label: "LED brightness".into(),
            profile: Some(0),
            report: 2,
            addr: 0x044F,
            payload: vec![2],
        }];
        let path = write_backup(&paths, &desc, 0x04d9, 0xfc5c, 0, "test", &ops).expect("backup");
        assert!(path.is_file());
        let loaded = load(&path).expect("load");
        assert_eq!(loaded.values.len(), 1);
        assert_eq!(loaded.ops()[0].payload, vec![2]);
        assert_eq!(loaded.descriptor_id, desc.id);
        let listed = list(&paths);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].path, path);
    }

    #[test]
    fn hex_round_trip_preserves_bytes() {
        assert_eq!(
            decode_hex(&encode_hex(&[0x00, 0x0f, 0xff])),
            vec![0, 15, 255]
        );
    }

    #[test]
    fn macro_names_are_stored_per_descriptor() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = Paths {
            state_dir: tmp.path().to_path_buf(),
            config_dir: tmp.path().join("config"),
            runtime_dir: tmp.path().join("run"),
        };
        let mut names = MacroNames::default();
        names.set("holtek-04d9-fc5c", vec!["burst".into()]);
        save_names(&paths, &names).expect("save");
        let loaded = load_names(&paths);
        assert_eq!(
            loaded.names_for("holtek-04d9-fc5c"),
            vec!["burst".to_string()]
        );
    }
}
