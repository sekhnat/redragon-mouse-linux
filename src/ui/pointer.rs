//! Pointer tuning: DPI stages, polling rate, and global pointer settings.

use eframe::egui;

use super::app::App;
use crate::settings::state::ProfileDpi;

impl App {
    pub(crate) fn draw_pointer(&mut self, ui: &mut egui::Ui) {
        ui.heading("Pointer");
        ui.add_space(4.0);

        let Some(desc) = self.descriptor.clone() else {
            ui.label("No settings available.");
            return;
        };
        let profile_index = self.selected_profile;
        let Some(state) = self.draft.as_mut() else {
            ui.label("Waiting for the device to be read…");
            return;
        };
        let Some(profile) = state.profiles.get_mut(profile_index as usize) else {
            return;
        };

        let active = state.active_profile == profile_index;
        ui.weak(if active {
            format!("Profile {} is active on the mouse.", profile_index + 1)
        } else {
            format!(
                "Profile {} is stored, not active: its settings apply once you activate it.",
                profile_index + 1
            )
        });
        ui.add_space(6.0);

        if let Some(spec) = &desc.dpi {
            ui.label(egui::RichText::new("DPI stages").strong());
            ui.small(
                "The DPI button on the mouse cycles through the enabled stages; the marked stage \
                 is the one in use.",
            );
            let values = desc.dpi_values();

            let Some(dpi) = profile.dpi.as_mut() else {
                ui.label("No DPI data was read.");
                ui.add_space(8.0);
                return;
            };
            let enabled_count = dpi.stages.iter().filter(|stage| stage.enabled).count();
            let mut toggle: Option<(usize, bool)> = None;
            let mut new_active: Option<u8> = None;
            let mut set_value: Option<(usize, u16)> = None;
            let mut set_link: Option<(usize, bool)> = None;

            egui::Grid::new("dpi-grid")
                .num_columns(if spec.per_axis { 5 } else { 4 })
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label(egui::RichText::new("Stage").strong());
                    ui.label(egui::RichText::new("On").strong());
                    ui.label(egui::RichText::new("DPI").strong());
                    if spec.per_axis {
                        ui.label(egui::RichText::new("Link X/Y").strong());
                    }
                    ui.label(egui::RichText::new("In use").strong());
                    ui.end_row();

                    for (index, stage) in dpi.stages.iter().enumerate() {
                        ui.label(format!("{}", index + 1));
                        let mut enabled = stage.enabled;
                        if ui.checkbox(&mut enabled, "").changed() {
                            toggle = Some((index, enabled));
                        }
                        ui.horizontal(|ui| {
                            if let Some(value) = dpi_slider(ui, &values, stage.x, index) {
                                set_value = Some((index, value));
                            }
                            if stage.x == 0 {
                                ui.colored_label(
                                    egui::Color32::from_rgb(220, 160, 40),
                                    format!("code {:#06x} is not in the table", stage.raw),
                                );
                            }
                        });
                        if spec.per_axis {
                            let mut linked = dpi.linked.get(index).copied().unwrap_or(true);
                            if ui.checkbox(&mut linked, "").changed() {
                                set_link = Some((index, linked));
                            }
                        }
                        if ui.radio(dpi.active == index as u8, "").clicked() {
                            new_active = Some(index as u8);
                        }
                        ui.end_row();
                    }
                });

