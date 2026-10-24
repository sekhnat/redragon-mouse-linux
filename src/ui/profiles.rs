//! Profile slots, activation, copy/reset, and file import/export.

use eframe::egui;

use super::app::{App, Dialog, PendingAction};

impl App {
    pub(crate) fn draw_profiles(&mut self, ui: &mut egui::Ui) {
        ui.heading("Profiles");
        ui.add_space(4.0);

        let Some(desc) = self.descriptor.clone() else {
            ui.label("No settings available.");
            return;
        };
        let Some(state) = &self.draft else {
            ui.label("Waiting for the device to be read…");
            return;
        };

        ui.label(
            "Each slot lives on the mouse. Editing a slot only changes that slot; switching \
             profiles writes the activation sequence.",
        );
        ui.add_space(6.0);

        let mut activate: Option<u8> = None;
        let mut edit: Option<u8> = None;
        egui::Grid::new("profile-slots")
            .num_columns(5)
            .spacing([14.0, 6.0])
            .show(ui, |ui| {
                ui.label(egui::RichText::new("Slot").strong());
                ui.label(egui::RichText::new("State").strong());
                ui.label(egui::RichText::new("Polling").strong());
                ui.label(egui::RichText::new("DPI stage").strong());
                ui.label("");
                ui.end_row();
                for profile in desc.profile_indices() {
                    let active = state.active_profile == profile;
                    let editing = self.selected_profile == profile;
                    ui.label(format!("Profile {}", profile + 1));
                    if active {
                        ui.colored_label(egui::Color32::from_rgb(90, 170, 90), "active");
                    } else {
                        ui.weak("stored");
                    }
                    let slot = state.profiles.get(profile as usize);
                    ui.label(
                        slot.and_then(|slot| slot.polling_hz)
                            .map(|hz| format!("{hz} Hz"))
                            .unwrap_or_else(|| "—".into()),
                    );
                    ui.label(
                        slot.and_then(|slot| slot.dpi.as_ref())
                            .map(|dpi| format!("{} / {}", dpi.active + 1, dpi.stages.len()))
                            .unwrap_or_else(|| "—".into()),
                    );
                    ui.horizontal(|ui| {
                        if !active && ui.button("Activate").clicked() {
                            activate = Some(profile);
                        }
                        if ui
                            .selectable_label(editing, if editing { "Editing" } else { "Edit" })
                            .clicked()
                        {
                            edit = Some(profile);
                        }
                    });
                    ui.end_row();
                }
            });

        if let Some(profile) = edit {
            self.selected_profile = profile;
            self.section = super::app::Section::Pointer;
        }
        if let Some(profile) = activate {
            if self.is_dirty() {
                self.dialog = Some(Dialog::Notice {
                    title: "Apply or discard first".into(),
                    lines: vec![
                        "Switching profiles would leave the pending edits without a home.".into(),
                        "Apply or discard them, then activate the profile.".into(),
                    ],
                });
            } else {
                self.dialog = Some(Dialog::Confirm {
                    title: format!("Activate profile {}", profile + 1),
                    body: "The mouse switches to this slot. The switch is confirmed by reading \
                           the active profile back."
                        .into(),
                    action: PendingAction::ActivateProfile(profile),
                    destructive: false,
                });
            }
        }

        ui.add_space(12.0);
        ui.separator();
        ui.label(egui::RichText::new("Copy or reset").strong());
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("copy-from")
                .selected_text(format!("from profile {}", self.copy_from + 1))
                .show_ui(ui, |ui| {
                    for profile in desc.profile_indices() {
                        ui.selectable_value(
                            &mut self.copy_from,
                            profile,
                            format!("Profile {}", profile + 1),
                        );
                    }
                });
            egui::ComboBox::from_id_salt("copy-to")
                .selected_text(format!("to profile {}", self.copy_to + 1))
                .show_ui(ui, |ui| {
                    for profile in desc.profile_indices() {
                        ui.selectable_value(
                            &mut self.copy_to,
                            profile,
                            format!("Profile {}", profile + 1),
                        );
                    }
                });
            if ui.button("Copy").clicked() {
                if self.copy_from == self.copy_to {
                    self.status = Some(super::app::StatusMessage {
                        text: "Source and destination are the same slot.".into(),
                        error: true,
                    });
                } else {
                    self.dialog = Some(Dialog::Confirm {
                        title: "Copy profile settings".into(),
                        body: format!(
                            "Profile {} will be overwritten with profile {}'s settings.",
                            self.copy_to + 1,
                            self.copy_from + 1
                        ),
                        action: PendingAction::CopyProfile {
                            from: self.copy_from,
                            to: self.copy_to,
                        },
                        destructive: true,
                    });
                }
            }
        });
        if ui
            .button(format!(
                "Reset profile {} to defaults",
                self.selected_profile + 1
            ))
            .clicked()
        {
            self.dialog = Some(Dialog::Confirm {
                title: "Reset profile to defaults".into(),
                body: format!(
                    "Profile {} will be reset to the descriptor's default values. A backup of \
                     the current registers is written before anything is sent.",
                    self.selected_profile + 1
                ),
                action: PendingAction::ResetProfile(self.selected_profile),
                destructive: true,
            });
        }

        ui.add_space(12.0);
        ui.separator();
        ui.label(egui::RichText::new("Profile files").strong());
        ui.label(
            "Files use the vendor .prof field names. Import validates every value against this \
             model and rejects the whole file if one cannot be expressed.",
        );
        ui.horizontal(|ui| {
            ui.label("Import");
            ui.add(
                egui::TextEdit::singleline(&mut self.import_path)
                    .hint_text("/path/to/Profile 1.prof")
                    .desired_width(320.0),
            );
            if ui.button("Import").clicked() {
                self.import_profile_file();
            }
        });
        ui.horizontal(|ui| {
            ui.label("Export");
            ui.add(
                egui::TextEdit::singleline(&mut self.export_path)
                    .hint_text(format!("profile-{}.prof", self.selected_profile + 1))
                    .desired_width(320.0),
            );
            if ui.button("Export").clicked() {
                self.request_export();
            }
        });
        let suggested = format!(
            "{}/Profile {}.prof",
            std::env::var("HOME").unwrap_or_else(|_| ".".into()),
            desc.profile_indices()
                .nth(self.selected_profile as usize)
                .unwrap_or(0)
                + 1
        );
        ui.small(format!(
            "Vendor tree example: REDRAGON Gaming Mouse/<MODEL>/Profile/*.prof ({suggested})"
        ));
    }
}
