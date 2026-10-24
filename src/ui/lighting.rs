//! Lighting: effect, colour, brightness, speed, and zones.

use eframe::egui;

use super::app::App;

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

        ui.label(egui::RichText::new("Effect").strong());
        let current_mode = if lighting.mode.is_empty() {
            format!("Unrecognised code {:#06x}", lighting.mode_code)
        } else {
            spec.mode(&lighting.mode)
                .map(|mode| mode.label.clone())
                .unwrap_or_else(|| lighting.mode.clone())
        };
        let mut selected_mode = lighting.mode.clone();
        egui::ComboBox::from_id_salt("led-mode")
            .selected_text(current_mode)
            .show_ui(ui, |ui| {
                for mode in &spec.modes {
                    ui.selectable_value(&mut selected_mode, mode.id.clone(), &mode.label);
                }
            });
        if selected_mode != lighting.mode {
            lighting.set_mode(&spec, &selected_mode);
        }
        let mode = spec.mode(&lighting.mode).cloned();
        let uses_color = mode.as_ref().map(|mode| mode.uses_color).unwrap_or(false);
        let uses_speed = mode.as_ref().map(|mode| mode.uses_speed).unwrap_or(false);

        ui.add_space(8.0);
        ui.label(egui::RichText::new("Colour").strong());
        ui.add_enabled_ui(uses_color, |ui| {
            ui.horizontal(|ui| {
                ui.color_edit_button_srgb(&mut lighting.color);
                let mut hex = format!(
                    "#{:02x}{:02x}{:02x}",
                    lighting.color[0], lighting.color[1], lighting.color[2]
                );
                if ui
                    .add(egui::TextEdit::singleline(&mut hex).desired_width(90.0))
                    .changed()
                {
                    if let Some(color) = parse_hex(&hex) {
                        lighting.color = color;
                    }
                }
                ui.label("RGB");
            });
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
            ui.horizontal(|ui| {
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
        });
        if !uses_color {
            ui.small(
                "The selected effect does not use a colour; colour registers are left untouched.",
            );
        }

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let mut brightness = lighting.brightness as u32;
            let max = spec.brightness_levels.saturating_sub(1) as u32;
            if ui
                .add(egui::Slider::new(&mut brightness, 0..=max).text("Brightness"))
                .changed()
            {
                lighting.brightness = brightness as u8;
            }
        });
        ui.add_enabled_ui(uses_speed, |ui| {
            let max = spec.speed_levels.saturating_sub(1) as u32;
            let raw = (lighting.speed as u32).min(max);
            // Show "higher is faster" even when the register counts the other way.
            let mut speed = if spec.speed_inverted { max - raw } else { raw };
            if ui
                .add(egui::Slider::new(&mut speed, 0..=max).text("Effect speed"))
                .changed()
            {
                lighting.speed = if spec.speed_inverted {
                    max - speed
                } else {
                    speed
                } as u8;
            }
        });
        if !uses_speed {
            ui.small("This effect has no adjustable speed on this model.");
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

        ui.add_space(10.0);
        ui.separator();
        ui.label(egui::RichText::new("On the mouse").strong());
        match &baseline_lighting {
            Some(baseline) => {
                let mode_label = if baseline.mode.is_empty() {
                    spec.mode_by_code(baseline.mode_code)
                        .map(|mode| mode.label.clone())
                        .unwrap_or_else(|| format!("code {:#06x}", baseline.mode_code))
                } else {
                    spec.mode(&baseline.mode)
                        .map(|mode| mode.label.clone())
                        .unwrap_or_else(|| baseline.mode.clone())
                };
                ui.label(format!(
                    "{} · colour #{:02x}{:02x}{:02x} · speed {} · brightness {}",
                    mode_label,
                    baseline.color[0],
                    baseline.color[1],
                    baseline.color[2],
                    baseline.speed,
                    baseline.brightness
                ));
                if baseline.mode != lighting.mode
                    || baseline.color != lighting.color
                    || baseline.speed != lighting.speed
                    || baseline.brightness != lighting.brightness
                {
                    ui.colored_label(
                        egui::Color32::from_rgb(220, 160, 40),
                        "The values above are pending; the preview is not the device's state.",
                    );
                }
            }
            None => {
                ui.small("No baseline read for this profile.");
            }
        }
    }
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
