//! Macro library, action editor, and burst fire.

use eframe::egui;

use super::app::App;
use crate::settings::state::{Burst, MacroAction, MacroActionKind};

impl App {
    pub(crate) fn draw_macros(&mut self, ui: &mut egui::Ui) {
        ui.heading("Macros");
        ui.add_space(4.0);

        let Some(desc) = self.descriptor.clone() else {
            ui.label("No settings available.");
            return;
        };
        let Some(spec) = desc.macros.clone() else {
            ui.label("This model's descriptor does not declare macro storage.");
            return;
        };

        let Some(state) = self.draft.as_mut() else {
            ui.label("Waiting for the device to be read…");
            return;
        };
        if self.selected_macro >= spec.slots {
            self.selected_macro = 0;
        }

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Slots").strong());
            for slot in 0..spec.slots {
                let used = state
                    .macros
                    .iter()
                    .find(|m| m.slot == slot)
                    .map(|m| !m.actions.is_empty())
                    .unwrap_or(false);
                let name = state
                    .macros
                    .iter()
                    .find(|m| m.slot == slot)
                    .map(|m| m.name.clone())
                    .unwrap_or_default();
                let label = if name.is_empty() {
                    format!("{}", slot + 1)
                } else {
                    name
                };
                let mut text = egui::RichText::new(label);
                if used {
                    text = text.strong();
                }
                if ui
                    .selectable_label(self.selected_macro == slot, text)
                    .clicked()
                {
                    self.selected_macro = slot;
                }
            }
        });

        let slot = self.selected_macro;
        ui.add_space(6.0);
        let (Some(baseline_state), Some(draft)) = (self.baseline.as_ref(), self.draft.as_mut())
        else {
            return;
        };
        let baseline_actions = baseline_state
            .macros
            .iter()
            .find(|m| m.slot == slot)
            .map(|m| m.actions.clone())
            .unwrap_or_default();
        let Some(macro_state) = draft.macros.iter_mut().find(|m| m.slot == slot) else {
            return;
        };

        let mut name_changed = false;
        let mut delete_requested = false;
        ui.horizontal(|ui| {
            ui.label("Name");
            let mut name = macro_state.name.clone();
            if ui
                .add(egui::TextEdit::singleline(&mut name).desired_width(220.0))
                .changed()
            {
                macro_state.name = name;
                name_changed = true;
            }
            if ui.button("Delete macro").clicked() {
                delete_requested = true;
            }
        });
        if let Some(binding) = macro_state.burst.as_mut() {
            let burst_spec = spec.burst.clone();
            if let Some(burst_spec) = burst_spec {
                ui.horizontal(|ui| {
                    let mut bullets = binding.bullets as u32;
                    if ui
                        .add(
                            egui::Slider::new(
                                &mut bullets,
                                1..=burst_spec.max_bullets.max(1) as u32,
                            )
                            .text("Burst bullets"),
                        )
                        .changed()
                    {
                        binding.bullets = bullets as u8;
                    }
                    let mut delay = binding.delay_ms as u32;
                    if ui
                        .add(
                            egui::DragValue::new(&mut delay)
                                .range(
                                    burst_spec.min_delay_ms as u32..=burst_spec.max_delay_ms as u32,
                                )
                                .suffix(" ms")
                                .prefix("interval "),
                        )
                        .changed()
                    {
                        binding.delay_ms = delay as u16;
                    }
                });
            }
        } else if let Some(burst_spec) = &spec.burst {
            if ui
                .button(format!(
                    "Add burst fire ({}..={} ms)",
                    burst_spec.min_delay_ms, burst_spec.max_delay_ms
                ))
                .clicked()
            {
                macro_state.burst = Some(Burst {
                    bullets: 2,
                    delay_ms: burst_spec.min_delay_ms,
                });
            }
        }

        ui.add_space(8.0);
        ui.separator();
        ui.label(egui::RichText::new("Actions").strong());
        ui.small(format!(
            "{} of {} action records used ({} bytes per record).",
            macro_state.actions.len(),
            spec.max_actions,
            spec.action_len
        ));

        let key_options: Vec<(String, String)> = desc
            .tables
            .keys
            .iter()
            .map(|key| (key.id.clone(), key.label.clone()))
            .collect();
        let mut remove: Option<usize> = None;
        for (index, action) in macro_state.actions.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("{:>2}.", index + 1));
                ui.label(describe_action(action, &key_options));
                if ui.button("Remove").clicked() {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = remove {
            macro_state.actions.remove(index);
        }

        ui.add_space(6.0);
        if macro_state.actions.len() >= spec.max_actions as usize {
            ui.colored_label(
                egui::Color32::from_rgb(220, 160, 40),
                format!(
                    "The {} action limit for this model is reached.",
                    spec.max_actions
                ),
            );
        } else {
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("macro-key")
                    .selected_text(if self.macro_key.is_empty() {
                        "choose key".to_string()
                    } else {
                        self.macro_key.clone()
                    })
                    .show_ui(ui, |ui| {
                        for (id, label) in &key_options {
                            ui.selectable_value(&mut self.macro_key, id.clone(), label);
                        }
                    });
                let key = self.macro_key.clone();
                if ui.button("Add key down").clicked() && !key.is_empty() {
                    macro_state.actions.push(MacroAction {
                        kind: MacroActionKind::KeyDown,
                        key: Some(key),
                        button: None,
                        delay_ms: 0,
                    });
                }
                let key = self.macro_key.clone();
                if ui.button("Add key up").clicked() && !key.is_empty() {
                    macro_state.actions.push(MacroAction {
                        kind: MacroActionKind::KeyUp,
                        key: Some(key),
                        button: None,
                        delay_ms: 0,
                    });
                }
            });
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut self.macro_delay_ms).suffix(" ms"));
                if ui.button("Add delay").clicked() {
                    macro_state.actions.push(MacroAction {
                        kind: MacroActionKind::Delay,
                        key: None,
                        button: None,
                        delay_ms: self.macro_delay_ms,
                    });
                }
                if ui.button("Add mouse button press").clicked() {
                    macro_state.actions.push(MacroAction {
                        kind: MacroActionKind::MouseDown,
                        key: None,
                        button: Some(1),
                        delay_ms: 0,
                    });
                    macro_state.actions.push(MacroAction {
                        kind: MacroActionKind::MouseUp,
                        key: None,
                        button: Some(1),
                        delay_ms: 0,
                    });
                }
            });
        }

        ui.add_space(10.0);
        ui.separator();
        ui.label(egui::RichText::new("On the mouse").strong());
        if baseline_actions == macro_state.actions {
            ui.small("The editor matches the actions read from the device.");
        } else {
            ui.colored_label(
                egui::Color32::from_rgb(220, 160, 40),
                "The editor holds pending changes; they reach the mouse only when you apply.",
            );
        }

        ui.add_space(10.0);
        ui.separator();
        ui.label(egui::RichText::new("Macro files").strong());
        ui.horizontal(|ui| {
            ui.label("Import");
            ui.add(
                egui::TextEdit::singleline(&mut self.macro_import_path)
                    .hint_text("/path/to/macro.json")
                    .desired_width(280.0),
            );
            if ui.button("Import").clicked() {
                self.import_macro_file();
            }
        });
        ui.horizontal(|ui| {
            ui.label("Export");
            ui.add(
                egui::TextEdit::singleline(&mut self.macro_export_path)
                    .hint_text("macro.json")
                    .desired_width(280.0),
            );
            if ui.button("Export").clicked() {
                self.request_macro_export();
            }
        });

        if delete_requested {
            self.delete_macro(slot);
        }
        if name_changed {
            self.names_dirty = true;
        }

        ui.add_space(10.0);
        ui.separator();
        ui.small(
            "Macro names live in this tool's configuration, not on the mouse; the vendor format \
             stores only the records. Action encoding for this model is a hypothesis and the \
             descriptor is unverified, so writes need the extra confirmation.",
        );
    }
}

pub(crate) fn describe_action(action: &MacroAction, keys: &[(String, String)]) -> String {
    match action.kind {
        MacroActionKind::KeyDown => format!(
            "Key down · {}",
            action
                .key
                .as_deref()
                .map(|key| label_for(key, keys))
                .unwrap_or_else(|| "?".into())
        ),
        MacroActionKind::KeyUp => format!(
            "Key up · {}",
            action
                .key
                .as_deref()
                .map(|key| label_for(key, keys))
                .unwrap_or_else(|| "?".into())
        ),
        MacroActionKind::MouseDown => {
            format!("Mouse button {} down", action.button.unwrap_or(1))
        }
        MacroActionKind::MouseUp => format!("Mouse button {} up", action.button.unwrap_or(1)),
        MacroActionKind::Delay => format!("Delay {} ms", action.delay_ms),
    }
}

fn label_for(key: &str, keys: &[(String, String)]) -> String {
    keys.iter()
        .find(|(id, _)| id == key)
        .map(|(_, label)| label.clone())
        .unwrap_or_else(|| key.to_string())
}