            if let Some((index, enabled)) = toggle {
                if !enabled && enabled_count <= 1 {
                    self.status = Some(super::app::StatusMessage {
                        text: "At least one DPI stage must stay enabled.".into(),
                        error: true,
                    });
                } else {
                    dpi.stages[index].enabled = enabled;
                    if !enabled && dpi.active == index as u8 {
                        dpi.active = nearest_enabled(dpi, index);
                    }
                }
            }
            if let Some((index, value)) = set_value {
                dpi.stages[index].x = value;
                if !spec.per_axis || dpi.linked.get(index).copied().unwrap_or(true) {
                    dpi.stages[index].y = value;
                }
            }
            if let Some((index, linked)) = set_link {
                if let Some(entry) = dpi.linked.get_mut(index) {
                    *entry = linked;
                }
                if linked {
                    dpi.stages[index].y = dpi.stages[index].x;
                }
            }
            if let Some(active) = new_active {
                if dpi
                    .stages
                    .get(active as usize)
                    .is_some_and(|stage| stage.enabled)
                {
                    dpi.active = active;
                } else {
                    self.status = Some(super::app::StatusMessage {
                        text: "Enable a stage before making it active.".into(),
                        error: true,
                    });
                }
            }
        }

        if let Some(spec) = &desc.polling {
            ui.add_space(12.0);
            ui.label(egui::RichText::new("Polling rate").strong());
            ui.horizontal(|ui| {
                for code in &spec.codes {
                    let selected = profile.polling_hz == Some(code.hz);
                    if ui
                        .selectable_label(selected, format!("{} Hz", code.hz))
                        .clicked()
                    {
                        profile.polling_hz = Some(code.hz);
                    }
                }
                if profile.polling_hz.is_none() {
                    ui.weak("not read");
                }
            });
            ui.small("Higher rates are smoother but use more CPU; 1000 Hz suits most games.");
        }

        if let Some(spec) = &desc.device {
            ui.add_space(10.0);
            ui.separator();
            ui.label(egui::RichText::new("Pointer behaviour (device-wide)").strong());
            let mut device = state.device.clone();
            let mut changed = false;
            if let Some(field) = &spec.angle_snap {
                let mut value = device.angle_snap.unwrap_or(false);
                if ui.checkbox(&mut value, "Angle snap").changed() {
                    device.angle_snap = Some(value);
                    changed = true;
                }
                let _ = field;
            }
            if let Some(field) = &spec.lift_off {
                let current = device.lift_off.clone().unwrap_or_else(|| "not read".into());
                egui::ComboBox::from_id_salt("liftoff")
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        for value in &field.values {
                            if ui
                                .selectable_label(
                                    device.lift_off.as_deref() == Some(value.id.as_str()),
                                    &value.label,
                                )
                                .clicked()
                            {
                                device.lift_off = Some(value.id.clone());
                                changed = true;
                            }
                        }
                    });
            }
            if let Some(field) = &spec.scroll {
                let mut value = device.scroll.unwrap_or(field.min) as u32;
                if ui
                    .add(
                        egui::Slider::new(&mut value, field.min as u32..=field.max as u32)
                            .text("Scroll speed"),
                    )
                    .changed()
                {
                    device.scroll = Some(value as u16);
                    changed = true;
                }
            }
            if let Some(field) = &spec.scroll_page {
                let mut value = device.scroll_page.unwrap_or(false);
                if ui.checkbox(&mut value, "Scroll by page").changed() {
                    device.scroll_page = Some(value);
                    changed = true;
                }
                let _ = field;
            }
            if let Some(field) = &spec.accel {
                let mut value = device.accel.unwrap_or(false);
                if ui.checkbox(&mut value, "Pointer acceleration").changed() {
                    device.accel = Some(value);
                    changed = true;
                }
                let _ = field;
            }
            if let Some(field) = &spec.double_click {
                let mut value = device.double_click.unwrap_or(field.min) as u32;
                if ui
                    .add(
                        egui::Slider::new(&mut value, field.min as u32..=field.max as u32)
                            .text("Double-click speed"),
                    )
                    .changed()
                {
                    device.double_click = Some(value as u16);
                    changed = true;
                }
            } else {
                ui.small("This model does not declare a double-click register.");
            }
            if changed {
                state.device = device;
            }
        }

        ui.add_space(10.0);
        ui.separator();
        ui.small("Values are read from the mouse. Unchanged registers are never rewritten.");
    }
}

/// A slider over the model's DPI table (index, not value, so the uneven step
/// above 16000 DPI still moves one entry at a time). Returns the new value when
/// the person changed it.
fn dpi_slider(ui: &mut egui::Ui, values: &[u16], current: u16, id: usize) -> Option<u16> {
    if values.is_empty() {
        return None;
    }
    let last = values.len() - 1;
    let mut position = values.iter().position(|v| *v == current).unwrap_or(0);
    let table = values.to_vec();
    let parse_table = values.to_vec();
    let slider = egui::Slider::new(&mut position, 0..=last)
        .show_value(true)
        .custom_formatter(move |n, _| format!("{} DPI", table[(n as usize).min(last)]))
        .custom_parser(move |text| {
            let wanted: f64 = text
                .trim()
                .trim_end_matches(|c: char| !c.is_ascii_digit())
                .parse()
                .ok()?;
            parse_table
                .iter()
                .enumerate()
                .min_by_key(|(_, v)| ((**v as f64) - wanted).abs() as i64)
                .map(|(i, _)| i as f64)
        });
    let response = ui.push_id(("dpi-slider", id), |ui| ui.add(slider)).inner;
    response.changed().then(|| values[position])
}

fn nearest_enabled(dpi: &ProfileDpi, disabled: usize) -> u8 {
    for offset in 1..dpi.stages.len() {
        if let Some(index) = disabled.checked_sub(offset) {
            if dpi.stages[index].enabled {
                return index as u8;
            }
        }
        let index = disabled + offset;
        if index < dpi.stages.len() && dpi.stages[index].enabled {
            return index as u8;
        }
    }
    disabled as u8
}
