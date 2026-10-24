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

        if let Some(spec) = &desc.dpi {
            ui.label(egui::RichText::new("DPI stages").strong());
            ui.small(format!(
                "Profile {} · {} stage(s) · {} values in this model's table",
                profile_index + 1,
                spec.stage_count,
                desc.tables.dpi_codes.len()
            ));
            if !spec.per_axis {
                ui.small(
                    "This model stores a single sensitivity value per stage; X and Y stay linked.",
                );
            }
            let values: Vec<u16> = desc
                .tables
                .dpi_codes
                .iter()
                .map(|entry| entry.dpi)
                .collect();

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
                .num_columns(6)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    ui.label(egui::RichText::new("Stage").strong());
                    ui.label(egui::RichText::new("On").strong());
                    ui.label(egui::RichText::new("DPI").strong());
                    ui.label(egui::RichText::new("Link X/Y").strong());
                    ui.label(egui::RichText::new("Active").strong());
                    ui.label("");
                    ui.end_row();

                    for (index, stage) in dpi.stages.iter().enumerate() {
                        ui.label(format!("{}", index + 1));
                        let mut enabled = stage.enabled;
                        if ui.checkbox(&mut enabled, "").changed() {
                            toggle = Some((index, enabled));
                        }
                        let current = if stage.x == 0 {
                            format!("unknown code {:#06x}", stage.raw)
                        } else {
                            format!("{}", stage.x)
                        };
                        egui::ComboBox::from_id_salt(("dpi-value", index))
                            .selected_text(current)
                            .show_ui(ui, |ui| {
                                egui::ScrollArea::vertical()
                                    .max_height(320.0)
                                    .show(ui, |ui| {
                                        for value in &values {
                                            if ui
                                                .selectable_label(
                                                    stage.x == *value,
                                                    format!("{value}"),
                                                )
                                                .clicked()
                                            {
                                                set_value = Some((index, *value));
                                            }
                                        }
                                    });
                            });
                        if spec.per_axis {
                            let mut linked = dpi.linked.get(index).copied().unwrap_or(true);
                            if ui.checkbox(&mut linked, "").changed() {
                                set_link = Some((index, linked));
                            }
                        } else {
                            ui.weak("linked");
                        }
                        if ui.radio(dpi.active == index as u8, "").clicked() {
                            new_active = Some(index as u8);
                        }
                        if stage.x == 0 {
                            ui.weak("not in this model's table");
                        } else {
                            ui.weak("");
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
            ui.add_space(10.0);
            ui.label(egui::RichText::new("Polling rate").strong());
            let current = profile.polling_hz;
            let mut selected = current;
            egui::ComboBox::from_id_salt("polling")
                .selected_text(match current {
                    Some(hz) => format!("{hz} Hz"),
                    None => "not read".into(),
                })
                .show_ui(ui, |ui| {
                    for code in &spec.codes {
                        ui.selectable_value(
                            &mut selected,
                            Some(code.hz),
                            format!("{} Hz", code.hz),
                        );
                    }
                });
            if selected != current {
                profile.polling_hz = selected;
            }
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
