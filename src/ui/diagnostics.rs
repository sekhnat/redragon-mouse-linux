//! Diagnostics: log location, descriptor problems, access guidance, apply report.

use eframe::egui;

use super::app::{App, Mode};

impl App {
    pub(crate) fn draw_diagnostics(&mut self, ui: &mut egui::Ui) {
        ui.heading("Diagnostics");
        ui.add_space(4.0);

        egui::Grid::new("diag")
            .num_columns(2)
            .spacing([14.0, 6.0])
            .show(ui, |ui| {
                ui.label("Session log");
                ui.horizontal(|ui| {
                    if self.log_path.as_os_str().is_empty() {
                        ui.weak("no log file");
                    } else {
                        ui.monospace(self.log_path.display().to_string());
                        if ui.button("Copy").clicked() {
                            ui.ctx().copy_text(self.log_path.display().to_string());
                        }
                    }
                });
                ui.end_row();
                ui.label("Mode");
                match &self.mode {
                    Mode::Hardware => {
                        ui.label("hardware");
                    }
                    Mode::Simulation { model } => {
                        ui.label(format!("simulation — {model} (no hardware access)"));
                    }
                }
                ui.end_row();
                ui.label("Descriptors");
                ui.label(self.registry.descriptors.len().to_string());
                ui.end_row();
            });

        if !self.registry.errors.is_empty() {
            ui.add_space(8.0);
            ui.colored_label(
                egui::Color32::from_rgb(220, 160, 40),
                "Descriptor load problems",
            );
            for error in &self.registry.errors {
                ui.label(format!("{}: {}", error.file.display(), error.message));
            }
        }

        if let Some(desc) = &self.descriptor {
            ui.add_space(8.0);
            ui.separator();
            ui.label(egui::RichText::new("Active descriptor").strong());
            ui.label(format!(
                "{} ({}) · schema {} · {}",
                desc.model,
                desc.id,
                desc.schema,
                desc.provenance.label()
            ));
            ui.label(format!(
                "Verified: {}",
                if desc.verified {
                    "yes"
                } else {
                    "no — writes require the extra confirmation"
                }
            ));
            ui.label(format!(
                "Write regions: {} · read windows: {}",
                desc.write_footprint().len(),
                desc.window_instances().map(|list| list.len()).unwrap_or(0)
            ));
        }

        if let Some(guidance) = &self.access {
            ui.add_space(8.0);
            ui.separator();
            ui.label(egui::RichText::new("Access guidance").strong());
            ui.label(&guidance.detail);
            ui.monospace(&guidance.rule);
            if ui.button("Copy udev rule").clicked() {
                ui.ctx().copy_text(guidance.rule.clone());
            }
            if ui.button("Copy install commands").clicked() {
                ui.ctx().copy_text(guidance.instructions());
            }
        }

        if let Some(report) = &self.last_report {
            ui.add_space(8.0);
            ui.separator();
            ui.label(egui::RichText::new("Last apply report").strong());
            ui.label(format!(
                "{} changed · {} confirmed · {} mismatch(es)",
                report.changed,
                report.confirmed,
                report.mismatches.len()
            ));
            for mismatch in &report.mismatches {
                ui.colored_label(egui::Color32::from_rgb(220, 160, 40), mismatch);
            }
            for warning in &report.stale_check_errors {
                ui.weak(warning);
            }
            for warning in &report.warnings {
                ui.colored_label(egui::Color32::from_rgb(220, 160, 40), warning);
            }
            if let Some(backup) = &report.backup {
                ui.label(format!("Backup: {}", backup.display()));
            }
        }

        ui.add_space(10.0);
        ui.separator();
        ui.small(
            "The application never elevates privileges or modifies system files. Install the udev \
             rule yourself if the vendor interface is not accessible.",
        );
        if ui.button("Rescan devices").clicked() {
            self.refresh_devices(true);
        }
    }
}
