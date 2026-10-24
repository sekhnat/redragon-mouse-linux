//! Lighting: effect, colour, brightness, speed, and zones.

use eframe::egui;

use super::app::App;
use crate::models::descriptor::LightingSpec;
use crate::settings::state::ProfileLighting;

/// Quick colour choices shown under the picker.
const PRESETS: [([u8; 3], &str); 9] = [
    ([255, 0, 0], "Red"),
    ([255, 128, 0], "Orange"),
    ([255, 220, 0], "Yellow"),
    ([0, 255, 0], "Green"),
    ([0, 255, 220], "Cyan"),
    ([0, 80, 255], "Blue"),
    ([150, 0, 255], "Purple"),
    ([255, 0, 160], "Pink"),
    ([255, 255, 255], "White"),
];

const EFFECT_COLUMNS: usize = 4;

impl App {
    pub(crate) fn draw_lighting(&mut self, ui: &mut egui::Ui) {
        ui.heading("Lighting");
        ui.add_space(4.0);

        let Some(desc) = self.descriptor.clone() else {
            ui.label("No settings available.");
            return;
        };
        let Some(spec) = desc.lighting.clone() else {
            ui.label(
                "This model's descriptor does not declare configurable lighting.\n\
                 No lighting register will be written.",
            );
            return;
        };
        let profile_index = self.selected_profile;
        let active = self
            .draft
            .as_ref()
            .map(|state| state.active_profile == profile_index)
            .unwrap_or(false);
        let Some(state) = self.draft.as_mut() else {
            ui.label("Waiting for the device to be read…");
            return;
        };
        let baseline_lighting = self
            .baseline
            .as_ref()
            .and_then(|baseline| baseline.profiles.get(profile_index as usize))
            .and_then(|profile| profile.lighting.clone());
        let Some(profile) = state.profiles.get_mut(profile_index as usize) else {
            return;
        };
        let Some(lighting) = profile.lighting.as_mut() else {
            ui.label("No lighting data was read for this profile.");
            return;
        };

        // The mouse shows the lighting of the active profile only.
        if active {
            ui.label(format!(
                "Profile {} is active: applied changes show on the mouse straight away.",
                profile_index + 1
            ));
        } else {
            ui.weak(format!(
                "Profile {} is stored, not active: its lighting shows once you activate it.",
                profile_index + 1
            ));
        }
        ui.add_space(8.0);

        effect_picker(ui, &spec, lighting);
        let mode = spec.mode(&lighting.mode).cloned();
        let uses_color = mode.as_ref().map(|m| m.uses_color).unwrap_or(false);
        let uses_speed = mode.as_ref().map(|m| m.uses_speed).unwrap_or(false);

        if uses_color {
            ui.add_space(10.0);
            colour_picker(ui, lighting);
        }

        ui.add_space(10.0);
        ui.label(egui::RichText::new("Brightness").strong());
        let top = spec.brightness_levels.saturating_sub(1);
        ui.horizontal(|ui| {
            for level in 0..=top {
                if ui
                    .selectable_label(lighting.brightness == level, format!("{level}"))
                    .clicked()
                {
                    lighting.brightness = level;
                }
            }
            ui.weak(format!("0 – {top}"));
        });

        if uses_speed {
            ui.add_space(10.0);
            ui.label(egui::RichText::new("Speed").strong());
            let steps = spec.speed_steps() as u32;
            let mut level = spec.speed_level(lighting.speed) as u32;
            ui.horizontal(|ui| {
                ui.weak("slower");
                if ui
                    .add(egui::Slider::new(&mut level, 1..=steps).show_value(false))
                    .changed()
                {
                    lighting.speed = spec.speed_raw(level as u8);
                }
                ui.weak("faster");
            });
        }

        if !spec.zones.is_empty() {
            ui.add_space(10.0);
            ui.separator();
            ui.label(egui::RichText::new("Zones").strong());
            if lighting.zone_colors.len() < spec.zones.len() {
                lighting
                    .zone_colors
                    .resize(spec.zones.len(), lighting.color);
            }
            egui::Grid::new("zone-grid")
                .num_columns(3)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    for (index, zone) in spec.zones.iter().enumerate() {
                        ui.label(&zone.label);
                        ui.color_edit_button_srgb(&mut lighting.zone_colors[index]);
                        if ui.button("Use main colour").clicked() {
                            lighting.zone_colors[index] = lighting.color;
                        }
                        ui.end_row();
                    }
                });
        }

        ui.add_space(12.0);
        ui.separator();
        match &baseline_lighting {
            Some(baseline) if baseline != lighting => {
                ui.colored_label(
                    egui::Color32::from_rgb(220, 160, 40),
                    "Pending: nothing is written until you apply.",
                );
                ui.weak(format!("On the mouse now: {}", summary(&spec, baseline)));
            }
            Some(baseline) => {
                ui.weak(format!("On the mouse: {}", summary(&spec, baseline)));
            }
            None => {
                ui.small("No baseline read for this profile.");
            }
        }
    }
}

