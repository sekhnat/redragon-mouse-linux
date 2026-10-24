//! Import and export of per-profile INI files in the vendor's field layout.
//!
//! Field names follow the shipped `*.prof` files. Values that the vendor
//! application stores as its own enumerations (button function numbers, key
//! map bytes) are written as our descriptor's action/key codes; the file stays
//! self-consistent for round-tripping and stays readable for the fields both
//! applications share.

use std::collections::BTreeMap;

use crate::models::Descriptor;

use super::state::{
    code_for_dpi, usage_for_key, DpiStage, FieldError, ProfileDpi, ProfileLighting, ProfileState,
};

pub const SECTION: &str = "PROFILE";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportResult {
    pub profile: ProfileState,
    /// The base device settings with any imported global values applied.
    pub device: super::state::DeviceSettings,
    pub warnings: Vec<String>,
}

/// Serialise a profile using the vendor field names, CRLF line endings.
pub fn export_profile(
    desc: &Descriptor,
    profile: &ProfileState,
    device: &super::state::DeviceSettings,
) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("[{SECTION}]"));

    if let Some(hz) = profile.polling_hz {
        lines.push(format!("pollrate={hz}"));
    }
    if let Some(dpi) = &profile.dpi {
        lines.push(format!("dpi={}", dpi.active));
        for (index, stage) in dpi.stages.iter().enumerate() {
            let linked = dpi.linked.get(index).copied().unwrap_or(true);
            lines.push(format!("DPI_INFO_{index}_used={}", stage.enabled as u8));
            lines.push(format!("DPI_INFO_{index}_link={}", linked as u8));
            lines.push(format!("DPI_INFO_{index}_xy={}", stage.x));
            lines.push(format!("DPI_INFO_{index}_x={}", stage.x));
            lines.push(format!("DPI_INFO_{index}_y={}", stage.y));
        }
    }
    if let Some(value) = device.scroll {
        lines.push(format!("scrollspeed={value}"));
    }
    if let Some(value) = device.scroll_page {
        lines.push(format!("scrollispage={}", value as u8));
    }
    if let Some(value) = device.accel {
        lines.push(format!("accelenable={}", value as u8));
        lines.push(format!("acee={}", value as u8));
    }
    if let Some(value) = device.double_click {
        lines.push(format!("dbclickspeed={value}"));
    }
    if let Some(lighting) = &profile.lighting {
        lines.push(format!("lightmode={}", lighting.mode));
        lines.push(format!(
            "color={:02x}{:02x}{:02x}",
            lighting.color[0], lighting.color[1], lighting.color[2]
        ));
        lines.push(format!("brightness={}", lighting.brightness));
        lines.push(format!("speed={}", lighting.speed));
    }
    if let Some(buttons) = &profile.buttons {
        for (index, binding) in buttons.iter().enumerate() {
            let code = desc
                .tables
                .actions
                .iter()
                .find(|action| action.id == binding.action)
                .map(|action| action.code)
                .unwrap_or(binding.raw[0]);
            lines.push(format!("BUTTON_INFO_{index}_ID={index}"));
            lines.push(format!("BUTTON_INFO_{index}_Fun={code}"));
            lines.push(format!("BUTTON_INFO_{index}_LoopNumber=0"));
            lines.push(format!(
                "BUTTON_INFO_{index}_KeyNumber={}",
                binding.combo.len().max(binding.key.iter().count())
            ));
            lines.push(format!("BUTTON_INFO_{index}_Res1=0"));
            lines.push(format!("BUTTON_INFO_{index}_Res2=0"));
            lines.push(format!("BUTTON_INFO_{index}_Res3=0"));
            lines.push(format!("BUTTON_INFO_{index}_Res4=0"));
            lines.push(format!(
                "BUTTON_INFO_{index}_macroname={}",
                binding
                    .macro_slot
                    .map(|s| s.to_string())
                    .unwrap_or_default()
            ));
            let mut slot = 0usize;
            if let Some(key) = &binding.key {
                if let Some(usage) = usage_for_key(desc, key) {
                    write_key_slot(&mut lines, index, slot, usage);
                    slot += 1;
                }
            }
            for key in &binding.combo {
                if let Some(usage) = usage_for_key(desc, key) {
                    write_key_slot(&mut lines, index, slot, usage);
                    slot += 1;
                }
            }
        }
    }
    let mut out = lines.join("\r\n");
    out.push_str("\r\n");
    out
}

