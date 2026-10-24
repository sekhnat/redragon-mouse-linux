//! Device information, access guidance, and capability summary.

use eframe::egui;

use super::app::App;

impl App {
    pub(crate) fn draw_device(&mut self, ui: &mut egui::Ui) {
        ui.heading("Device");
        ui.add_space(4.0);

        let Some(desc) = self.descriptor.clone() else {
            ui.label("No supported mouse is connected.");
            ui.label(
                "Connect a Redragon/Holtek mouse, or start with --simulate <model> to explore.",
            );
            if ui.button("Scan again").clicked() {
                self.refresh_devices(true);
            }
            ui.add_space(8.0);
            self.draw_detected(ui);
            return;
        };

        egui::Grid::new("device-info")
            .num_columns(2)
            .spacing([16.0, 6.0])
            .show(ui, |ui| {
                ui.label("Model");
                ui.label(egui::RichText::new(&desc.model).strong());
                ui.end_row();
                ui.label("Descriptor");
                ui.monospace(&desc.id);
                ui.end_row();
                ui.label("Provenance");
                ui.label(desc.provenance.label());
                ui.end_row();
                ui.label("Verification");
                if desc.verified {
                    ui.colored_label(egui::Color32::from_rgb(90, 170, 90), "verified");
                } else {
                    ui.colored_label(
                        egui::Color32::from_rgb(220, 160, 40),
                        "unverified — writing needs an extra confirmation",
                    );
                }
                ui.end_row();
                if let Some(det) = &self.device {
                    ui.label("Node");
                    ui.monospace(det.device.node.display().to_string());
                    ui.end_row();
                    ui.label("USB IDs");
                    ui.monospace(format!(
                        "{:04x}:{:04x}  bcdDevice {:#06x}  interface {}",
                        det.device.vendor_id,
                        det.device.product_id,
                        det.device.bcd_device,
                        det.device.interface_number
                    ));
                    ui.end_row();
                } else {
                    ui.label("Node");
                    ui.monospace("simulation (no hardware access)");
                    ui.end_row();
                }
                ui.label("Collection");
                ui.monospace(format!(
                    "usage page {:#06x} / usage {:#04x}",
                    desc.collections.usage_page, desc.collections.usage
                ));
                ui.end_row();
                ui.label("Frames");
                let frames: Vec<String> = desc
                    .frames
                    .iter()
                    .map(|frame| format!("id {}: {} B", frame.id, frame.len))
                    .collect();
                ui.monospace(frames.join(" · "));
                ui.end_row();
                ui.label("Profiles");
                ui.label(desc.profile_count().to_string());
                ui.end_row();
            });

        ui.add_space(8.0);
        ui.label(egui::RichText::new("Capabilities").strong());
        ui.horizontal_wrapped(|ui| {
            for capability in desc.capability_list() {
                ui.label(
                    egui::RichText::new(capability).background_color(ui.visuals().faint_bg_color),
                );
            }
        });

        if let Some(notes) = &desc.notes {
            ui.add_space(8.0);
            ui.collapsing("Descriptor notes", |ui| {
                ui.label(notes);
            });
        }

        if let Some(guidance) = &self.access {
            ui.add_space(10.0);
            ui.separator();
            ui.colored_label(
                egui::Color32::from_rgb(220, 100, 90),
                "The configuration interface is not accessible",
            );
            ui.label(&guidance.detail);
            ui.add_space(4.0);
            ui.label("Run these commands yourself (the tool never escalates privileges):");
            ui.add(
                egui::TextEdit::multiline(&mut guidance.instructions())
                    .desired_rows(6)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY),
            );
            if ui.button("Copy udev rule").clicked() {
                ui.ctx().copy_text(guidance.rule.clone());
            }
            if ui.button("Retry access").clicked() {
                self.refresh_devices(true);
            }
        }

        if let Some(report) = &self.last_report {
            ui.add_space(10.0);
            ui.separator();
            ui.label(egui::RichText::new("Last apply").strong());
            ui.label(format!(
                "{} register(s) changed, {} confirmed by read-back",
                report.changed, report.confirmed
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
                ui.weak(format!("Backup: {}", backup.display()));
            }
        }

        ui.add_space(10.0);
        self.draw_detected(ui);
    }

    fn draw_detected(&self, ui: &mut egui::Ui) {
        if self.detected.is_empty() {
            return;
        }
        ui.collapsing("Detected HID interfaces", |ui| {
            for det in &self.detected {
                let dev = &det.device;
                let state = match &det.identification {
                    crate::device::Identification::Supported {
                        descriptor_index, ..
                    } => self
                        .registry
                        .descriptor(*descriptor_index)
                        .map(|desc| format!("{} ({})", desc.model, desc.id))
                        .unwrap_or_else(|| "supported".into()),
                    crate::device::Identification::Unsupported => {
                        "unsupported: no model descriptor".into()
                    }
                    crate::device::Identification::Conflict(ids) => {
                        format!("descriptor conflict: {}", ids.join(", "))
                    }
                };
                ui.monospace(format!(
                    "{}  {:04x}:{:04x}  iface {}  bcdDevice {:#06x}  — {state}",
                    dev.node.display(),
                    dev.vendor_id,
                    dev.product_id,
                    dev.interface_number,
                    dev.bcd_device
                ));
            }
        });
    }
}
