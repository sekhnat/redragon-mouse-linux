//! Typed settings state over the raw register image.
//!
//! The image is the source of truth (it is what backup, diff and verification
//! work on); the typed state below is a decoded view plus the user's edits.

use std::collections::BTreeMap;

use crate::models::descriptor::{ActionKind, Descriptor};
use crate::transport::frames::REPORT_BLOCK;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReadImage {
    windows: BTreeMap<(String, u8), WindowData>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowData {
    pub report: u8,
    pub addr: u16,
    pub bytes: Vec<u8>,
}

impl ReadImage {
    pub fn insert(&mut self, id: &str, profile: u8, data: WindowData) {
        self.windows.insert((id.to_string(), profile), data);
    }

    pub fn get(&self, id: &str, profile: u8) -> Option<&[u8]> {
        self.windows
            .get(&(id.to_string(), profile))
            .map(|w| w.bytes.as_slice())
    }

    pub fn window(&self, id: &str, profile: u8) -> Option<&WindowData> {
        self.windows.get(&(id.to_string(), profile))
    }

    pub fn iter(&self) -> impl Iterator<Item = (&(String, u8), &WindowData)> {
        self.windows.iter()
    }

    pub fn len(&self) -> usize {
        self.windows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    /// True when every window is either all zero or all `0xFF`, which is the
    /// signature of a device that answers reads with filler.
    pub fn is_blank(&self) -> bool {
        !self.windows.is_empty()
            && self.windows.values().all(|window| {
                window.bytes.iter().all(|b| *b == 0) || window.bytes.iter().all(|b| *b == 0xFF)
            })
    }

    pub fn same_content(&self, other: &ReadImage) -> bool {
        if self.windows.len() != other.windows.len() {
            return false;
        }
        self.windows.iter().all(|(key, value)| {
            other
                .windows
                .get(key)
                .map(|other| other.bytes == value.bytes)
                .unwrap_or(false)
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DpiStage {
    pub enabled: bool,
    /// DPI value shown to the user; 0 when the register code has no table entry.
    pub x: u16,
    pub y: u16,
    /// Register code as read from the device, preserved for unchanged writes.
    pub raw: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileDpi {
    pub stages: Vec<DpiStage>,
    pub linked: Vec<bool>,
    pub active: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileLighting {
    pub mode: String,
    /// Mode code as read (`(mode << 8) | mode2` on models with two mode bytes),
    /// used when `mode` is empty (unrecognised code).
    pub mode_code: u16,
    pub color: [u8; 3],
    pub speed: u8,
    pub brightness: u8,
    /// Per-zone colours in descriptor order; empty when the model has no zones.
    pub zone_colors: Vec<[u8; 3]>,
}

impl ProfileLighting {
    /// Select an effect by id, keeping `speed` consistent with what the effect
    /// can use: effects without a speed store 0 (as the vendor does) and an
    /// effect that gains one starts from the middle of the range.
    pub fn set_mode(&mut self, spec: &crate::models::descriptor::LightingSpec, id: &str) -> bool {
        let Some(mode) = spec.mode(id) else {
            return false;
        };
        self.mode = mode.id.clone();
        self.mode_code = mode.code;
        if !mode.uses_speed {
            self.speed = 0;
        } else if self.speed == 0 {
            self.speed = spec.speed_levels / 2;
        }
        true
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ButtonBinding {
    pub action: String,
    /// Key id from the descriptor table, when the action is a keyboard key.
    pub key: Option<String>,
    /// Multi-key combo slots, in order. Empty when the model has no combos.
    pub combo: Vec<String>,
    pub macro_slot: Option<u8>,
    pub profile_target: Option<u8>,
    /// Raw 4-byte entry as read, preserving unknown bytes and codes.
    pub raw: [u8; 4],
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProfileState {
    pub dpi: Option<ProfileDpi>,
    pub polling_hz: Option<u16>,
    pub lighting: Option<ProfileLighting>,
    pub buttons: Option<Vec<ButtonBinding>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceSettings {
    pub angle_snap: Option<bool>,
    pub lift_off: Option<String>,
    pub scroll: Option<u16>,
    pub scroll_page: Option<bool>,
    pub accel: Option<bool>,
    pub double_click: Option<u16>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MacroActionKind {
    KeyDown,
    KeyUp,
    MouseDown,
    MouseUp,
    Delay,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MacroAction {
    pub kind: MacroActionKind,
    pub key: Option<String>,
    pub button: Option<u8>,
    pub delay_ms: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Burst {
    pub bullets: u8,
    pub delay_ms: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Macro {
    pub slot: u8,
    pub name: String,
    pub actions: Vec<MacroAction>,
    pub burst: Option<Burst>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceState {
    pub active_profile: u8,
    pub profiles: Vec<ProfileState>,
    pub device: DeviceSettings,
    pub macros: Vec<Macro>,
    pub image: ReadImage,
}

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("the model descriptor has no `{0}` section")]
    MissingSection(&'static str),
    #[error("window `{window}` for profile {profile} was not read")]
    MissingWindow { window: String, profile: u8 },
    #[error("window `{window}` is too short: needs {needed} bytes, has {actual}")]
    ShortWindow {
        window: String,
        needed: usize,
        actual: usize,
    },
    #[error("profile index {0} is outside the model's profile count")]
    ProfileOutOfRange(u8),
}

impl DeviceState {
    pub fn profile(&self, index: u8) -> Option<&ProfileState> {
        self.profiles.get(index as usize)
    }

    pub fn macro_for_slot(&self, slot: u8) -> Option<&Macro> {
        self.macros.iter().find(|m| m.slot == slot)
    }

    /// A copy with edited values but the same baseline image.
    pub fn draft(&self) -> DeviceState {
        self.clone()
    }
}

fn read_bytes<'a>(
    image: &'a ReadImage,
    window: &str,
    profile: u8,
    offset: usize,
    len: usize,
) -> Result<&'a [u8], DecodeError> {
    let bytes = image
        .get(window, profile)
        .ok_or_else(|| DecodeError::MissingWindow {
            window: window.to_string(),
            profile,
        })?;
    if offset + len > bytes.len() {
        return Err(DecodeError::ShortWindow {
            window: window.to_string(),
            needed: offset + len,
            actual: bytes.len(),
        });
    }
    Ok(&bytes[offset..offset + len])
}

fn dpi_for_code(desc: &Descriptor, code: u16) -> Option<u16> {
    desc.tables
        .dpi_codes
        .iter()
        .find(|entry| entry.code == code)
        .map(|entry| entry.dpi)
}

pub fn code_for_dpi(desc: &Descriptor, dpi: u16) -> Option<u16> {
    desc.tables
        .dpi_codes
        .iter()
        .find(|entry| entry.dpi == dpi)
        .map(|entry| entry.code)
}

pub fn decode(desc: &Descriptor, image: &ReadImage) -> Result<DeviceState, DecodeError> {
    let profiles = desc.profile_count();
    let mut out = Vec::with_capacity(profiles as usize);

    // The model's active-profile register, when it has one.
    let active_profile = match (&desc.profiles.active_window, &desc.dpi) {
        (Some(window), _) => read_bytes(image, window, 0, 0, 1)?[0].min(profiles.saturating_sub(1)),
        (None, Some(dpi)) => {
            read_bytes(image, &dpi.window, 0, dpi.active_window_offset as usize, 1)?[0]
                .min(profiles.saturating_sub(1))
        }
        (None, None) => 0,
    };

    for profile in 0..profiles {
        let mut state = ProfileState::default();
        if let Some(dpi) = &desc.dpi {
            let active = read_bytes(
                image,
                &dpi.window,
                profile,
                dpi.active_window_offset as usize,
                1,
            )?[0]
                .min(dpi.stage_count.saturating_sub(1));
            let mut stages = Vec::with_capacity(dpi.stage_count as usize);
            for stage in 0..dpi.stage_count {
                let offset =
                    dpi.stage_window_offset as usize + dpi.stage_stride as usize * stage as usize;
                let bytes = read_bytes(image, &dpi.window, profile, offset, 3)?;
                let raw = u16::from_le_bytes([bytes[1], bytes[2]]);
                let value = dpi_for_code(desc, raw).unwrap_or(0);
                stages.push(DpiStage {
                    enabled: bytes[0] != 0,
                    x: value,
                    y: value,
                    raw,
                });
            }
            state.dpi = Some(ProfileDpi {
                stages,
                linked: vec![true; dpi.stage_count as usize],
                active,
            });
        }
        if let Some(polling) = &desc.polling {
            for source in &polling.sources {
                if image.get(&source.window, profile).is_none() {
                    continue;
                }
                let bytes = read_bytes(image, &source.window, profile, source.offset as usize, 1)?;
                let code = bytes[0];
                if let Some(rate) = polling.codes.iter().find(|c| c.code == code) {
                    state.polling_hz = Some(rate.hz);
                    break;
                }
            }
        }
        if let Some(lighting) = &desc.lighting {
            let color = read_bytes(
                image,
                &lighting.window,
                profile,
                lighting.color_offset as usize,
                3,
            )?;
            let mode_byte = read_bytes(
                image,
                &lighting.window,
                profile,
                lighting.mode_offset as usize,
                1,
            )?[0];
            let mode_code = match lighting.mode2_offset {
                Some(offset) => {
                    let second =
                        read_bytes(image, &lighting.window, profile, offset as usize, 1)?[0];
                    ((mode_byte as u16) << 8) | second as u16
                }
                None => mode_byte as u16,
            };
            let speed = read_bytes(
                image,
                &lighting.window,
                profile,
                lighting.speed_offset as usize,
                1,
            )?[0];
            let brightness = read_bytes(
                image,
                &lighting.window,
                profile,
                lighting.brightness_offset as usize,
                1,
            )?[0];
            let mut zone_colors = Vec::with_capacity(lighting.zones.len());
            for (zone_index, zone) in lighting.zones.iter().enumerate() {
                let color = match &zone.read {
                    Some(read) => {
                        let bytes = read_bytes(
                            image,
                            &read.window,
                            zone_index as u8,
                            read.offset as usize,
                            3,
                        )?;
                        [bytes[0], bytes[1], bytes[2]]
                    }
                    None => [color[0], color[1], color[2]],
                };
                zone_colors.push(color);
            }
            state.lighting = Some(ProfileLighting {
                mode: lighting
                    .mode_by_code(mode_code)
                    .map(|m| m.id.clone())
                    .unwrap_or_default(),
                mode_code,
                color: [color[0], color[1], color[2]],
                speed,
                brightness,
                zone_colors,
            });
        }
        if let Some(buttons) = &desc.buttons {
            let mut bindings = Vec::with_capacity(buttons.count as usize);
            for button in 0..buttons.count {
                let offset = buttons.window_offset as usize
                    + buttons.entry_stride as usize * button as usize;
                let bytes = read_bytes(image, &buttons.window, profile, offset, 4)?;
                let raw = [bytes[0], bytes[1], bytes[2], bytes[3]];
                let action = desc.tables.actions.iter().find(|a| a.code == raw[0]);
                let action_id = action
                    .map(|a| a.id.clone())
                    .unwrap_or_else(|| format!("code:0x{:02x}", raw[0]));
                let usage = u16::from_le_bytes([raw[1], raw[2]]);
                let key = desc
                    .tables
                    .keys
                    .iter()
                    .find(|k| k.usage == usage)
                    .map(|k| k.id.clone());
                let macro_slot = action
                    .filter(|a| a.kind == ActionKind::Macro)
                    .map(|_| raw[1]);
                let mut combo = Vec::new();
                if let (Some(combo_window), true) = (&buttons.combo_window, buttons.combo_slots > 0)
                {
                    for slot in 0..buttons.combo_slots {
                        let at =
                            buttons.combo_stride as usize * button as usize + 2 * slot as usize;
                        let bytes = read_bytes(image, combo_window, profile, at, 2)?;
                        let usage = u16::from_le_bytes([bytes[0], bytes[1]]);
                        if usage == 0 {
                            break;
                        }
                        if let Some(key) = desc
                            .tables
                            .keys
                            .iter()
                            .find(|k| k.usage == usage)
                            .map(|k| k.id.clone())
                        {
                            combo.push(key);
                        }
                    }
                }
                bindings.push(ButtonBinding {
                    action: action_id,
                    key,
                    combo,
                    macro_slot,
                    profile_target: None,
                    raw,
                });
            }
            state.buttons = Some(bindings);
        }
        out.push(state);
    }

    let device = decode_device(desc, image)?;
    let macros = decode_macros(desc, image)?;
    let state = DeviceState {
        active_profile,
        profiles: out,
        device,
        macros,
        image: image.clone(),
    };
    if state.profiles.is_empty() {
        return Err(DecodeError::ProfileOutOfRange(0));
    }
    Ok(state)
}

fn decode_device(desc: &Descriptor, image: &ReadImage) -> Result<DeviceSettings, DecodeError> {
    let Some(spec) = &desc.device else {
        return Ok(DeviceSettings::default());
    };
    let mut settings = DeviceSettings::default();
    if let Some(field) = &spec.angle_snap {
        let byte = read_bytes(image, &spec.window, 0, field.window_offset as usize, 1)?[0];
        settings.angle_snap = Some(byte == field.on);
    }
    if let Some(field) = &spec.lift_off {
        let byte = read_bytes(image, &spec.window, 0, field.window_offset as usize, 1)?[0];
        settings.lift_off = field
            .values
            .iter()
            .find(|v| v.code == byte)
            .map(|v| v.id.clone());
    }
    if let Some(field) = &spec.scroll {
        let bytes = read_bytes(image, &spec.window, 0, field.window_offset as usize, 1)?;
        settings.scroll = Some(bytes[0] as u16);
    }
    if let Some(field) = &spec.scroll_page {
        let byte = read_bytes(image, &spec.window, 0, field.window_offset as usize, 1)?[0];
        settings.scroll_page = Some(byte == field.on);
    }
    if let Some(field) = &spec.accel {
        let byte = read_bytes(image, &spec.window, 0, field.window_offset as usize, 1)?[0];
        settings.accel = Some(byte == field.on);
    }
    if let Some(field) = &spec.double_click {
        let bytes = read_bytes(image, &spec.window, 0, field.window_offset as usize, 1)?;
        settings.double_click = Some(bytes[0] as u16);
    }
    Ok(settings)
}

fn decode_macros(desc: &Descriptor, image: &ReadImage) -> Result<Vec<Macro>, DecodeError> {
    let Some(spec) = &desc.macros else {
        return Ok(Vec::new());
    };
    let mut macros = Vec::with_capacity(spec.slots as usize);
    for slot in 0..spec.slots {
        let bytes = image
            .get(&spec.window, slot)
            .map(|b| b.to_vec())
            .unwrap_or_default();
        let actions = decode_macro_actions(desc, &bytes);
        macros.push(Macro {
            slot,
            name: String::new(),
            actions,
            burst: None,
        });
    }
    Ok(macros)
}

pub fn decode_macro_actions(desc: &Descriptor, bytes: &[u8]) -> Vec<MacroAction> {
    let Some(spec) = &desc.macros else {
        return Vec::new();
    };
    let record = spec.action_len as usize;
    let mut actions = Vec::new();
    for chunk in bytes.chunks(record).take(spec.max_actions as usize) {
        if chunk.len() < record {
            break;
        }
        let kind = chunk[0];
        if kind == 0 {
            break;
        }
        let action = match kind {
            0x01 => MacroAction {
                kind: MacroActionKind::KeyDown,
                key: key_for_usage(desc, u16::from_le_bytes([chunk[1], chunk[2]])),
                button: None,
                delay_ms: 0,
            },
            0x02 => MacroAction {
                kind: MacroActionKind::KeyUp,
                key: key_for_usage(desc, u16::from_le_bytes([chunk[1], chunk[2]])),
                button: None,
                delay_ms: 0,
            },
            0x03 => MacroAction {
                kind: MacroActionKind::MouseDown,
                key: None,
                button: Some(chunk[1]),
                delay_ms: 0,
            },
            0x04 => MacroAction {
                kind: MacroActionKind::MouseUp,
                key: None,
                button: Some(chunk[1]),
                delay_ms: 0,
            },
            0x05 => MacroAction {
                kind: MacroActionKind::Delay,
                key: None,
                button: None,
                delay_ms: u16::from_le_bytes([chunk[1], chunk[2]]),
            },
            _ => break,
        };
        actions.push(action);
    }
    actions
}

pub fn key_for_usage(desc: &Descriptor, usage: u16) -> Option<String> {
    desc.tables
        .keys
        .iter()
        .find(|k| k.usage == usage)
        .map(|k| k.id.clone())
}

pub fn usage_for_key(desc: &Descriptor, key: &str) -> Option<u16> {
    desc.tables
        .keys
        .iter()
        .find(|k| k.id == key)
        .map(|k| k.usage)
}

pub fn encode_macro_actions(desc: &Descriptor, actions: &[MacroAction]) -> Option<Vec<u8>> {
    let spec = desc.macros.as_ref()?;
    let record = spec.action_len as usize;
    let mut out = vec![0u8; spec.record_len as usize];
    for (index, action) in actions.iter().enumerate() {
        if index >= spec.max_actions as usize {
            return None;
        }
        let start = index * record;
        let chunk = &mut out[start..start + record];
        match action.kind {
            MacroActionKind::KeyDown | MacroActionKind::KeyUp => {
                let usage = usage_for_key(desc, action.key.as_deref()?)?;
                chunk[0] = if action.kind == MacroActionKind::KeyDown {
                    0x01
                } else {
                    0x02
                };
                chunk[1..3].copy_from_slice(&usage.to_le_bytes());
            }
            MacroActionKind::MouseDown | MacroActionKind::MouseUp => {
                chunk[0] = if action.kind == MacroActionKind::MouseDown {
                    0x03
                } else {
                    0x04
                };
                chunk[1] = action.button?;
            }
            MacroActionKind::Delay => {
                chunk[0] = 0x05;
                chunk[1..3].copy_from_slice(&action.delay_ms.to_le_bytes());
            }
        }
    }
    Some(out)
}

/// One register write derived from the typed state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteOp {
    pub id: String,
    pub label: String,
    pub profile: Option<u8>,
    pub report: u8,
    pub addr: u16,
    pub payload: Vec<u8>,
}

impl WriteOp {
    pub fn describe(&self) -> String {
        match self.profile {
            Some(profile) => format!("{} (profile {})", self.label, profile + 1),
            None => self.label.clone(),
        }
    }

    pub fn hex_payload(&self) -> String {
        self.payload
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Pad `bytes` with zeros up to `len` (never truncates).
fn padded(mut bytes: Vec<u8>, len: u8) -> Vec<u8> {
    if bytes.len() < len as usize {
        bytes.resize(len as usize, 0);
    }
    bytes
}

pub fn encode_ops(desc: &Descriptor, state: &DeviceState) -> Vec<WriteOp> {
    let mut ops = Vec::new();
    if let Some(dpi) = &desc.dpi {
        let pages = desc.window_instances_for(&dpi.window).unwrap_or_default();
        for (profile, profile_state) in state.profiles.iter().enumerate() {
            let profile = profile as u8;
            let Some(value) = &profile_state.dpi else {
                continue;
            };
            let Some(page) = pages.iter().find(|page| page.profile == profile) else {
                continue;
            };
            if value.active < dpi.stage_count {
                ops.push(WriteOp {
                    id: format!("dpi.active:{profile}"),
                    label: "Active DPI stage".into(),
                    profile: Some(profile),
                    report: dpi.write_report,
                    addr: dpi.active_addr_at(page.addr),
                    payload: padded(vec![value.active], dpi.active_len),
                });
            }
            for (stage, stage_value) in value.stages.iter().enumerate() {
                let raw_x = if stage_value.x == 0 {
                    stage_value.raw
                } else {
                    code_for_dpi(desc, stage_value.x).unwrap_or(stage_value.raw)
                };
                let raw_y = if stage_value.y == 0 {
                    stage_value.raw
                } else {
                    code_for_dpi(desc, stage_value.y).unwrap_or(stage_value.raw)
                };
                let payload = if dpi.per_axis {
                    vec![
                        stage_value.enabled as u8,
                        (raw_x & 0xff) as u8,
                        (raw_x >> 8) as u8,
                        (raw_y & 0xff) as u8,
                        (raw_y >> 8) as u8,
                    ]
                } else {
                    padded(
                        vec![
                            stage_value.enabled as u8,
                            (raw_x & 0xff) as u8,
                            (raw_x >> 8) as u8,
                        ],
                        dpi.stage_len,
                    )
                };
                ops.push(WriteOp {
                    id: format!("dpi.stage:{profile}:{stage}"),
                    label: format!(
                        "DPI stage {} {}",
                        stage + 1,
                        if stage_value.enabled {
                            "enabled"
                        } else {
                            "disabled"
                        }
                    ),
                    profile: Some(profile),
                    report: dpi.write_report,
                    addr: dpi.stage_addr_at(page.addr, stage as u8),
                    payload,
                });
            }
        }
    }
    if let Some(polling) = &desc.polling {
        for (profile, profile_state) in state.profiles.iter().enumerate() {
            let Some(hz) = profile_state.polling_hz else {
                continue;
            };
            let Some(code) = polling.codes.iter().find(|c| c.hz == hz).map(|c| c.code) else {
                continue;
            };
            if let Some(addr) = polling.write_addrs.get(profile) {
                ops.push(WriteOp {
                    id: format!("polling:{profile}"),
                    label: format!("Polling rate {hz} Hz"),
                    profile: Some(profile as u8),
                    report: 2,
                    addr: *addr,
                    payload: vec![code],
                });
            }
        }
    }
    if let Some(lighting) = &desc.lighting {
        for (profile, profile_state) in state.profiles.iter().enumerate() {
            let Some(value) = &profile_state.lighting else {
                continue;
            };
            let Some(addr) = lighting.write_addrs.get(profile) else {
                continue;
            };
            let profile = profile as u8;
            // The vendor writes the whole record (colour, both mode bytes,
            // speed, brightness) in one frame, so do the same. Bytes the model
            // does not name keep the value that was read.
            let mut record = state
                .image
                .get(&lighting.window, profile)
                .map(|bytes| bytes.to_vec())
                .unwrap_or_default();
            record.resize(lighting.record_len() as usize, 0);
            if lighting.zones.is_empty() {
                let at = lighting.color_offset as usize;
                record[at..at + 3].copy_from_slice(&value.color);
            }
            let mode_code = if value.mode.is_empty() {
                value.mode_code
            } else {
                lighting
                    .mode(&value.mode)
                    .map(|m| m.code)
                    .unwrap_or(value.mode_code)
            };
            let (first, second) = lighting.mode_bytes(mode_code);
            record[lighting.mode_offset as usize] = first;
            if let Some(offset) = lighting.mode2_offset {
                record[offset as usize] = second;
            }
            record[lighting.speed_offset as usize] = value.speed;
            record[lighting.brightness_offset as usize] = value.brightness;
            ops.push(WriteOp {
                id: format!("lighting:{profile}"),
                label: "LED settings".into(),
                profile: Some(profile),
                report: 2,
                addr: *addr,
                payload: record,
            });
        }
        for (zone_index, zone) in lighting.zones.iter().enumerate() {
            for profile_state in state.profiles.iter() {
                if let Some(value) = &profile_state.lighting {
                    let color = value
                        .zone_colors
                        .get(zone_index)
                        .copied()
                        .unwrap_or(value.color);
                    ops.push(WriteOp {
                        id: format!("lighting.zone:{}", zone.id),
                        label: format!("{} colour", zone.label),
                        profile: None,
                        report: 2,
                        addr: zone.write_addr,
                        payload: color.to_vec(),
                    });
                    break;
                }
            }
        }
    }
    if let Some(buttons) = &desc.buttons {
        for (profile, profile_state) in state.profiles.iter().enumerate() {
            let Some(bindings) = &profile_state.buttons else {
                continue;
            };
            let Some(base) = buttons.write_addrs.get(profile) else {
                continue;
            };
            for (index, binding) in bindings.iter().enumerate() {
                let code = desc
                    .tables
                    .actions
                    .iter()
                    .find(|a| a.id == binding.action)
                    .map(|a| a.code)
                    .unwrap_or(binding.raw[0]);
                let usage = binding
                    .key
                    .as_deref()
                    .and_then(|key| usage_for_key(desc, key))
                    .unwrap_or(u16::from_le_bytes([binding.raw[1], binding.raw[2]]));
                let mut payload = binding.raw;
                payload[0] = code;
                payload[1..3].copy_from_slice(&usage.to_le_bytes());
                if let Some(slot) = binding.macro_slot {
                    payload[1] = slot;
                }
                ops.push(WriteOp {
                    id: format!("buttons:{profile}:{index}"),
                    label: format!(
                        "{} button mapping",
                        buttons
                            .names
                            .get(index)
                            .cloned()
                            .unwrap_or_else(|| format!("Button {}", index + 1))
                    ),
                    profile: Some(profile as u8),
                    report: buttons.write_report,
                    addr: base.wrapping_add(buttons.entry_stride * index as u16),
                    payload: payload.to_vec(),
                });
                if let (Some(combo_addr), true) = (buttons.combo_addr, buttons.combo_slots > 0) {
                    for slot in 0..buttons.combo_slots {
                        let usage = binding
                            .combo
                            .get(slot as usize)
                            .and_then(|key| usage_for_key(desc, key))
                            .unwrap_or(0);
                        ops.push(WriteOp {
                            id: format!("buttons.combo:{profile}:{index}:{slot}"),
                            label: format!("Button {} combo slot {}", index + 1, slot + 1),
                            profile: Some(profile as u8),
                            report: buttons.write_report,
                            addr: combo_addr
                                .wrapping_add(buttons.combo_profile_stride * profile as u16)
                                .wrapping_add(buttons.combo_stride * index as u16)
                                .wrapping_add(2 * slot as u16),
                            payload: usage.to_le_bytes().to_vec(),
                        });
                    }
                }
            }
        }
    }
    if let Some(spec) = &desc.macros {
        for macro_state in &state.macros {
            if macro_state.slot >= spec.slots {
                continue;
            }
            if let Some(payload) = encode_macro_actions(desc, &macro_state.actions) {
                ops.push(WriteOp {
                    id: format!("macros:{}", macro_state.slot),
                    label: format!("Macro slot {}", macro_state.slot + 1),
                    profile: None,
                    report: spec.report,
                    addr: spec.slot_addr(macro_state.slot),
                    payload,
                });
            }
        }
        if let Some(burst) = &spec.burst {
            for macro_state in &state.macros {
                let Some(values) = &macro_state.burst else {
                    continue;
                };
                if let Some(buttons) = &desc.buttons {
                    for index in 0..buttons.count {
                        ops.push(WriteOp {
                            id: format!("burst:{index}"),
                            label: format!("Burst fire for button {}", index + 1),
                            profile: None,
                            report: 2,
                            addr: burst.addr.wrapping_add(burst.stride * index as u16),
                            payload: vec![
                                values.bullets,
                                (values.delay_ms / 10).min(u8::MAX as u16) as u8,
                            ],
                        });
                    }
                }
            }
        }
    }
    if let Some(spec) = &desc.device {
        if let Some(field) = &spec.angle_snap {
            if let Some(value) = state.device.angle_snap {
                ops.push(WriteOp {
                    id: "device.angle".into(),
                    label: "Angle snap".into(),
                    profile: None,
                    report: 2,
                    addr: field.addr.wrapping_add(field.offset),
                    payload: vec![if value { field.on } else { field.off }],
                });
            }
        }
        if let Some(field) = &spec.lift_off {
            if let Some(value) = state.device.lift_off.as_deref() {
                if let Some(code) = field.values.iter().find(|v| v.id == value).map(|v| v.code) {
                    ops.push(WriteOp {
                        id: "device.liftoff".into(),
                        label: "Lift-off distance".into(),
                        profile: None,
                        report: 2,
                        addr: field.addr.wrapping_add(field.offset),
                        payload: vec![code],
                    });
                }
            }
        }
        if let Some(field) = &spec.scroll {
            if let Some(value) = state.device.scroll {
                ops.push(WriteOp {
                    id: "device.scroll".into(),
                    label: "Scroll speed".into(),
                    profile: None,
                    report: 2,
                    addr: field.addr.wrapping_add(field.offset),
                    payload: vec![value.min(255) as u8],
                });
            }
        }
        if let Some(field) = &spec.scroll_page {
            if let Some(value) = state.device.scroll_page {
                ops.push(WriteOp {
                    id: "device.scroll_page".into(),
                    label: "Scroll by page".into(),
                    profile: None,
                    report: 2,
                    addr: field.addr.wrapping_add(field.offset),
                    payload: vec![if value { field.on } else { field.off }],
                });
            }
        }
        if let Some(field) = &spec.accel {
            if let Some(value) = state.device.accel {
                ops.push(WriteOp {
                    id: "device.accel".into(),
                    label: "Pointer acceleration".into(),
                    profile: None,
                    report: 2,
                    addr: field.addr.wrapping_add(field.offset),
                    payload: vec![if value { field.on } else { field.off }],
                });
            }
        }
        if let Some(field) = &spec.double_click {
            if let Some(value) = state.device.double_click {
                ops.push(WriteOp {
                    id: "device.double_click".into(),
                    label: "Double-click speed".into(),
                    profile: None,
                    report: 2,
                    addr: field.addr.wrapping_add(field.offset),
                    payload: vec![value.min(255) as u8],
                });
            }
        }
    }
    ops
}

/// Per-field validation failures that block an apply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldError {
    pub field: String,
    pub message: String,
}

pub fn validate_state(desc: &Descriptor, state: &DeviceState) -> Vec<FieldError> {
    let mut errors = Vec::new();
    if state.profiles.len() != desc.profile_count() as usize {
        errors.push(FieldError {
            field: "profiles".into(),
            message: format!(
                "state holds {} profiles but the model declares {}",
                state.profiles.len(),
                desc.profile_count()
            ),
        });
    }
    if let Some(dpi) = &desc.dpi {
        for (profile, profile_state) in state.profiles.iter().enumerate() {
            let Some(value) = &profile_state.dpi else {
                continue;
            };
            let enabled: Vec<u8> = value
                .stages
                .iter()
                .enumerate()
                .filter(|(_, s)| s.enabled)
                .map(|(i, _)| i as u8)
                .collect();
            if enabled.is_empty() {
                errors.push(FieldError {
                    field: format!("dpi.stage:{profile}"),
                    message: "at least one DPI stage must stay enabled".into(),
                });
            }
            if value.stages.len() != dpi.stage_count as usize {
                errors.push(FieldError {
                    field: format!("dpi.stage:{profile}"),
                    message: format!(
                        "profile {} has {} stages, the model declares {}",
                        profile + 1,
                        value.stages.len(),
                        dpi.stage_count
                    ),
                });
            }
            if !enabled.contains(&value.active) {
                errors.push(FieldError {
                    field: format!("dpi.active:{profile}"),
                    message: format!(
                        "profile {} selects disabled stage {}",
                        profile + 1,
                        value.active + 1
                    ),
                });
            }
            for (stage, stage_value) in value.stages.iter().enumerate() {
                if stage_value.x != 0 && code_for_dpi(desc, stage_value.x).is_none() {
                    errors.push(FieldError {
                        field: format!("dpi.stage:{profile}:{stage}"),
                        message: format!("{} DPI is not in this model's code table", stage_value.x),
                    });
                }
                if dpi.per_axis && stage_value.y != 0 && code_for_dpi(desc, stage_value.y).is_none()
                {
                    errors.push(FieldError {
                        field: format!("dpi.stage:{profile}:{stage}"),
                        message: format!("{} DPI is not in this model's code table", stage_value.y),
                    });
                }
                if !dpi.per_axis && stage_value.y != 0 && stage_value.y != stage_value.x {
                    errors.push(FieldError {
                        field: format!("dpi.stage:{profile}:{stage}"),
                        message: "this model does not support per-axis DPI".into(),
                    });
                }
            }
        }
    }
    if let Some(polling) = &desc.polling {
        for (profile, profile_state) in state.profiles.iter().enumerate() {
            if let Some(hz) = profile_state.polling_hz {
                if !polling.codes.iter().any(|c| c.hz == hz) {
                    errors.push(FieldError {
                        field: format!("polling:{profile}"),
                        message: format!("{hz} Hz is not supported by this model"),
                    });
                }
            }
        }
    }
    if let Some(lighting) = &desc.lighting {
        for (profile, profile_state) in state.profiles.iter().enumerate() {
            let Some(value) = &profile_state.lighting else {
                continue;
            };
            if !value.mode.is_empty() && lighting.mode(&value.mode).is_none() {
                errors.push(FieldError {
                    field: format!("lighting.mode:{profile}"),
                    message: format!("`{}` is not a mode this model declares", value.mode),
                });
            }
            if value.speed >= lighting.speed_levels {
                errors.push(FieldError {
                    field: format!("lighting.speed:{profile}"),
                    message: format!(
                        "speed {} is above the {} levels this model declares",
                        value.speed, lighting.speed_levels
                    ),
                });
            }
            if value.brightness >= lighting.brightness_levels {
                errors.push(FieldError {
                    field: format!("lighting.brightness:{profile}"),
                    message: format!(
                        "brightness {} is above the {} levels this model declares",
                        value.brightness, lighting.brightness_levels
                    ),
                });
            }
        }
    }
    if desc.buttons.is_some() {
        for (profile, profile_state) in state.profiles.iter().enumerate() {
            let Some(bindings) = &profile_state.buttons else {
                continue;
            };
            for (index, binding) in bindings.iter().enumerate() {
                if desc.tables.actions.iter().all(|a| a.id != binding.action) {
                    errors.push(FieldError {
                        field: format!("buttons:{profile}:{index}"),
                        message: format!(
                            "`{}` is not an action this model declares",
                            binding.action
                        ),
                    });
                    continue;
                }
                if let Some(key) = &binding.key {
                    if usage_for_key(desc, key).is_none() {
                        errors.push(FieldError {
                            field: format!("buttons:{profile}:{index}"),
                            message: format!("`{key}` is not in this model's key table"),
                        });
                    }
                }
                if let Some(slot) = binding.macro_slot {
                    let slots = desc.macros.as_ref().map(|m| m.slots).unwrap_or(0);
                    if slot >= slots {
                        errors.push(FieldError {
                            field: format!("buttons:{profile}:{index}"),
                            message: format!("macro slot {slot} is outside the {slots} slots"),
                        });
                    }
                }
                let combo_limit = desc.buttons.as_ref().map(|b| b.combo_slots).unwrap_or(0);
                if binding.combo.len() > combo_limit as usize {
                    errors.push(FieldError {
                        field: format!("buttons:{profile}:{index}"),
                        message: format!(
                            "combo has {} keys but this model stores {combo_limit}",
                            binding.combo.len()
                        ),
                    });
                }
                for key in &binding.combo {
                    if usage_for_key(desc, key).is_none() {
                        errors.push(FieldError {
                            field: format!("buttons:{profile}:{index}"),
                            message: format!("`{key}` is not in this model's key table"),
                        });
                    }
                }
            }
        }
    }
    if let Some(spec) = &desc.macros {
        for macro_state in &state.macros {
            if macro_state.actions.len() > spec.max_actions as usize {
                errors.push(FieldError {
                    field: format!("macros:{}", macro_state.slot),
                    message: format!(
                        "{} actions exceed the {} this model stores",
                        macro_state.actions.len(),
                        spec.max_actions
                    ),
                });
            }
            for action in &macro_state.actions {
                match action.kind {
                    MacroActionKind::KeyDown | MacroActionKind::KeyUp => {
                        match action.key.as_deref() {
                            None => errors.push(FieldError {
                                field: format!("macros:{}", macro_state.slot),
                                message: "a key action needs a key".into(),
                            }),
                            Some(key) if usage_for_key(desc, key).is_none() => {
                                errors.push(FieldError {
                                    field: format!("macros:{}", macro_state.slot),
                                    message: format!("`{key}` is not in this model's key table"),
                                })
                            }
                            _ => {}
                        }
                    }
                    MacroActionKind::Delay if action.delay_ms > 10_000 => {
                        errors.push(FieldError {
                            field: format!("macros:{}", macro_state.slot),
                            message: "delays above 10 s cannot be encoded".into(),
                        });
                    }
                    _ => {}
                }
            }
        }
    }
    if let Some(spec) = &desc.device {
        if let Some(field) = &spec.lift_off {
            if let Some(value) = state.device.lift_off.as_deref() {
                if field.values.iter().all(|v| v.id != value) {
                    errors.push(FieldError {
                        field: "device.liftoff".into(),
                        message: format!("`{value}` is not a lift-off value this model declares"),
                    });
                }
            }
        }
        if let Some(field) = &spec.double_click {
            if let Some(value) = state.device.double_click {
                if value < field.min || value > field.max {
                    errors.push(FieldError {
                        field: "device.double_click".into(),
                        message: format!(
                            "{value} is outside the declared range {}..{}",
                            field.min, field.max
                        ),
                    });
                }
            }
        }
        if let Some(field) = &spec.scroll {
            if let Some(value) = state.device.scroll {
                if value < field.min || value > field.max {
                    errors.push(FieldError {
                        field: "device.scroll".into(),
                        message: format!(
                            "{value} is outside the declared range {}..{}",
                            field.min, field.max
                        ),
                    });
                }
            }
        }
    }
    errors
}

/// Seed an image with plausible values so the simulator is usable and the
/// read-consistency gate sees non-blank data.
///
/// Values are laid out in a flat per-report memory first, then every read
/// window is filled from it. Windows overlap on the real device (a polling
/// window covers neighbouring profiles' bytes), so deriving them from one
/// memory keeps overlapping reads consistent.
pub fn seed_image(desc: &Descriptor) -> ReadImage {
    use std::collections::BTreeMap;

    let mut mem: BTreeMap<(u8, u16), u8> = BTreeMap::new();
    let mut put = |report: u8, addr: u16, bytes: &[u8]| {
        for (i, byte) in bytes.iter().enumerate() {
            mem.insert((report, addr.wrapping_add(i as u16)), *byte);
        }
    };

    let dpi_pages = desc
        .dpi
        .as_ref()
        .and_then(|dpi| desc.window_instances_for(&dpi.window).ok())
        .unwrap_or_default();
    for profile in desc.profile_indices() {
        if let Some(dpi) = &desc.dpi {
            let Some(page) = dpi_pages.iter().find(|page| page.profile == profile) else {
                continue;
            };
            let active_len = dpi.active_len.max(1) as usize;
            put(
                page.report,
                dpi.active_addr_at(page.addr),
                &vec![0u8; active_len],
            );
            for stage in 0..dpi.stage_count {
                let code = desc
                    .tables
                    .dpi_codes
                    .get(stage as usize % desc.tables.dpi_codes.len().max(1))
                    .map(|entry| entry.code)
                    .unwrap_or(0x16);
                let payload = if dpi.per_axis {
                    vec![
                        1,
                        (code & 0xff) as u8,
                        (code >> 8) as u8,
                        (code & 0xff) as u8,
                        (code >> 8) as u8,
                    ]
                } else {
                    padded(
                        vec![1, (code & 0xff) as u8, (code >> 8) as u8],
                        dpi.stage_len,
                    )
                };
                put(page.report, dpi.stage_addr_at(page.addr, stage), &payload);
            }
        }
        if let Some(polling) = &desc.polling {
            if let Some(addr) = polling.write_addrs.get(profile as usize) {
                let code = polling
                    .codes
                    .iter()
                    .find(|c| c.hz == 500)
                    .or_else(|| polling.codes.first())
                    .map(|c| c.code)
                    .unwrap_or(0);
                put(2, *addr, &[code]);
            }
        }
        if let Some(lighting) = &desc.lighting {
            if let Some(addr) = lighting.write_addrs.get(profile as usize) {
                put(
                    2,
                    addr.wrapping_add(lighting.color_offset),
                    &[0xFF, 0xFF, 0xFF],
                );
                let mode = lighting.modes.first().map(|m| m.code).unwrap_or(0);
                let (first, second) = lighting.mode_bytes(mode);
                put(2, addr.wrapping_add(lighting.mode_offset), &[first]);
                if let Some(offset) = lighting.mode2_offset {
                    put(2, addr.wrapping_add(offset), &[second]);
                }
                put(2, addr.wrapping_add(lighting.speed_offset), &[1]);
                put(
                    2,
                    addr.wrapping_add(lighting.brightness_offset),
                    &[lighting.brightness_levels.saturating_sub(1)],
                );
            }
        }
        if let Some(buttons) = &desc.buttons {
            if let Some(base) = buttons.write_addrs.get(profile as usize) {
                let action = desc
                    .tables
                    .actions
                    .iter()
                    .find(|a| a.kind == ActionKind::Default)
                    .map(|a| a.code)
                    .unwrap_or(0);
                for button in 0..buttons.count {
                    put(
                        buttons.write_report,
                        base.wrapping_add(buttons.entry_stride * button as u16),
                        &[action, 0, 0, 0],
                    );
                }
            }
        }
    }
    if let Some(lighting) = &desc.lighting {
        for zone in &lighting.zones {
            put(2, zone.write_addr, &[0xFF, 0xFF, 0xFF]);
        }
    }
    if let Some(device) = &desc.device {
        if let Some(field) = &device.angle_snap {
            put(2, field.addr.wrapping_add(field.offset), &[field.off]);
        }
        if let Some(field) = &device.lift_off {
            if let Some(first) = field.values.first() {
                put(2, field.addr.wrapping_add(field.offset), &[first.code]);
            }
        }
        if let Some(field) = &device.scroll {
            put(2, field.addr.wrapping_add(field.offset), &[field.min as u8]);
        }
        if let Some(field) = &device.scroll_page {
            put(2, field.addr.wrapping_add(field.offset), &[field.off]);
        }
        if let Some(field) = &device.accel {
            put(2, field.addr.wrapping_add(field.offset), &[field.off]);
        }
        if let Some(field) = &device.double_click {
            put(2, field.addr.wrapping_add(field.offset), &[field.min as u8]);
        }
    }

    let mut image = ReadImage::default();
    for instance in desc.window_instances().unwrap_or_default() {
        let bytes = (0..instance.len)
            .map(|i| {
                mem.get(&(instance.report, instance.addr.wrapping_add(i)))
                    .copied()
                    .unwrap_or(0)
            })
            .collect();
        image.insert(
            &instance.id,
            instance.profile,
            WindowData {
                report: instance.report,
                addr: instance.addr,
                bytes,
            },
        );
    }
    image
}

pub fn seed_state(desc: &Descriptor) -> DeviceState {
    let image = seed_image(desc);
    decode(desc, &image).unwrap_or_else(|_| DeviceState {
        active_profile: 0,
        profiles: vec![ProfileState::default(); desc.profile_count() as usize],
        device: DeviceSettings::default(),
        macros: Vec::new(),
        image,
    })
}

/// Report-block write detection, used by the pacer and the change summary.
pub fn is_block_report(report: u8) -> bool {
    report == REPORT_BLOCK
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Descriptor;

    fn desc() -> Descriptor {
        Descriptor::from_toml(include_str!("../../models/holtek-04d9-fc5c.toml"))
            .expect("descriptor")
    }

    #[test]
    fn seeded_state_decodes_typed_values() {
        let desc = desc();
        let state = seed_state(&desc);
        assert_eq!(state.profiles.len(), 5);
        let dpi = state.profiles[0].dpi.as_ref().expect("dpi");
        assert_eq!(dpi.stages.len(), 5);
        assert_eq!(state.profiles[0].polling_hz, Some(500));
        let lighting = state.profiles[0].lighting.as_ref().expect("lighting");
        assert_eq!(lighting.mode, "breathing");
        assert_eq!(lighting.color, [0xFF, 0xFF, 0xFF]);
    }

    #[test]
    fn encode_decode_round_trip_is_byte_identical() {
        let desc = desc();
        let state = seed_state(&desc);
        let ops = encode_ops(&desc, &state);
        assert!(!ops.is_empty());
        let mut image = state.image.clone();
        for op in &ops {
            if let Some(window) = image
                .windows
                .iter_mut()
                .find(|((_, profile), w)| {
                    *profile == op.profile.unwrap_or(0)
                        && w.report == op.report
                        && w.addr <= op.addr
                })
                .map(|(_, w)| w)
            {
                let start = (op.addr - window.addr) as usize;
                if start + op.payload.len() <= window.bytes.len() {
                    window.bytes[start..start + op.payload.len()].copy_from_slice(&op.payload);
                }
            }
        }
        let decoded = decode(&desc, &image).expect("decode");
        let ops_again = encode_ops(&desc, &decoded);
        let mut first: Vec<_> = ops
            .iter()
            .map(|o| (o.id.clone(), o.payload.clone()))
            .collect();
        let mut second: Vec<_> = ops_again
            .iter()
            .map(|o| (o.id.clone(), o.payload.clone()))
            .collect();
        first.sort();
        second.sort();
        assert_eq!(first, second);
    }

    #[test]
    fn disabled_last_stage_is_rejected() {
        let desc = desc();
        let mut state = seed_state(&desc);
        let dpi = state.profiles[0].dpi.as_mut().expect("dpi");
        for stage in dpi.stages.iter_mut() {
            stage.enabled = false;
        }
        let errors = validate_state(&desc, &state);
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("at least one DPI stage")),
            "{errors:?}"
        );
    }

    #[test]
    fn unsupported_dpi_value_is_rejected() {
        let desc = desc();
        let mut state = seed_state(&desc);
        state.profiles[0].dpi.as_mut().unwrap().stages[0].x = 1234;
        let errors = validate_state(&desc, &state);
        assert!(
            errors.iter().any(|e| e.message.contains("code table")),
            "{errors:?}"
        );
    }

    #[test]
    fn unsupported_polling_rate_is_rejected() {
        let desc = desc();
        let mut state = seed_state(&desc);
        state.profiles[0].polling_hz = Some(2000);
        let errors = validate_state(&desc, &state);
        assert!(
            errors.iter().any(|e| e.message.contains("not supported")),
            "{errors:?}"
        );
    }

    #[test]
    fn blank_image_detection_catches_filler_reads() {
        let mut image = ReadImage::default();
        image.insert(
            "led",
            0,
            WindowData {
                report: 2,
                addr: 0x449,
                bytes: vec![0; 7],
            },
        );
        assert!(image.is_blank());
        image.insert(
            "led",
            1,
            WindowData {
                report: 2,
                addr: 0x451,
                bytes: vec![1, 2, 3],
            },
        );
        assert!(!image.is_blank());
    }
    fn fixture() -> Descriptor {
        crate::models::fixture::descriptor()
    }

    #[test]
    fn fixture_supports_per_axis_dpi_combos_and_zones() {
        let desc = fixture();
        let mut state = seed_state(&desc);
        {
            let dpi = state.profiles[0].dpi.as_mut().expect("dpi");
            dpi.stages[0].x = 800;
            dpi.stages[0].y = 1600;
            dpi.linked[0] = false;
        }
        {
            let buttons = state.profiles[0].buttons.as_mut().expect("buttons");
            buttons[1].action = "macro".into();
            buttons[1].macro_slot = Some(0);
            buttons[1].combo = vec!["ctrl".into(), "c".into()];
        }
        assert!(validate_state(&desc, &state).is_empty());
        let ops = encode_ops(&desc, &state);
        assert!(ops.iter().any(|op| op.id == "dpi.stage:0:0"));
        assert!(ops.iter().any(|op| op.id == "buttons.combo:0:1:1"));
        assert!(ops.iter().any(|op| op.id == "lighting.zone:side"));
        assert!(ops.iter().any(|op| op.id == "device.angle"));
    }

    #[test]
    fn combo_beyond_declared_limit_is_rejected() {
        let desc = fixture();
        let mut state = seed_state(&desc);
        let buttons = state.profiles[0].buttons.as_mut().expect("buttons");
        buttons[1].combo = vec!["ctrl".into(), "a".into(), "b".into(), "c".into()];
        let errors = validate_state(&desc, &state);
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("combo has 4 keys")),
            "{errors:?}"
        );
    }

    #[test]
    fn per_axis_dpi_is_rejected_on_a_single_axis_model() {
        let desc = desc();
        let mut state = seed_state(&desc);
        state.profiles[0].dpi.as_mut().unwrap().stages[0].y = 5000;
        let errors = validate_state(&desc, &state);
        assert!(
            errors.iter().any(|e| e.message.contains("per-axis")),
            "{errors:?}"
        );
    }

    #[test]
    fn fixture_zone_colours_encode_per_zone() {
        let desc = fixture();
        let mut state = seed_state(&desc);
        state.profiles[0].lighting.as_mut().unwrap().zone_colors = vec![[1, 2, 3], [4, 5, 6]];
        let ops = encode_ops(&desc, &state);
        let side = ops
            .iter()
            .find(|op| op.id == "lighting.zone:side")
            .expect("side zone op");
        assert_eq!(side.payload, vec![1, 2, 3]);
        let front = ops
            .iter()
            .find(|op| op.id == "lighting.zone:front")
            .expect("front zone op");
        assert_eq!(front.payload, vec![4, 5, 6]);
    }

    #[test]
    fn fixture_device_settings_are_encoded() {
        let desc = fixture();
        let mut state = seed_state(&desc);
        state.device.angle_snap = Some(true);
        state.device.lift_off = Some("2mm".into());
        state.device.scroll = Some(5);
        state.device.accel = Some(true);
        state.device.double_click = Some(4);
        let ops = encode_ops(&desc, &state);
        assert!(ops
            .iter()
            .any(|op| op.id == "device.angle" && op.payload == vec![1]));
        assert!(ops
            .iter()
            .any(|op| op.id == "device.liftoff" && op.payload == vec![2]));
        assert!(ops
            .iter()
            .any(|op| op.id == "device.scroll" && op.payload == vec![5]));
        assert!(ops
            .iter()
            .any(|op| op.id == "device.double_click" && op.payload == vec![4]));
        assert!(validate_state(&desc, &state).is_empty());
    }
}