fn write_key_slot(lines: &mut Vec<String>, button: usize, slot: usize, usage: u16) {
    lines.push(format!("BUTTON_INFO_{button}_KeyInfo_{slot}_map=0"));
    lines.push(format!(
        "BUTTON_INFO_{button}_KeyInfo_{slot}_={}",
        usage & 0xFF
    ));
    lines.push(format!(
        "BUTTON_INFO_{button}_KeyInfo_{slot}_L={}",
        usage & 0xFF
    ));
    lines.push(format!(
        "BUTTON_INFO_{button}_KeyInfo_{slot}_H={}",
        usage >> 8
    ));
}

/// Parse a profile file. The whole file is rejected when a value cannot be
/// expressed by the active model; unrecognised keys are reported as warnings.
pub fn import_profile(
    desc: &Descriptor,
    text: &str,
    base: &ProfileState,
    base_device: &super::state::DeviceSettings,
) -> Result<ImportResult, Vec<FieldError>> {
    let mut values: BTreeMap<String, String> = BTreeMap::new();
    let mut warnings = Vec::new();
    let mut section = String::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim().to_ascii_uppercase();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().to_string();
        let value = value.trim().to_string();
        if section.is_empty() || section == SECTION {
            values.insert(key, value);
        } else {
            warnings.push(format!("ignored key `{key}` from section [{section}]"));
        }
    }

    let mut errors = Vec::new();
    let mut profile = base.clone();
    let mut device = base_device.clone();

    if let Some(raw) = values.get("pollrate") {
        match parse_rate(raw) {
            Some(hz)
                if desc
                    .polling
                    .as_ref()
                    .map(|p| p.codes.iter().any(|c| c.hz == hz))
                    .unwrap_or(false) =>
            {
                profile.polling_hz = Some(hz);
            }
            Some(hz) => errors.push(FieldError {
                field: "pollrate".into(),
                message: format!("{hz} Hz is not supported by this model"),
            }),
            None => errors.push(FieldError {
                field: "pollrate".into(),
                message: format!("`{raw}` is not a polling rate"),
            }),
        }
    }

    if let Some(dpi) = &desc.dpi {
        let mut state = profile.dpi.clone().unwrap_or_else(|| ProfileDpi {
            stages: vec![
                DpiStage {
                    enabled: true,
                    x: 0,
                    y: 0,
                    raw: 0,
                };
                dpi.stage_count as usize
            ],
            linked: vec![true; dpi.stage_count as usize],
            active: 0,
        });
        if let Some(raw) = values.get("dpi") {
            match raw.parse::<u8>() {
                Ok(active) if active < dpi.stage_count => state.active = active,
                _ => errors.push(FieldError {
                    field: "dpi".into(),
                    message: format!(
                        "`{raw}` is not a DPI stage index (0..{})",
                        dpi.stage_count - 1
                    ),
                }),
            }
        }
        for stage in 0..dpi.stage_count {
            let stage_state = &mut state.stages[stage as usize];
            let read = |name: &str| values.get(&format!("DPI_INFO_{stage}_{name}")).cloned();
            if let Some(raw) = read("used") {
                match parse_bool(&raw) {
                    Some(value) => stage_state.enabled = value,
                    None => errors.push(FieldError {
                        field: format!("DPI_INFO_{stage}_used"),
                        message: format!("`{raw}` is not 0 or 1"),
                    }),
                }
            }
            if let Some(raw) = read("link") {
                if let Some(value) = parse_bool(&raw) {
                    if let Some(entry) = state.linked.get_mut(stage as usize) {
                        *entry = value;
                    }
                }
            }
            for (name, target) in [("x", true), ("y", false)] {
                let Some(raw) = read(name) else { continue };
                let Ok(dpi_value) = raw.parse::<u16>() else {
                    errors.push(FieldError {
                        field: format!("DPI_INFO_{stage}_{name}"),
                        message: format!("`{raw}` is not a DPI value"),
                    });
                    continue;
                };
                if dpi_value == 0 {
                    continue;
                }
                if code_for_dpi(desc, dpi_value).is_none() {
                    errors.push(FieldError {
                        field: format!("DPI_INFO_{stage}_{name}"),
                        message: format!("{dpi_value} DPI is not in this model's code table"),
                    });
                    continue;
                }
                if target {
                    stage_state.x = dpi_value;
                } else {
                    stage_state.y = dpi_value;
                }
            }
            if stage_state.y == 0 {
                stage_state.y = stage_state.x;
            }
        }
        profile.dpi = Some(state);
    }

    if let Some(spec) = &desc.device {
        if let Some(field) = &spec.scroll {
            if let Some(raw) = values.get("scrollspeed") {
                match raw.parse::<u16>() {
                    Ok(value) if value >= field.min && value <= field.max => {
                        device.scroll = Some(value)
                    }
                    _ => errors.push(FieldError {
                        field: "scrollspeed".into(),
                        message: format!(
                            "`{raw}` is outside the declared range {}..{}",
                            field.min, field.max
                        ),
                    }),
                }
            }
        }
        if let Some(field) = &spec.scroll_page {
            if let Some(raw) = values.get("scrollispage") {
                match parse_bool(raw) {
                    Some(value) => {
                        let _ = field;
                        device.scroll_page = Some(value)
                    }
                    None => errors.push(FieldError {
                        field: "scrollispage".into(),
                        message: format!("`{raw}` is not 0 or 1"),
                    }),
                }
            }
        }
        if let Some(field) = &spec.accel {
            if let Some(raw) = values.get("accelenable").or_else(|| values.get("acee")) {
                match parse_bool(raw) {
                    Some(value) => {
                        let _ = field;
                        device.accel = Some(value)
                    }
                    None => errors.push(FieldError {
                        field: "accelenable".into(),
                        message: format!("`{raw}` is not 0 or 1"),
                    }),
                }
            }
        }
        if let Some(field) = &spec.double_click {
            if let Some(raw) = values.get("dbclickspeed") {
                match raw.parse::<u16>() {
                    Ok(value) if value >= field.min && value <= field.max => {
                        device.double_click = Some(value)
                    }
                    _ => errors.push(FieldError {
                        field: "dbclickspeed".into(),
                        message: format!(
                            "`{raw}` is outside the declared range {}..{}",
                            field.min, field.max
                        ),
                    }),
                }
            }
        }
        if let Some(field) = &spec.lift_off {
            if let Some(raw) = values.get("lifthigh").or_else(|| values.get("lift_off")) {
                match field
                    .values
                    .iter()
                    .find(|v| v.id == *raw || v.code.to_string() == *raw)
                {
                    Some(value) => device.lift_off = Some(value.id.clone()),
                    None => errors.push(FieldError {
                        field: "lifthigh".into(),
                        message: format!("`{raw}` is not a lift-off value this model declares"),
                    }),
                }
            }
        }
    }

    if let Some(lighting) = &desc.lighting {
        let mut state = profile.lighting.clone().unwrap_or_else(|| ProfileLighting {
            mode: String::new(),
            mode_code: 0,
            color: [0, 0, 0],
            speed: 0,
            brightness: 0,
            zone_colors: Vec::new(),
        });
        if let Some(raw) = values.get("lightmode") {
            if let Some(mode) = lighting.mode(raw) {
                state.mode = mode.id.clone();
                state.mode_code = mode.code;
            } else if let Ok(code) = u8::from_str_radix(raw.trim_start_matches("0x"), 16) {
                state.mode.clear();
                state.mode_code = code;
            } else {
                errors.push(FieldError {
                    field: "lightmode".into(),
                    message: format!("`{raw}` is not a lighting mode"),
                });
            }
        }
        if let Some(raw) = values.get("color") {
            match parse_color(raw) {
                Some(color) => state.color = color,
                None => errors.push(FieldError {
                    field: "color".into(),
                    message: format!("`{raw}` is not a 6-digit RGB value"),
                }),
            }
        }
        if let Some(raw) = values.get("brightness") {
            match raw.parse::<u8>() {
                Ok(value) if value < lighting.brightness_levels => state.brightness = value,
                _ => errors.push(FieldError {
                    field: "brightness".into(),
                    message: format!(
                        "`{raw}` is above the {} levels this model declares",
                        lighting.brightness_levels
                    ),
                }),
            }
        }
        if let Some(raw) = values.get("speed") {
            match raw.parse::<u8>() {
                Ok(value) if value < lighting.speed_levels => state.speed = value,
                _ => errors.push(FieldError {
                    field: "speed".into(),
                    message: format!(
                        "`{raw}` is above the {} levels this model declares",
                        lighting.speed_levels
                    ),
                }),
            }
        }
        profile.lighting = Some(state);
    }

    if let Some(buttons_spec) = &desc.buttons {
        let mut bindings = profile.buttons.clone().unwrap_or_else(|| {
            vec![
                super::state::ButtonBinding {
                    action: "default".into(),
                    key: None,
                    combo: Vec::new(),
                    macro_slot: None,
                    profile_target: None,
                    raw: [0; 4],
                };
                buttons_spec.count as usize
            ]
        });
        for (index, binding) in bindings
            .iter_mut()
            .enumerate()
            .take(buttons_spec.count as usize)
        {
            let prefix = format!("BUTTON_INFO_{index}_");
            if let Some(raw) = values.get(&format!("{prefix}Fun")) {
                match raw.parse::<u8>() {
                    Ok(code) => match desc.tables.actions.iter().find(|a| a.code == code) {
                        Some(action) => binding.action = action.id.clone(),
                        None => errors.push(FieldError {
                            field: format!("{prefix}Fun"),
                            message: format!("{code} is not an action this model declares"),
                        }),
                    },
                    Err(_) => errors.push(FieldError {
                        field: format!("{prefix}Fun"),
                        message: format!("`{raw}` is not an action code"),
                    }),
                }
            }
            // The file is the source of truth for this button's keys.
            binding.key = None;
            binding.combo.clear();
            let mut slot = 0usize;
            while slot < buttons_spec.combo_slots as usize + 1 {
                let key_prefix = format!("{prefix}KeyInfo_{slot}_");
                let low = values
                    .get(&format!("{key_prefix}L"))
                    .or_else(|| values.get(&format!("{key_prefix}_")))
                    .cloned();
                let Some(low) = low else { break };
                let high = values
                    .get(&format!("{key_prefix}H"))
                    .cloned()
                    .unwrap_or_else(|| "0".into());
                let usage = low.parse::<u16>().ok().zip(high.parse::<u16>().ok());
                match usage {
                    Some((lo, hi)) => {
                        let usage = lo | (hi << 8);
                        if usage != 0 {
                            match desc.tables.keys.iter().find(|k| k.usage == usage) {
                                Some(key) => {
                                    if slot == 0 {
                                        binding.key = Some(key.id.clone());
                                    } else {
                                        binding.combo.push(key.id.clone());
                                    }
                                }
                                None => errors.push(FieldError {
                                    field: format!("{key_prefix}_"),
                                    message: format!(
                                        "usage {usage} is not in this model's key table"
                                    ),
                                }),
                            }
                        }
                    }
                    None => errors.push(FieldError {
                        field: format!("{key_prefix}_"),
                        message: "key usage bytes are not numbers".into(),
                    }),
                }
                slot += 1;
            }
            if let Some(raw) = values.get(&format!("{prefix}macroname")) {
                if !raw.is_empty() {
                    match raw.parse::<u8>() {
                        Ok(slot)
                            if desc
                                .macros
                                .as_ref()
                                .map(|m| slot < m.slots)
                                .unwrap_or(false) =>
                        {
                            binding.macro_slot = Some(slot);
                        }
                        _ => errors.push(FieldError {
                            field: format!("{prefix}macroname"),
                            message: format!("`{raw}` is not a macro slot"),
                        }),
                    }
                }
            }
        }
        profile.buttons = Some(bindings);
    }

    for key in values.keys() {
        if !is_known_key(key) {
            warnings.push(format!("unrecognised key `{key}` was ignored"));
        }
    }

    if errors.is_empty() {
        Ok(ImportResult {
            profile,
            device,
            warnings,
        })
    } else {
        Err(errors)
    }
}

