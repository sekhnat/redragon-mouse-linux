//! Plain-language description of pending edits, for the apply dialog.
//!
//! The register-level diff (`settings::diff`) stays available for logs and the
//! details view; this is what a person reads before confirming a write.

use crate::models::Descriptor;
use crate::settings::state::{DeviceState, DpiStage, ProfileLighting};

fn dpi_text(stage: &DpiStage) -> String {
    if stage.x == 0 {
        format!("code {:#06x}", stage.raw)
    } else {
        format!("{} DPI", stage.x)
    }
}

fn hex(color: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2])
}

fn mode_label(desc: &Descriptor, lighting: &ProfileLighting) -> String {
    let spec = desc.lighting.as_ref();
    let by_id = spec
        .filter(|_| !lighting.mode.is_empty())
        .and_then(|spec| spec.mode(&lighting.mode));
    match by_id.or_else(|| spec.and_then(|spec| spec.mode_by_code(lighting.mode_code))) {
        Some(mode) => mode.label.clone(),
        None => format!("code {:#06x}", lighting.mode_code),
    }
}

fn lighting_parts(
    desc: &Descriptor,
    before: &ProfileLighting,
    after: &ProfileLighting,
) -> Vec<String> {
    let mut parts = Vec::new();
    let (old_mode, new_mode) = (mode_label(desc, before), mode_label(desc, after));
    if old_mode != new_mode {
        parts.push(format!("{old_mode} → {new_mode}"));
    }
    if before.color != after.color {
        parts.push(format!(
            "colour {} → {}",
            hex(before.color),
            hex(after.color)
        ));
    }
    if before.speed != after.speed {
        match desc.lighting.as_ref() {
            Some(spec) => parts.push(format!(
                "speed {} → {}",
                spec.speed_level(before.speed),
                spec.speed_level(after.speed)
            )),
            None => parts.push(format!("speed {} → {}", before.speed, after.speed)),
        }
    }
    if before.brightness != after.brightness {
        parts.push(format!(
            "brightness {} → {}",
            before.brightness, after.brightness
        ));
    }
    parts
}

/// One line per edit, ordered by profile. Empty when nothing differs.
pub fn describe_changes(
    desc: &Descriptor,
    baseline: &DeviceState,
    draft: &DeviceState,
) -> Vec<String> {
    let mut lines = Vec::new();
    for (index, (before, after)) in baseline.profiles.iter().zip(&draft.profiles).enumerate() {
        let tag = format!("Profile {}", index + 1);
        if let (Some(old), Some(new)) = (&before.dpi, &after.dpi) {
            for (stage, (old, new)) in old.stages.iter().zip(&new.stages).enumerate() {
                if old.x != new.x || old.y != new.y {
                    lines.push(format!(
                        "{tag} · DPI stage {}: {} → {}",
                        stage + 1,
                        dpi_text(old),
                        dpi_text(new)
                    ));
                }
                if old.enabled != new.enabled {
                    lines.push(format!(
                        "{tag} · DPI stage {} {}",
                        stage + 1,
                        if new.enabled { "enabled" } else { "disabled" }
                    ));
                }
            }
            if old.active != new.active {
                lines.push(format!(
                    "{tag} · Active DPI stage: {} → {}",
                    old.active + 1,
                    new.active + 1
                ));
            }
        }
        if before.polling_hz != after.polling_hz {
            let show = |hz: Option<u16>| hz.map_or("unknown".to_string(), |hz| format!("{hz} Hz"));
            lines.push(format!(
                "{tag} · Polling rate: {} → {}",
                show(before.polling_hz),
                show(after.polling_hz)
            ));
        }
        if let (Some(old), Some(new)) = (&before.lighting, &after.lighting) {
            let parts = lighting_parts(desc, old, new);
            if !parts.is_empty() {
                lines.push(format!("{tag} · Lighting: {}", parts.join(", ")));
            }
        }
        if before.buttons != after.buttons {
            lines.push(format!("{tag} · Button mappings changed"));
        }
    }
    if baseline.device != draft.device {
        lines.push("Device-wide pointer settings changed".into());
    }
    if baseline.macros != draft.macros {
        lines.push("Macros changed".into());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::state::seed_state;

    fn desc() -> Descriptor {
        Descriptor::from_toml(include_str!("../../models/holtek-04d9-fc5c.toml"))
            .expect("descriptor")
    }

    #[test]
    fn no_edits_means_no_lines() {
        let desc = desc();
        let state = seed_state(&desc);
        assert!(describe_changes(&desc, &state, &state.clone()).is_empty());
    }

    #[test]
    fn edits_read_as_sentences() {
        let desc = desc();
        let baseline = seed_state(&desc);
        let mut draft = baseline.clone();
        {
            let led = draft.profiles[0].lighting.as_mut().unwrap();
            led.set_mode(desc.lighting.as_ref().unwrap(), "wave");
            led.color = [0, 255, 0];
            led.brightness = 1;
        }
        draft.profiles[0].dpi.as_mut().unwrap().stages[1].x = 1600;
        draft.profiles[0].dpi.as_mut().unwrap().active = 2;
        draft.profiles[1].polling_hz = Some(1000);
        let lines = describe_changes(&desc, &baseline, &draft);
        assert!(
            lines.iter().any(|l| l.contains("Breathing → Wave")),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.contains("colour #ffffff → #00ff00")),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l.contains("DPI stage 2: 200 DPI → 1600 DPI")),
            "{lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.contains("Active DPI stage: 1 → 3")),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l.contains("Profile 2 · Polling rate: 500 Hz → 1000 Hz")),
            "{lines:?}"
        );
    }

    #[test]
    fn speed_levels_round_trip_through_the_register() {
        let desc = desc();
        let spec = desc.lighting.as_ref().unwrap();
        for level in 1..=spec.speed_steps() {
            assert_eq!(spec.speed_level(spec.speed_raw(level)), level);
        }
        // Inverted: slowest is the largest register value, fastest the smallest non-zero.
        assert_eq!(spec.speed_raw(1), 8);
        assert_eq!(spec.speed_raw(8), 1);
    }
}
