//! Scripted edits: `key=value` assignments applied to a draft state.
//!
//! Used by the `--set` command line so every feature can be exercised without
//! the GUI. Profiles and stages are 1-based, matching what the GUI shows.
//!
//! ```text
//! led.1.mode=breathing    led.1.color=00ff00    led.1.speed=3    led.1.brightness=2
//! dpi.1.2=1600            dpi.1.2.on=false      dpi.1.active=3
//! poll.1=1000             profile=2
//! ```

use crate::models::Descriptor;

use super::state::DeviceState;

/// What an assignment list asks for beyond edits to the draft state.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Assigned {
    /// 0-based profile to activate on the mouse, from a `profile=N` item.
    pub activate: Option<u8>,
}

pub fn apply_assignments(
    desc: &Descriptor,
    draft: &mut DeviceState,
    items: &[String],
) -> Result<Assigned, String> {
    let mut assigned = Assigned::default();
    for item in items {
        let (key, value) = item
            .split_once('=')
            .ok_or_else(|| format!("`{item}` is not key=value"))?;
        let parts: Vec<&str> = key.split('.').collect();
        match parts.as_slice() {
            ["profile"] => {
                let index = profile_index(desc, value)?;
                assigned.activate = Some(index);
            }
            ["poll", profile] => {
                let index = profile_index(desc, profile)?;
                let hz: u16 = value
                    .parse()
                    .map_err(|_| format!("`{value}` is not a polling rate in Hz"))?;
                draft.profiles[index as usize].polling_hz = Some(hz);
            }
            ["led", profile, field] => {
                let index = profile_index(desc, profile)?;
                let spec = desc.lighting.as_ref().ok_or("this model has no lighting")?;
                let lighting = draft.profiles[index as usize]
                    .lighting
                    .as_mut()
                    .ok_or_else(|| format!("no lighting data was read for profile {profile}"))?;
                match *field {
                    "mode" => {
                        if !lighting.set_mode(spec, value) {
                            let known: Vec<&str> =
                                spec.modes.iter().map(|m| m.id.as_str()).collect();
                            return Err(format!(
                                "`{value}` is not a lighting mode (known: {})",
                                known.join(", ")
                            ));
                        }
                    }
                    "color" => lighting.color = parse_color(value)?,
                    "speed" => lighting.speed = parse_u8(value, "speed")?,
                    "brightness" => lighting.brightness = parse_u8(value, "brightness")?,
                    other => return Err(format!("unknown lighting field `{other}`")),
                }
            }
            ["dpi", profile, "active"] => {
                let index = profile_index(desc, profile)?;
                let stage = stage_index(draft, index, value)?;
                draft.profiles[index as usize]
                    .dpi
                    .as_mut()
                    .ok_or("this model has no DPI stages")?
                    .active = stage;
            }
            ["dpi", profile, stage] => {
                let index = profile_index(desc, profile)?;
                let stage = stage_index(draft, index, stage)?;
                let dpi = draft.profiles[index as usize]
                    .dpi
                    .as_mut()
                    .ok_or("this model has no DPI stages")?;
                let value: u16 = value
                    .parse()
                    .map_err(|_| format!("`{value}` is not a DPI value"))?;
                dpi.stages[stage as usize].x = value;
                dpi.stages[stage as usize].y = value;
            }
            ["dpi", profile, stage, "on"] => {
                let index = profile_index(desc, profile)?;
                let stage = stage_index(draft, index, stage)?;
                let on = match value {
                    "true" | "on" | "1" => true,
                    "false" | "off" | "0" => false,
                    other => return Err(format!("`{other}` is not true/false")),
                };
                draft.profiles[index as usize]
                    .dpi
                    .as_mut()
                    .ok_or("this model has no DPI stages")?
                    .stages[stage as usize]
                    .enabled = on;
            }
            _ => return Err(format!("unknown setting `{key}`")),
        }
    }
    Ok(assigned)
}

fn profile_index(desc: &Descriptor, raw: &str) -> Result<u8, String> {
    let number: u8 = raw
        .parse()
        .map_err(|_| format!("`{raw}` is not a profile number"))?;
    if number == 0 || number > desc.profile_count() {
        return Err(format!(
            "profile {number} is outside 1..={}",
            desc.profile_count()
        ));
    }
    Ok(number - 1)
}

fn stage_index(draft: &DeviceState, profile: u8, raw: &str) -> Result<u8, String> {
    let number: u8 = raw
        .parse()
        .map_err(|_| format!("`{raw}` is not a DPI stage number"))?;
    let count = draft.profiles[profile as usize]
        .dpi
        .as_ref()
        .map(|dpi| dpi.stages.len())
        .unwrap_or(0);
    if number == 0 || number as usize > count {
        return Err(format!("DPI stage {number} is outside 1..={count}"));
    }
    Ok(number - 1)
}

fn parse_u8(raw: &str, what: &str) -> Result<u8, String> {
    raw.parse()
        .map_err(|_| format!("`{raw}` is not a valid {what}"))
}

fn parse_color(raw: &str) -> Result<[u8; 3], String> {
    let hex = raw.trim().trim_start_matches('#');
    if hex.len() != 6 {
        return Err(format!("`{raw}` is not a 6-digit RGB value"));
    }
    let channel = |at: usize| {
        u8::from_str_radix(&hex[at..at + 2], 16)
            .map_err(|_| format!("`{raw}` is not a 6-digit RGB value"))
    };
    Ok([channel(0)?, channel(2)?, channel(4)?])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::state::seed_state;

    fn desc() -> Descriptor {
        Descriptor::from_toml(include_str!("../../models/holtek-04d9-fc5c.toml"))
            .expect("descriptor")
    }

    fn items(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn led_assignments_edit_the_draft() {
        let desc = desc();
        let mut draft = seed_state(&desc);
        let assigned = apply_assignments(
            &desc,
            &mut draft,
            &items(&[
                "led.2.mode=steady",
                "led.2.color=00ff00",
                "led.2.brightness=3",
            ]),
        )
        .expect("assign");
        assert_eq!(assigned, Assigned::default());
        let led = draft.profiles[1].lighting.as_ref().unwrap();
        assert_eq!(led.mode, "steady");
        assert_eq!(led.mode_code, 0x0102);
        assert_eq!(led.color, [0, 255, 0]);
        assert_eq!(led.brightness, 3);
        assert_eq!(led.speed, 0, "steady has no speed");
    }

    #[test]
    fn dpi_polling_and_profile_assignments() {
        let desc = desc();
        let mut draft = seed_state(&desc);
        let assigned = apply_assignments(
            &desc,
            &mut draft,
            &items(&["dpi.1.2=1600", "dpi.1.active=3", "poll.1=1000", "profile=2"]),
        )
        .expect("assign");
        assert_eq!(assigned.activate, Some(1));
        let profile = &draft.profiles[0];
        assert_eq!(profile.dpi.as_ref().unwrap().stages[1].x, 1600);
        assert_eq!(profile.dpi.as_ref().unwrap().active, 2);
        assert_eq!(profile.polling_hz, Some(1000));
    }

    #[test]
    fn bad_assignments_are_explained() {
        let desc = desc();
        let mut draft = seed_state(&desc);
        for bad in [
            "led.9.mode=steady",
            "led.1.mode=disco",
            "dpi.1.9=800",
            "nonsense",
            "x.y=1",
        ] {
            assert!(
                apply_assignments(&desc, &mut draft, &items(&[bad])).is_err(),
                "{bad}"
            );
        }
    }
}