fn effect_picker(ui: &mut egui::Ui, spec: &LightingSpec, lighting: &mut ProfileLighting) {
    ui.label(egui::RichText::new("Effect").strong());
    if lighting.mode.is_empty() {
        ui.colored_label(
            egui::Color32::from_rgb(220, 160, 40),
            format!(
                "The mouse reports an effect this model does not list (code {:#06x}); choose one \
                 below to replace it.",
                lighting.mode_code
            ),
        );
    }
    let mut chosen: Option<String> = None;
    egui::Grid::new("effect-grid")
        .spacing([6.0, 6.0])
        .show(ui, |ui| {
            for (index, mode) in spec.modes.iter().enumerate() {
                let selected = lighting.mode == mode.id;
                let button = egui::Button::selectable(selected, &mode.label)
                    .min_size(egui::vec2(120.0, 28.0));
                if ui.add(button).clicked() && !selected {
                    chosen = Some(mode.id.clone());
                }
                if (index + 1) % EFFECT_COLUMNS == 0 {
                    ui.end_row();
                }
            }
        });
    if let Some(id) = chosen {
        lighting.set_mode(spec, &id);
    }
}

fn colour_picker(ui: &mut egui::Ui, lighting: &mut ProfileLighting) {
    ui.label(egui::RichText::new("Colour").strong());
    ui.horizontal(|ui| {
        ui.color_edit_button_srgb(&mut lighting.color);
        let mut hex = format!(
            "#{:02x}{:02x}{:02x}",
            lighting.color[0], lighting.color[1], lighting.color[2]
        );
        if ui
            .add(egui::TextEdit::singleline(&mut hex).desired_width(80.0))
            .changed()
        {
            if let Some(color) = parse_hex(&hex) {
                lighting.color = color;
            }
        }
        for (color, name) in PRESETS {
            let swatch = egui::Button::new("")
                .fill(egui::Color32::from_rgb(color[0], color[1], color[2]))
                .min_size(egui::vec2(22.0, 22.0));
            if ui.add(swatch).on_hover_text(name).clicked() {
                lighting.color = color;
            }
        }
    });
    egui::CollapsingHeader::new("Fine-tune colour")
        .default_open(false)
        .show(ui, |ui| {
            let (mut hue, mut saturation, mut value) = rgb_to_hsv(lighting.color);
            let mut changed = false;
            changed |= ui
                .add(egui::Slider::new(&mut hue, 0.0..=360.0).text("Hue"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(&mut saturation, 0.0..=1.0).text("Saturation"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(&mut value, 0.0..=1.0).text("Value"))
                .changed();
            if changed {
                lighting.color = hsv_to_rgb(hue, saturation, value);
            }
            for (index, channel) in ["R", "G", "B"].iter().enumerate() {
                let mut component = lighting.color[index] as u32;
                if ui
                    .add(egui::Slider::new(&mut component, 0..=255).text(*channel))
                    .changed()
                {
                    lighting.color[index] = component as u8;
                }
            }
        });
}

fn summary(spec: &LightingSpec, lighting: &ProfileLighting) -> String {
    let label = spec
        .mode(&lighting.mode)
        .or_else(|| spec.mode_by_code(lighting.mode_code))
        .map(|mode| mode.label.clone())
        .unwrap_or_else(|| format!("code {:#06x}", lighting.mode_code));
    let uses_color = spec
        .mode(&lighting.mode)
        .map(|mode| mode.uses_color)
        .unwrap_or(true);
    let mut parts = vec![label];
    if uses_color {
        parts.push(format!(
            "#{:02x}{:02x}{:02x}",
            lighting.color[0], lighting.color[1], lighting.color[2]
        ));
    }
    parts.push(format!("brightness {}", lighting.brightness));
    parts.join(" · ")
}

fn parse_hex(text: &str) -> Option<[u8; 3]> {
    let hex = text.trim().trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    Some([
        u8::from_str_radix(&hex[0..2], 16).ok()?,
        u8::from_str_radix(&hex[2..4], 16).ok()?,
        u8::from_str_radix(&hex[4..6], 16).ok()?,
    ])
}

fn rgb_to_hsv(rgb: [u8; 3]) -> (f32, f32, f32) {
    let r = rgb[0] as f32 / 255.0;
    let g = rgb[1] as f32 / 255.0;
    let b = rgb[2] as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let hue = if delta == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / delta) % 6.0)
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    let hue = if hue < 0.0 { hue + 360.0 } else { hue };
    let saturation = if max == 0.0 { 0.0 } else { delta / max };
    (hue, saturation, max)
}

fn hsv_to_rgb(hue: f32, saturation: f32, value: f32) -> [u8; 3] {
    let c = value * saturation;
    let h = hue / 60.0;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = value - c;
    [
        ((r + m) * 255.0).round().clamp(0.0, 255.0) as u8,
        ((g + m) * 255.0).round().clamp(0.0, 255.0) as u8,
        ((b + m) * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_hsv_round_trip_is_stable() {
        for color in [
            [255, 0, 0],
            [0, 255, 0],
            [0, 0, 255],
            [18, 52, 86],
            [255, 255, 255],
        ] {
            let (h, s, v) = rgb_to_hsv(color);
            let back = hsv_to_rgb(h, s, v);
            assert!(
                back.iter()
                    .zip(color.iter())
                    .all(|(a, b)| (i32::from(*a) - i32::from(*b)).abs() <= 1),
                "{color:?} -> {back:?}"
            );
        }
    }

    #[test]
    fn hex_parsing_accepts_hash_and_plain() {
        assert_eq!(parse_hex("#ff8800"), Some([255, 136, 0]));
        assert_eq!(parse_hex("FF8800"), Some([255, 136, 0]));
        assert_eq!(parse_hex("ff88"), None);
    }
}
