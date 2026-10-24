//! Backup list and restore.

use eframe::egui;

use super::app::{App, Dialog, PendingAction};

impl App {
    pub(crate) fn draw_backups(&mut self, ui: &mut egui::Ui) {
        ui.heading("Backups");
        ui.add_space(4.0);
        ui.label(format!(
            "Backups are written before the first frame of every apply, to {}.",
            self.paths.backups_dir().display()
        ));
        ui.horizontal(|ui| {
            if ui.button("Refresh").clicked() {
                self.backups = crate::settings::backup::list(&self.paths);
            }
            if ui.button("Copy folder path").clicked() {
                ui.ctx()
                    .copy_text(self.paths.backups_dir().display().to_string());
            }
            ui.weak(format!("{} backup(s)", self.backups.len()));
        });
        ui.add_space(8.0);

        if self.backups.is_empty() {
            ui.label("No backups yet. The first apply will create one.");
            return;
        }

        let mut restore: Option<usize> = None;
        egui::Grid::new("backup-list")
            .num_columns(4)
            .spacing([14.0, 6.0])
            .show(ui, |ui| {
                ui.label(egui::RichText::new("Created").strong());
                ui.label(egui::RichText::new("Descriptor").strong());
                ui.label(egui::RichText::new("Registers").strong());
                ui.label("");
                ui.end_row();
                for (index, info) in self.backups.iter().enumerate() {
                    ui.label(info.created_text());
                    ui.monospace(&info.descriptor_id);
                    ui.label(info.values.to_string());
                    if ui.button("Restore").clicked() {
                        restore = Some(index);
                    }
                    ui.end_row();
                }
            });

        if let Some(index) = restore {
            if let Some(info) = self.backups.get(index) {
                self.dialog = Some(Dialog::Confirm {
                    title: "Restore this backup?".into(),
                    body: format!(
                        "Backup from {} for `{}`.\n\n\
                         Every recorded register is written back through the same validate / \
                         apply / verify path, and a fresh backup of the current values is taken \
                         first.",
                        info.created_text(),
                        info.descriptor_id
                    ),
                    action: PendingAction::RestoreBackup(index),
                    destructive: true,
                });
            }
        }

        ui.add_space(10.0);
        ui.separator();
        ui.small(
            "Backups are never pruned automatically. Restoring a backup for a different model is \
             refused.",
        );
    }
}