fn is_known_key(key: &str) -> bool {
    matches!(
        key,
        "pollrate"
            | "dpi"
            | "scrollspeed"
            | "scrollispage"
            | "accelenable"
            | "acee"
            | "aceel"
            | "dbclickspeed"
            | "mousespeed"
            | "lightmode"
            | "color"
            | "brightness"
            | "speed"
            | "lifthigh"
            | "lift_off"
    ) || key.starts_with("DPI_INFO_")
        || key.starts_with("BUTTON_INFO_")
}

fn parse_rate(raw: &str) -> Option<u16> {
    let value = raw.parse::<u16>().ok()?;
    match value {
        0 => Some(125),
        1 => Some(250),
        2 => Some(500),
        3 => Some(1000),
        hz @ (125 | 250 | 500 | 1000) => Some(hz),
        _ => None,
    }
}

fn parse_bool(raw: &str) -> Option<bool> {
    match raw {
        "0" | "false" => Some(false),
        "1" | "true" => Some(true),
        _ => None,
    }
}

fn parse_color(raw: &str) -> Option<[u8; 3]> {
    let hex = raw.trim().trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some([r, g, b])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Descriptor;
    use crate::settings::state::{seed_state, ButtonBinding};

    fn fixture() -> Descriptor {
        crate::models::fixture::descriptor()
    }

    #[test]
    fn export_then_import_round_trips() {
        let desc = fixture();
        let state = seed_state(&desc);
        let mut profile = state.profiles[0].clone();
        profile.polling_hz = Some(1000);
        profile.dpi.as_mut().unwrap().active = 2;
        profile.buttons.as_mut().unwrap()[1] = ButtonBinding {
            action: "macro".into(),
            key: Some("c".into()),
            combo: vec!["ctrl".into(), "c".into()],
            macro_slot: Some(1),
            profile_target: None,
            raw: [0x30, 1, 0, 0],
        };
        let text = export_profile(&desc, &profile, &state.device);
        let imported = import_profile(&desc, &text, &profile, &state.device).expect("import");
        assert!(imported.warnings.is_empty(), "{:?}", imported.warnings);
        assert_eq!(imported.profile.polling_hz, Some(1000));
        assert_eq!(imported.profile.dpi.as_ref().unwrap().active, 2);
        let binding = &imported.profile.buttons.as_ref().unwrap()[1];
        assert_eq!(binding.action, "macro");
        assert_eq!(binding.combo, vec!["ctrl", "c"]);
        assert_eq!(binding.macro_slot, Some(1));
    }

    #[test]
    fn import_accepts_both_keyinfo_variants() {
        let desc = fixture();
        let state = seed_state(&desc);
        let base = state.profiles[0].clone();
        let text = "\r\n[PROFILE]\r\npollrate=2\r\nBUTTON_INFO_0_Fun=2\r\nBUTTON_INFO_0_KeyInfo_0_L=4\r\nBUTTON_INFO_0_KeyInfo_0_H=0\r\n";
        let imported = import_profile(&desc, text, &base, &state.device).expect("import");
        assert_eq!(imported.profile.polling_hz, Some(500));
        assert_eq!(
            imported.profile.buttons.as_ref().unwrap()[0].key.as_deref(),
            Some("a")
        );
    }

    #[test]
    fn unsupported_rate_rejects_the_whole_file() {
        let desc = fixture();
        let state = seed_state(&desc);
        let base = state.profiles[0].clone();
        let text = "[PROFILE]\npollrate=250\n";
        let errors =
            import_profile(&desc, text, &base, &state.device).expect_err("250 Hz not declared");
        assert!(errors.iter().any(|e| e.field == "pollrate"), "{errors:?}");
    }

    #[test]
    fn unknown_keys_are_warnings_not_errors() {
        let desc = fixture();
        let state = seed_state(&desc);
        let base = state.profiles[0].clone();
        let text = "[PROFILE]\npollrate=500\nsomething_else=7\n";
        let imported = import_profile(&desc, text, &base, &state.device).expect("import");
        assert_eq!(imported.profile.polling_hz, Some(500));
        assert!(imported
            .warnings
            .iter()
            .any(|w| w.contains("something_else")));
    }

    #[test]
    fn keyinfo_beyond_slots_is_ignored() {
        let desc = fixture();
        let state = seed_state(&desc);
        let base = state.profiles[0].clone();
        let text = "[PROFILE]\nBUTTON_INFO_0_KeyInfo_9_L=4\n";
        let imported = import_profile(&desc, text, &base, &state.device).expect("import");
        assert!(imported.profile.buttons.as_ref().unwrap()[0]
            .combo
            .is_empty());
    }
}
