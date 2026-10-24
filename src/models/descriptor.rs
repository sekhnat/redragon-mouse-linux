//! Model descriptor types: the data that describes one mouse's register layout.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::transport::FrameSizes;

/// Descriptor schema major version this build understands.
pub const SUPPORTED_SCHEMA: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Provenance {
    /// Captured from vendor software traffic or vendor data files.
    Capture,
    /// Derived from an upstream open-source implementation.
    Upstream,
    /// Read back from the device itself.
    Readback,
}

impl Provenance {
    pub fn label(self) -> &'static str {
        match self {
            Provenance::Capture => "vendor software capture",
            Provenance::Upstream => "upstream open-source derivation",
            Provenance::Readback => "live device read-back",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Collections {
    pub usage_page: u16,
    pub usage: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameSpec {
    /// HID report ID used to carry this frame class.
    pub id: u8,
    /// Total frame length, including the report-ID byte.
    pub len: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profiles {
    pub count: u8,
    /// Read window whose first byte is the index of the profile the mouse is
    /// using. When absent the index is taken from the first DPI page.
    #[serde(default)]
    pub active_window: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadDefaults {
    #[serde(default = "default_payload_offset")]
    pub payload_offset: u16,
}

fn default_payload_offset() -> u16 {
    8
}

impl Default for ReadDefaults {
    fn default() -> Self {
        ReadDefaults {
            payload_offset: default_payload_offset(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Commands {
    #[serde(default = "default_begin_report")]
    pub begin_report: u8,
    pub begin: u8,
    pub write: u8,
    pub read: u8,
    pub commit: u8,
    pub commit_report: u8,
    /// Refresh words sent after a write batch. Each word is
    /// `(mask << 8) | flag` and becomes the frame `{id, F1, mask, flag}`: the
    /// vendor sends mask `0x02` with flags `04 01 02 08 10` after an apply.
    pub commit_frames: Vec<u16>,
    /// Register that selects the active profile (e.g. `0x002C`). When absent,
    /// activation falls back to a begin frame followed by `commit_frames`.
    #[serde(default)]
    pub activate_addr: Option<u16>,
    #[serde(default = "default_activate_len")]
    pub activate_len: u8,
    /// Commit frames used after writing the activate register.
    #[serde(default)]
    pub activate_frames: Vec<u16>,
}

fn default_activate_len() -> u8 {
    2
}

/// A read window: one `0xF2` selector plus the report it is read from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Window {
    pub report: u8,
    /// Base address; unused when `addrs` lists explicit per-profile addresses.
    #[serde(default)]
    pub addr: u16,
    pub len: u16,
    #[serde(default)]
    pub stride: i32,
    #[serde(default)]
    pub profile_base: u8,
    #[serde(default = "default_count")]
    pub count: u8,
    /// Explicit per-profile addresses, overriding `addr`/`stride`.
    #[serde(default)]
    pub addrs: Option<Vec<u16>>,
    /// False when the window is indexed by something other than a profile
    /// (macro slots, for example). Such windows are exempt from the
    /// profile-count check.
    #[serde(default = "default_true")]
    pub per_profile: bool,
}

fn default_true() -> bool {
    true
}

fn default_count() -> u8 {
    1
}

fn default_begin_report() -> u8 {
    2
}

impl Window {
    /// Concrete per-profile reads described by this window.
    pub fn expand(&self, id: &str) -> Result<Vec<WindowInstance>, String> {
        let mut out = Vec::new();
        if let Some(addrs) = &self.addrs {
            if addrs.len() != self.count as usize {
                return Err(format!(
                    "window `{id}` declares {} addresses but count {}",
                    addrs.len(),
                    self.count
                ));
            }
            for (i, addr) in addrs.iter().enumerate() {
                out.push(WindowInstance {
                    id: id.to_string(),
                    profile: self.profile_base + i as u8,
                    report: self.report,
                    addr: *addr,
                    len: self.len,
                });
            }
            return Ok(out);
        }
        for i in 0..self.count {
            let addr = self.addr as i64 + self.stride as i64 * i as i64;
            if addr < 0 || addr > u16::MAX as i64 {
                return Err(format!(
                    "window `{id}` address overflows at index {i}: {addr}"
                ));
            }
            out.push(WindowInstance {
                id: id.to_string(),
                profile: self.profile_base + i,
                report: self.report,
                addr: addr as u16,
                len: self.len,
            });
        }
        Ok(out)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInstance {
    pub id: String,
    pub profile: u8,
    pub report: u8,
    pub addr: u16,
    pub len: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DpiSpec {
    pub window: String,
    pub active_window_offset: u16,
    pub stage_window_offset: u16,
    pub stage_count: u8,
    pub write_report: u8,
    pub stage_stride: u16,
    pub stage_len: u8,
    pub active_len: u8,
    #[serde(default)]
    pub per_axis: bool,
}

impl DpiSpec {
    /// Writes use the same per-profile page address the reads used, because
    /// profile pages are not always a uniform stride apart.
    pub fn stage_addr_at(&self, page_base: u16, stage: u8) -> u16 {
        page_base
            .wrapping_add(self.stage_window_offset)
            .wrapping_add(self.stage_stride.wrapping_mul(stage as u16))
    }

    pub fn active_addr_at(&self, page_base: u16) -> u16 {
        page_base.wrapping_add(self.active_window_offset)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PollingSpec {
    pub sources: Vec<WindowRef>,
    pub write_addrs: Vec<u16>,
    #[serde(default = "default_one")]
    pub write_len: u8,
    pub codes: Vec<RateCode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowRef {
    pub window: String,
    #[serde(default)]
    pub offset: u16,
}

fn default_report_register() -> u8 {
    2
}

fn default_one() -> u8 {
    1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RateCode {
    pub hz: u16,
    pub code: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LightingSpec {
    pub window: String,
    pub write_addrs: Vec<u16>,
    pub color_offset: u16,
    pub mode_offset: u16,
    /// Second mode byte. When set, a mode's `code` is `(mode << 8) | mode2`.
    #[serde(default)]
    pub mode2_offset: Option<u16>,
    pub speed_offset: u16,
    pub brightness_offset: u16,
    pub brightness_levels: u8,
    /// Number of speed steps; `speed` is the raw register value `0..speed_levels`.
    pub speed_levels: u8,
    /// True when a larger register value means a slower effect.
    #[serde(default)]
    pub speed_inverted: bool,
    pub modes: Vec<ModeCode>,
    #[serde(default)]
    pub zones: Vec<ZoneSpec>,
}

impl LightingSpec {
    /// Bytes covered by one profile's lighting record.
    pub fn record_len(&self) -> u16 {
        [
            self.color_offset + 3,
            self.mode_offset + 1,
            self.mode2_offset.map(|o| o + 1).unwrap_or(0),
            self.speed_offset + 1,
            self.brightness_offset + 1,
        ]
        .into_iter()
        .max()
        .unwrap_or(0)
    }

    /// Split a mode code into the bytes stored at `mode_offset`/`mode2_offset`.
    pub fn mode_bytes(&self, code: u16) -> (u8, u8) {
        if self.mode2_offset.is_some() {
            ((code >> 8) as u8, code as u8)
        } else {
            (code as u8, 0)
        }
    }

    pub fn mode(&self, id: &str) -> Option<&ModeCode> {
        self.modes.iter().find(|m| m.id == id)
    }

    pub fn mode_by_code(&self, code: u16) -> Option<&ModeCode> {
        self.modes.iter().find(|m| m.code == code)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModeCode {
    pub id: String,
    pub label: String,
    /// Mode byte, or `(mode << 8) | mode2` when the model has a second mode byte.
    pub code: u16,
    #[serde(default)]
    pub uses_color: bool,
    #[serde(default)]
    pub uses_speed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ZoneSpec {
    pub id: String,
    pub label: String,
    pub write_addr: u16,
    /// Where this zone's colour can be read back, if at all.
    #[serde(default)]
    pub read: Option<WindowRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ButtonsSpec {
    pub window: String,
    pub window_offset: u16,
    pub count: u8,
    pub names: Vec<String>,
    /// Per-profile write base addresses. Empty means mappings are read-only.
    #[serde(default)]
    pub write_addrs: Vec<u16>,
    #[serde(default = "default_report_register")]
    pub write_report: u8,
    pub entry_stride: u16,
    pub entry_len: u8,
    pub primary: u8,
    #[serde(default)]
    pub combo_slots: u8,
    /// Read window holding the combo area (one instance per profile).
    #[serde(default)]
    pub combo_window: Option<String>,
    #[serde(default)]
    pub combo_addr: Option<u16>,
    #[serde(default)]
    pub combo_stride: u16,
    /// Per-profile stride for the combo area; 0 when combos are not
    /// profile-scoped.
    #[serde(default)]
    pub combo_profile_stride: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacrosSpec {
    pub slots: u8,
    pub window: String,
    pub base: u16,
    pub slot_stride: u16,
    pub report: u8,
    pub record_len: u16,
    pub action_len: u8,
    pub max_actions: u8,
    #[serde(default)]
    pub burst: Option<BurstSpec>,
}

impl MacrosSpec {
    /// Window holding one record per slot (profile index == slot).
    pub fn window(&self) -> &str {
        &self.window
    }
    pub fn slot_addr(&self, slot: u8) -> u16 {
        self.base
            .wrapping_add(self.slot_stride.wrapping_mul(slot as u16))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BurstSpec {
    pub addr: u16,
    pub stride: u16,
    pub len: u8,
    pub max_bullets: u8,
    pub min_delay_ms: u16,
    pub max_delay_ms: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceSpec {
    pub window: String,
    #[serde(default)]
    pub angle_snap: Option<BoolField>,
    #[serde(default)]
    pub lift_off: Option<EnumField>,
    #[serde(default)]
    pub scroll: Option<ScalarField>,
    #[serde(default)]
    pub scroll_page: Option<BoolField>,
    #[serde(default)]
    pub accel: Option<BoolField>,
    #[serde(default)]
    pub double_click: Option<ScalarField>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoolField {
    pub window_offset: u16,
    pub addr: u16,
    pub len: u8,
    pub offset: u16,
    pub on: u8,
    pub off: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnumField {
    pub window_offset: u16,
    pub addr: u16,
    pub len: u8,
    pub offset: u16,
    pub values: Vec<LabeledCode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LabeledCode {
    pub id: String,
    pub label: String,
    pub code: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScalarField {
    pub window_offset: u16,
    pub addr: u16,
    pub len: u8,
    pub offset: u16,
    pub min: u16,
    pub max: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Tables {
    #[serde(default)]
    pub dpi_codes: Vec<DpiCode>,
    #[serde(default)]
    pub keys: Vec<KeyCode>,
    #[serde(default)]
    pub actions: Vec<ActionCode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DpiCode {
    pub dpi: u16,
    pub code: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyCode {
    pub id: String,
    pub label: String,
    pub usage: u16,
    #[serde(default)]
    pub modifiers: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActionKind {
    Default,
    Disabled,
    Mouse,
    Key,
    Multimedia,
    Macro,
    Profile,
    Dpi,
    Sniper,
    Burst,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionCode {
    pub id: String,
    pub label: String,
    pub code: u8,
    pub kind: ActionKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Descriptor {
    pub schema: u32,
    pub id: String,
    pub model: String,
    pub provenance: Provenance,
    pub verified: bool,
    #[serde(default)]
    pub notes: Option<String>,
    pub collections: Collections,
    pub frames: Vec<FrameSpec>,
    pub profiles: Profiles,
    #[serde(default)]
    pub read: ReadDefaults,
    pub commands: Commands,
    pub windows: BTreeMap<String, Window>,
    #[serde(default)]
    pub dpi: Option<DpiSpec>,
    #[serde(default)]
    pub polling: Option<PollingSpec>,
    #[serde(default)]
    pub lighting: Option<LightingSpec>,
    #[serde(default)]
    pub buttons: Option<ButtonsSpec>,
    #[serde(default)]
    pub macros: Option<MacrosSpec>,
    #[serde(default)]
    pub device: Option<DeviceSpec>,
    #[serde(default)]
    pub tables: Tables,
}

/// One region a descriptor will write to. Used for overlap validation and the
/// change-set summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Footprint {
    pub section: &'static str,
    pub profile: Option<u8>,
    pub report: u8,
    pub addr: u16,
    pub len: u16,
}

impl Descriptor {
    pub fn frame_sizes(&self) -> FrameSizes {
        FrameSizes::new(
            self.frames
                .iter()
                .map(|frame| (frame.id, frame.len as usize)),
        )
    }

    pub fn profile_count(&self) -> u8 {
        self.profiles.count
    }

    pub fn profile_indices(&self) -> std::ops::Range<u8> {
        0..self.profiles.count
    }

    /// All read windows expanded to concrete per-profile requests.
    pub fn window_instances(&self) -> Result<Vec<WindowInstance>, String> {
        let mut out = Vec::new();
        for (id, window) in &self.windows {
            out.extend(window.expand(id)?);
        }
        Ok(out)
    }

    pub fn window_instances_for(&self, id: &str) -> Result<Vec<WindowInstance>, String> {
        let window = self
            .windows
            .get(id)
            .ok_or_else(|| format!("descriptor references unknown window `{id}`"))?;
        window.expand(id)
    }

    pub fn supports_lighting(&self) -> bool {
        self.lighting.is_some()
    }

    pub fn supports_rgb(&self) -> bool {
        self.lighting
            .as_ref()
            .map(|l| l.modes.iter().any(|m| m.uses_color))
            .unwrap_or(false)
    }

    pub fn supports_dpi(&self) -> bool {
        self.dpi.is_some()
    }

    pub fn supports_polling(&self) -> bool {
        self.polling.is_some()
    }

    pub fn supports_buttons_write(&self) -> bool {
        self.buttons
            .as_ref()
            .map(|b| !b.write_addrs.is_empty())
            .unwrap_or(false)
    }

    pub fn supports_macros(&self) -> bool {
        self.macros.is_some()
    }

    pub fn supports_burst(&self) -> bool {
        self.macros
            .as_ref()
            .and_then(|m| m.burst.as_ref())
            .is_some()
    }

    pub fn supports_combos(&self) -> bool {
        self.buttons
            .as_ref()
            .map(|b| b.combo_slots > 0 && b.combo_addr.is_some() && b.combo_window.is_some())
            .unwrap_or(false)
    }

    pub fn supports_device_settings(&self) -> bool {
        self.device.is_some()
    }

    /// Capability labels for the device-information screen.
    pub fn capability_list(&self) -> Vec<&'static str> {
        let mut caps = Vec::new();
        if self.dpi.is_some() {
            caps.push("DPI stages");
        }
        if self.polling.is_some() {
            caps.push("polling rate");
        }
        if self.lighting.is_some() {
            caps.push("lighting");
        }
        if self.buttons.is_some() {
            caps.push(if self.supports_buttons_write() {
                "button remapping"
            } else {
                "button mappings (read-only)"
            });
        }
        if self.macros.is_some() {
            caps.push("macros");
        }
        if self.supports_burst() {
            caps.push("burst fire");
        }
        if self.supports_combos() {
            caps.push("multi-key combos");
        }
        let device = self.device.as_ref();
        if device.and_then(|d| d.angle_snap.as_ref()).is_some() {
            caps.push("angle snap");
        }
        if device.and_then(|d| d.lift_off.as_ref()).is_some() {
            caps.push("lift-off distance");
        }
        if device.and_then(|d| d.scroll.as_ref()).is_some() {
            caps.push("scroll speed");
        }
        if device.and_then(|d| d.double_click.as_ref()).is_some() {
            caps.push("double-click speed");
        }
        caps
    }

    /// Every write region implied by the declared sections.
    pub fn write_footprint(&self) -> Vec<Footprint> {
        let mut out = Vec::new();
        if let Some(dpi) = &self.dpi {
            for page in self.window_instances_for(&dpi.window).unwrap_or_default() {
                out.push(Footprint {
                    section: "dpi.active",
                    profile: Some(page.profile),
                    report: dpi.write_report,
                    addr: dpi.active_addr_at(page.addr),
                    len: dpi.active_len as u16,
                });
                for stage in 0..dpi.stage_count {
                    out.push(Footprint {
                        section: "dpi.stage",
                        profile: Some(page.profile),
                        report: dpi.write_report,
                        addr: dpi.stage_addr_at(page.addr, stage),
                        len: dpi.stage_len as u16,
                    });
                }
            }
        }
        if let Some(polling) = &self.polling {
            for (profile, addr) in polling.write_addrs.iter().enumerate() {
                out.push(Footprint {
                    section: "polling",
                    profile: Some(profile as u8),
                    report: 2,
                    addr: *addr,
                    len: polling.write_len as u16,
                });
            }
        }
        if let Some(lighting) = &self.lighting {
            for (profile, addr) in lighting.write_addrs.iter().enumerate() {
                out.push(Footprint {
                    section: "lighting",
                    profile: Some(profile as u8),
                    report: 2,
                    addr: *addr,
                    len: lighting.record_len(),
                });
            }
            for zone in &lighting.zones {
                out.push(Footprint {
                    section: "lighting.zone",
                    profile: None,
                    report: 2,
                    addr: zone.write_addr,
                    len: 3,
                });
            }
        }
        if let Some(buttons) = &self.buttons {
            for (profile, addr) in buttons.write_addrs.iter().enumerate() {
                for button in 0..buttons.count {
                    out.push(Footprint {
                        section: "buttons",
                        profile: Some(profile as u8),
                        report: buttons.write_report,
                        addr: addr.wrapping_add(buttons.entry_stride * button as u16),
                        len: buttons.entry_len as u16,
                    });
                }
            }
            if let (Some(combo_addr), true) = (buttons.combo_addr, buttons.combo_slots > 0) {
                for profile in self.profile_indices() {
                    for button in 0..buttons.count {
                        for slot in 0..buttons.combo_slots {
                            out.push(Footprint {
                                section: "buttons.combo",
                                profile: Some(profile),
                                report: buttons.write_report,
                                addr: combo_addr
                                    .wrapping_add(buttons.combo_profile_stride * profile as u16)
                                    .wrapping_add(buttons.combo_stride * button as u16)
                                    .wrapping_add(2 * slot as u16),
                                len: 2,
                            });
                        }
                    }
                }
            }
        }
        if let Some(macros) = &self.macros {
            for slot in 0..macros.slots {
                out.push(Footprint {
                    section: "macros",
                    profile: None,
                    report: macros.report,
                    addr: macros.slot_addr(slot),
                    len: macros.record_len,
                });
            }
            if let Some(burst) = &macros.burst {
                for button in 0..self.buttons.as_ref().map(|b| b.count).unwrap_or(0) {
                    out.push(Footprint {
                        section: "macros.burst",
                        profile: None,
                        report: 2,
                        addr: burst.addr.wrapping_add(burst.stride * button as u16),
                        len: burst.len as u16,
                    });
                }
            }
        }
        if let Some(device) = &self.device {
            let mut push = |section: &'static str, addr: u16, len: u8| {
                out.push(Footprint {
                    section,
                    profile: None,
                    report: 2,
                    addr,
                    len: len as u16,
                });
            };
            if let Some(f) = &device.angle_snap {
                push("device.angle", f.addr.wrapping_add(f.offset), f.len);
            }
            if let Some(f) = &device.scroll_page {
                push("device.scroll_page", f.addr.wrapping_add(f.offset), f.len);
            }
            if let Some(f) = &device.accel {
                push("device.accel", f.addr.wrapping_add(f.offset), f.len);
            }
            if let Some(f) = &device.lift_off {
                push("device.liftoff", f.addr.wrapping_add(f.offset), f.len);
            }
            if let Some(f) = &device.scroll {
                push("device.scroll", f.addr.wrapping_add(f.offset), f.len);
            }
            if let Some(f) = &device.double_click {
                push("device.double_click", f.addr.wrapping_add(f.offset), f.len);
            }
        }
        out
    }

    /// Parse and check a descriptor from TOML text.
    pub fn from_toml(text: &str) -> Result<Self, DescriptorError> {
        let desc: Descriptor = toml::from_str(text)?;
        if desc.schema != SUPPORTED_SCHEMA {
            return Err(DescriptorError::Schema {
                found: desc.schema,
                supported: SUPPORTED_SCHEMA,
            });
        }
        let errors = super::validate::validate(&desc);
        if !errors.is_empty() {
            return Err(DescriptorError::Invalid(errors));
        }
        Ok(desc)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DescriptorError {
    #[error("parse error: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("unsupported schema version {found}; this build supports {supported}")]
    Schema { found: u32, supported: u32 },
    #[error("descriptor is invalid: {}", .0.join(" | "))]
    Invalid(Vec<String>),
}
