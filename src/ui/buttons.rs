//! Button mappings, combos, and the primary-click guard.

use eframe::egui;

use super::app::{App, Dialog, PendingAction, StatusMessage};
use crate::models::ActionKind;

impl App {
    pub(crate) fn draw_buttons(&mut self, ui: &mut egui::Ui) {
        ui.heading("Buttons");
        ui.add_space(4.0);

        let Some(desc) = self.descriptor.clone() else {
            ui.label("No settings available.");
            return;
        };
        let Some(spec) = desc.buttons.clone() else {
            ui.label(
                "This model's descriptor does not declare button mappings.\n\
                 The shipped documentation does not pin the keymap registers, so no keymap write \
                 is attempted.",
            );
            return;
        };
        let writable = !spec.write_addrs.is_empty();
        if !writable {
            ui.colored_label(
                egui::Color32::from_rgb(220, 160, 40),
                "Button mappings are shown read-only: this model's keymap registers are not \
                 captured yet, so editing is disabled.",
            );
        }
        ui.label(format!(
            "Profile {} · {} buttons{}",
            self.selected_profile + 1,
            spec.count,
            if spec.combo_slots > 0 {
                format!(" · combos up to {} keys", spec.combo_slots)
            } else {
                String::new()
            }
        ));
        ui.add_space(6.0);

        let profile_index = self.selected_profile;
        let Some(state) = self.draft.as_mut() else {
            ui.label("Waiting for the device to be read…");
            return;
        };
        let Some(bindings) = state
            .profiles
            .get_mut(profile_index as usize)
            .and_then(|profile| profile.buttons.as_mut())
        else {
            ui.label("No button data was read for this profile.");
            return;
        };

        let mut primary_request: Option<(usize, String)> = None;
        let mut messages: Vec<String> = Vec::new();
        let mut reset_requested = false;

        egui::Grid::new("button-grid")
            .num_columns(4)
            .spacing([14.0, 8.0])
            .show(ui, |ui| {
                ui.label(egui::RichText::new("Button").strong());
                ui.label(egui::RichText::new("Action").strong());
                ui.label(egui::RichText::new("Detail").strong());
                ui.label(egui::RichText::new("Combo").strong());
                ui.end_row();

                for (index, binding) in bindings.iter_mut().enumerate() {
                    let name = spec
                        .names
                        .get(index)
                        .cloned()
                        .unwrap_or_else(|| format!("Button {}", index + 1));
                    ui.label(&name);

                    let known = desc.tables.actions.iter().find(|a| a.id == binding.action);
                    let action_label = known
                        .map(|action| action.label.clone())
                        .unwrap_or_else(|| format!("Unrecognised ({})", binding.action));
                    let mut selected = binding.action.clone();
                    ui.add_enabled_ui(writable, |ui| {
                        egui::ComboBox::from_id_salt(("button-action", index))
                            .selected_text(action_label)
                            .show_ui(ui, |ui| {
                                for action in &desc.tables.actions {
                                    ui.selectable_value(
                                        &mut selected,
                                        action.id.clone(),
                                        &action.label,
                                    );
                                }
                            });
                    });
                    if selected != binding.action && writable {
                        if index == spec.primary as usize {
                            primary_request = Some((index, selected));
                        } else {
                            binding.action = selected;
                        }
                    }

                    let kind = desc
                        .tables
                        .actions
                        .iter()
                        .find(|action| action.id == binding.action)
                        .map(|action| action.kind);
                    match kind {
                        Some(ActionKind::Key) | Some(ActionKind::Multimedia)
                            if writable =>
                        {
                            let mut key = binding.key.clone().unwrap_or_default();
                            egui::ComboBox::from_id_salt(("button-key", index))
                                .selected_text(if key.is_empty() {
                                    "choose key".to_string()
                                } else {
                                    desc.tables
                                        .keys
                                        .iter()
                                        .find(|k| k.id == key)
                                        .map(|k| k.label.clone())
                                        .unwrap_or_else(|| key.clone())
                                })
                                .show_ui(ui, |ui| {
                                    for candidate in &desc.tables.keys {
                                        ui.selectable_value(
                                            &mut key,
                                            candidate.id.clone(),
                                            &candidate.label,
                                        );
                                    }
                                });
                            if Some(&key) != binding.key.as_ref() && !key.is_empty() {
                                binding.key = Some(key);
                            }
                        }
                        Some(ActionKind::Macro) if writable => {
                            let macro_desc = desc.macros.clone();
                            if let Some(macros) = macro_desc {
                                let mut slot = binding.macro_slot;
                                egui::ComboBox::from_id_salt(("button-macro", index))
                                    .selected_text(
                                        slot.map(|slot| format!("macro {}", slot + 1))
                                            .unwrap_or_else(|| "choose macro".into()),
                                    )
                                    .show_ui(ui, |ui| {
                                        for candidate in 0..macros.slots {
                                            ui.selectable_value(
                                                &mut slot,
                                                Some(candidate),
                                                format!(
                                                    "{} {}",
                                                    candidate + 1,
                                                    state
                                                        .macros
                                                        .iter()
                                                        .find(|m| m.slot == candidate)
                                                        .map(|m| m.name.clone())
                                                        .unwrap_or_default()
                                                ),
                                            );
                                        }
                                    });
                                if slot != binding.macro_slot {
                                    binding.macro_slot = slot;
                                }
                            } else {
                                ui.weak("no macros");
                            }
                        }
                        _ => {
                            if let Some(key) = &binding.key {
                                ui.label(
                                    desc.tables
                                        .keys
                                        .iter()
                                        .find(|k| &k.id == key)
                                        .map(|k| k.label.clone())
                                        .unwrap_or_else(|| key.clone()),
                                );
                            } else {
                                ui.weak("—");
                            }
                        }
                    }

                    if writable && spec.combo_slots > 0 {
                        ui.horizontal(|ui| {
                            for (slot, key) in binding.combo.clone().iter().enumerate() {
                                let label = desc
                                    .tables
                                    .keys
                                    .iter()
                                    .find(|k| &k.id == key)
                                    .map(|k| k.label.clone())
                                    .unwrap_or_else(|| key.clone());
                                if ui.button(format!("{label} ✕")).clicked() {
                                    binding.combo.remove(slot);
                                }
                            }
                            if binding.combo.len() < spec.combo_slots as usize {
                                egui::ComboBox::from_id_salt(("button-combo", index))
                                    .selected_text("add key")
                                    .show_ui(ui, |ui| {
                                        for key in &desc.tables.keys {
                                            if ui.button(&key.label).clicked() {
                                                if binding.combo.len()
                                                    < spec.combo_slots as usize
                                                {
                                                    binding.combo.push(key.id.clone());
                                                } else {
                                                    messages.push(format!(
                                                        "A combo holds at most {} keys on this model.",
                                                        spec.combo_slots
                                                    ));
                                                }
                                            }
                                        }
                                    });
                            } else {
                                ui.weak(format!("limit {} reached", spec.combo_slots));
                            }
                        });
                    } else {
                        ui.weak("—");
                    }
                    ui.end_row();
                }
            });

        if let Some((index, action)) = primary_request {
            let name = spec
                .names
                .get(index)
                .cloned()
                .unwrap_or_else(|| format!("Button {}", index + 1));
            self.dialog = Some(Dialog::Confirm {
                title: format!("Reassign the {} button?", name.to_lowercase()),
                body: format!(
                    "Changing the primary click button can leave you unable to click in this \
                     application or on the desktop.\n\n\
                     Recovery: reconnect the mouse and restore a backup from the Backups screen, \
                     or run the vendor software.\n\n\
                     New action: {action}"
                ),
                action: PendingAction::ReassignPrimary {
                    profile: profile_index,
                    button: index,
                    action,
                },
                destructive: true,
            });
        }
        for message in messages {
            self.status = Some(StatusMessage {
                text: message,
                error: true,
            });
        }

        ui.add_space(12.0);
        ui.add_enabled_ui(writable, |ui| {
            if ui.button("Reset all buttons in this profile").clicked() {
                reset_requested = true;
            }
        });
        if reset_requested {
            self.dialog = Some(Dialog::Confirm {
                title: "Reset button mappings".into(),
                body: format!(
                    "Every button in profile {} will be reset to the descriptor's default \
                     mappings.",
                    self.selected_profile + 1
                ),
                action: PendingAction::ResetButtons,
                destructive: true,
            });
        }

        ui.add_space(10.0);
        ui.separator();
        ui.small(
            "Mappings are read back from the mouse. A value the descriptor cannot decode is shown \
             as unrecognised and never replaced with a default.",
        );
    }
}
