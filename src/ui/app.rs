//! Application shell: worker thread, device selection, dialogs, and section routing.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui;

use crate::device::{self, AccessGuidance, DetectedDevice, HotplugMonitor, Identification};
use crate::models::{Descriptor, Registry};
use crate::paths::{LogFile, Paths};
use crate::settings::{
    apply, apply_backup, backup, diff, encode_ops, read_state_consistent, seed_image,
    validate_state, ApplyError, ApplyReport, ApplySettings, BackupFile, BackupInfo, DeviceIds,
    DeviceState, FieldError, MacroNames, ReadImage, WriteOp,
};
use crate::transport::hidraw::HidrawTransport;
use crate::transport::sim::SimTransport;
use crate::transport::{Pacer, Transport};

#[derive(Clone, Debug, Default)]
pub struct RunOptions {
    pub simulation: Option<String>,
    pub models_dir: Option<PathBuf>,
    pub log_path: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Device,
    Pointer,
    Lighting,
    Profiles,
    Buttons,
    Macros,
    Backups,
    Diagnostics,
}

impl Section {
    const ALL: [Section; 8] = [
        Section::Device,
        Section::Pointer,
        Section::Lighting,
        Section::Profiles,
        Section::Buttons,
        Section::Macros,
        Section::Backups,
        Section::Diagnostics,
    ];

    fn label(self) -> &'static str {
        match self {
            Section::Device => "Device",
            Section::Pointer => "Pointer",
            Section::Lighting => "Lighting",
            Section::Profiles => "Profiles",
            Section::Buttons => "Buttons",
            Section::Macros => "Macros",
            Section::Backups => "Backups",
            Section::Diagnostics => "Diagnostics",
        }
    }
}

pub(crate) enum Mode {
    Hardware,
    Simulation { model: String },
}

pub(crate) enum Request {
    Read,
    Apply {
        baseline: Box<DeviceState>,
        draft: Box<DeviceState>,
        force_stale: bool,
        active_profile: u8,
    },
    Restore {
        current: Box<DeviceState>,
        backup: Box<BackupFile>,
        active_profile: u8,
    },
    Activate {
        profile: u8,
    },
}

pub(crate) enum Response {
    Read(Result<DeviceState, String>),
    Applied(Box<Result<ApplyReport, ApplyError>>),
}

pub(crate) struct Worker {
    tx: Sender<Request>,
    rx: Receiver<Response>,
}

impl Worker {
    fn spawn_hardware(
        device: device::HidrawDevice,
        desc: Descriptor,
        paths: Paths,
        ids: DeviceIds,
    ) -> Worker {
        let (tx, request_rx) = mpsc::channel::<Request>();
        let (response_tx, rx) = mpsc::channel::<Response>();
        thread::spawn(move || {
            let pacer = Pacer::production();
            let mut transport: Option<Box<dyn Transport>> = None;
            while let Ok(request) = request_rx.recv() {
                let result = handle_request(
                    &mut transport,
                    &device,
                    &desc,
                    &paths,
                    ids,
                    &pacer,
                    request,
                    false,
                );
                if response_tx.send(result).is_err() {
                    break;
                }
            }
        });
        Worker { tx, rx }
    }

    fn spawn_simulation(desc: Descriptor, paths: Paths, ids: DeviceIds) -> Worker {
        let (tx, request_rx) = mpsc::channel::<Request>();
        let (response_tx, rx) = mpsc::channel::<Response>();
        thread::spawn(move || {
            let pacer = Pacer::instant();
            let mut sim = SimTransport::new(desc.frame_sizes());
            install_image(&mut sim, &seed_image(&desc));
            let mut transport: Option<Box<dyn Transport>> = Some(Box::new(sim));
            while let Ok(request) = request_rx.recv() {
                let result = handle_request(
                    &mut transport,
                    &fake_device(&desc),
                    &desc,
                    &paths,
                    ids,
                    &pacer,
                    request,
                    true,
                );
                if response_tx.send(result).is_err() {
                    break;
                }
            }
        });
        Worker { tx, rx }
    }

    fn send(&self, request: Request) {
        let _ = self.tx.send(request);
    }

    fn try_recv(&self) -> Option<Response> {
        self.rx.try_recv().ok()
    }
}

fn fake_device(desc: &Descriptor) -> device::HidrawDevice {
    device::HidrawDevice {
        node: PathBuf::from("simulation"),
        sysfs: PathBuf::new(),
        vendor_id: 0xffff,
        product_id: 0x0000,
        bcd_device: 0,
        interface_number: 2,
        interface_class: 3,
        interface_subclass: 0,
        interface_protocol: 0,
        name: desc.model.clone(),
        bus: 0,
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_request(
    transport: &mut Option<Box<dyn Transport>>,
    device: &device::HidrawDevice,
    desc: &Descriptor,
    paths: &Paths,
    ids: DeviceIds,
    pacer: &Pacer,
    request: Request,
    simulated: bool,
) -> Response {
    if transport.is_none() {
        match HidrawTransport::open(&device.node) {
            Ok(opened) => *transport = Some(Box::new(opened)),
            Err(err) => {
                let access = device::inspect(&device.node).ok();
                let guidance = device::guidance(device, access.as_ref(), &err.to_string());
                return Response::Read(Err(format!(
                    "{}\n\n{}",
                    guidance.detail,
                    guidance.instructions()
                )));
            }
        }
    }
    match request {
        Request::Read => {
            let transport = transport.as_mut().expect("open above");
            let result = read_state_consistent(transport.as_mut(), desc, pacer)
                .map_err(|err| err.to_string());
            Response::Read(result)
        }
        Request::Apply {
            baseline,
            draft,
            force_stale,
            active_profile,
        } => {
            let settings = ApplySettings {
                pacer: *pacer,
                force_stale,
                active_profile,
                label: if simulated {
                    "simulated apply".into()
                } else {
                    format!("apply on {}", device.node.display())
                },
            };
            let transport = transport.as_mut().expect("open above");
            let result = apply(
                transport.as_mut(),
                desc,
                &baseline,
                &draft,
                ids,
                paths,
                &settings,
            );
            Response::Applied(Box::new(result))
        }
        Request::Activate { profile } => {
            let transport = transport.as_mut().expect("open above");
            match crate::settings::activate_profile(transport.as_mut(), desc, profile, pacer) {
                Ok(()) => {
                    let result = read_state_consistent(transport.as_mut(), desc, pacer)
                        .map_err(|err| err.to_string());
                    Response::Read(result)
                }
                Err(err) => Response::Read(Err(err.to_string())),
            }
        }
        Request::Restore {
            current,
            backup,
            active_profile,
        } => {
            let settings = ApplySettings {
                pacer: *pacer,
                force_stale: true,
                active_profile,
                label: format!("restore backup {}", backup.created_text()),
            };
            let transport = transport.as_mut().expect("open above");
            let result = apply_backup(
                transport.as_mut(),
                desc,
                &current,
                &backup,
                ids,
                paths,
                &settings,
            );
            Response::Applied(Box::new(result))
        }
    }
}

fn install_image(sim: &mut SimTransport, image: &ReadImage) {
    for (_, window) in image.iter() {
        sim.set_bytes(window.addr, &window.bytes);
    }
}

#[derive(Clone, Debug)]
pub(crate) struct StatusMessage {
    pub text: String,
    pub error: bool,
}

#[derive(Clone, Debug)]
pub(crate) enum PendingAction {
    ActivateProfile(u8),
    CopyProfile {
        from: u8,
        to: u8,
    },
    ResetProfile(u8),
    ResetButtons,
    DeleteMacro(u8),
    RestoreBackup(usize),
    ReassignPrimary {
        profile: u8,
        button: usize,
        action: String,
    },
    OverwriteExport(PathBuf),
    OverwriteMacroExport(PathBuf),
}

#[derive(Clone, Debug)]
pub(crate) enum Dialog {
    ApplySummary {
        changes: Vec<diff::OpChange>,
        unverified_ack: bool,
    },
    Stale {
        differences: Vec<String>,
    },
    Validation(Vec<FieldError>),
    Confirm {
        title: String,
        body: String,
        action: PendingAction,
        destructive: bool,
    },
    Error {
        title: String,
        body: String,
    },
    Notice {
        title: String,
        lines: Vec<String>,
    },
}

pub(crate) struct App {
    pub(crate) paths: Paths,
    pub(crate) registry: Registry,
    pub(crate) log_path: PathBuf,
    pub(crate) mode: Mode,
    pub(crate) detected: Vec<DetectedDevice>,
    pub(crate) device: Option<DetectedDevice>,
    pub(crate) descriptor: Option<Descriptor>,
    pub(crate) baseline: Option<DeviceState>,
    pub(crate) draft: Option<DeviceState>,
    pub(crate) selected_profile: u8,
    pub(crate) section: Section,
    pub(crate) busy: bool,
    pub(crate) status: Option<StatusMessage>,
    pub(crate) dialog: Option<Dialog>,
    pub(crate) last_report: Option<ApplyReport>,
    pub(crate) macro_names: MacroNames,
    pub(crate) backups: Vec<BackupInfo>,
    pub(crate) worker: Option<Worker>,
    pub(crate) hotplug: Option<HotplugMonitor>,
    last_hotplug: Instant,
    pub(crate) access: Option<AccessGuidance>,
    pub(crate) import_path: String,
    pub(crate) export_path: String,
    pub(crate) unverified_acknowledged: bool,
    pub(crate) allow_close: bool,
    pub(crate) show_close_prompt: bool,
    pub(crate) copy_from: u8,
    pub(crate) copy_to: u8,
    pub(crate) selected_macro: u8,
    pub(crate) macro_key: String,
    pub(crate) macro_delay_ms: u16,
    pub(crate) macro_import_path: String,
    pub(crate) macro_export_path: String,
    pub(crate) names_dirty: bool,
    screenshot_taken: bool,
    pub(crate) reconnect_pending: Option<DeviceState>,
    pub(crate) expect_profile: Option<u8>,
}

pub fn run(options: RunOptions) -> Result<(), String> {
    let paths = Paths::from_env();
    let models_dir = options
        .models_dir
        .clone()
        .unwrap_or_else(|| paths.models_dir());
    let registry = Registry::load(&models_dir);
    if registry.descriptors.is_empty() {
        return Err(format!(
            "no usable model descriptors in {}\n{}",
            models_dir.display(),
            registry
                .errors
                .iter()
                .map(|error| format!("  {}: {}", error.file.display(), error.message))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    let log_path = options
        .log_path
        .clone()
        .unwrap_or_else(|| match LogFile::create(&paths) {
            Ok(log) => log.path().to_path_buf(),
            Err(err) => {
                tracing::warn!(error = %err, "no session log available");
                PathBuf::new()
            }
        });
    let mut app = App::new(paths, registry, log_path, options.simulation.clone());
    if let Some(model) = options.simulation {
        app.start_simulation(&model)?;
    } else {
        app.refresh_devices(true);
    }

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1120.0, 780.0])
            .with_min_inner_size([860.0, 560.0])
            .with_title("Redragon Mouse"),
        ..Default::default()
    };
    eframe::run_native(
        "Redragon Mouse",
        native_options,
        Box::new(move |cc| {
            cc.egui_ctx.set_theme(egui::ThemePreference::System);
            // Test aids for visual checks; the defaults follow the desktop.
            match std::env::var("REDRAGON_GUI_THEME").as_deref() {
                Ok("light") => cc.egui_ctx.set_theme(egui::ThemePreference::Light),
                Ok("dark") => cc.egui_ctx.set_theme(egui::ThemePreference::Dark),
                _ => {}
            }
            if let Ok(scale) = std::env::var("REDRAGON_GUI_SCALE") {
                if let Ok(scale) = scale.parse::<f32>() {
                    cc.egui_ctx.set_zoom_factor(scale.clamp(0.5, 4.0));
                }
            }
            Ok(Box::new(app))
        }),
    )
    .map_err(|err| err.to_string())
}

impl App {
    pub(crate) fn new(
        paths: Paths,
        registry: Registry,
        log_path: PathBuf,
        simulation: Option<String>,
    ) -> Self {
        App {
            macro_names: backup::load_names(&paths),
            backups: backup::list(&paths),
            paths,
            registry,
            log_path,
            mode: match simulation {
                Some(model) => Mode::Simulation { model },
                None => Mode::Hardware,
            },
            detected: Vec::new(),
            device: None,
            descriptor: None,
            baseline: None,
            draft: None,
            selected_profile: 0,
            section: Section::Device,
            busy: false,
            status: None,
            dialog: None,
            last_report: None,
            worker: None,
            hotplug: None,
            last_hotplug: Instant::now(),
            access: None,
            import_path: String::new(),
            export_path: String::new(),
            unverified_acknowledged: false,
            allow_close: false,
            show_close_prompt: false,
            copy_from: 0,
            copy_to: 1,
            selected_macro: 0,
            macro_key: String::new(),
            macro_delay_ms: 50,
            macro_import_path: String::new(),
            macro_export_path: String::new(),
            names_dirty: false,
            screenshot_taken: false,
            reconnect_pending: None,
            expect_profile: None,
        }
    }

    fn is_simulated(&self) -> bool {
        matches!(self.mode, Mode::Simulation { .. })
    }

    pub(crate) fn start_simulation(&mut self, model: &str) -> Result<(), String> {
        let descriptor = self.resolve_model(model)?;
        let ids = DeviceIds {
            vendor_id: 0xffff,
            product_id: 0,
        };
        self.worker = Some(Worker::spawn_simulation(
            descriptor.clone(),
            self.paths.clone(),
            ids,
        ));
        self.descriptor = Some(descriptor);
        self.device = None;
        self.busy = true;
        self.status = Some(StatusMessage {
            text: format!("Reading simulated device `{model}`…"),
            error: false,
        });
        if let Some(worker) = &self.worker {
            worker.send(Request::Read);
        }
        Ok(())
    }

    fn resolve_model(&self, needle: &str) -> Result<Descriptor, String> {
        self.registry
            .descriptors
            .iter()
            .find(|desc| desc.id == needle)
            .or_else(|| {
                self.registry
                    .descriptors
                    .iter()
                    .find(|desc| desc.model.to_lowercase().contains(&needle.to_lowercase()))
            })
            .cloned()
            .ok_or_else(|| format!("no descriptor matches `{needle}`"))
    }

    pub(crate) fn refresh_devices(&mut self, announce: bool) {
        if self.is_simulated() {
            return;
        }
        let base = device::enumerate::default_base();
        match device::scan_and_identify(&self.registry, &base) {
            Ok(detected) => {
                self.registry_warnings();
                self.detected = detected;
                if let Some(current) = &self.device {
                    let still_there = self
                        .detected
                        .iter()
                        .any(|det| det.device.node == current.device.node);
                    if !still_there {
                        self.on_device_removed();
                    }
                }
                if self.device.is_none() {
                    if let Some(found) =
                        self.detected.iter().find(|det| det.is_supported()).cloned()
                    {
                        self.connect(found, announce);
                    } else if announce {
                        self.status = Some(StatusMessage {
                            text: "No supported mouse found. Waiting for one to be connected."
                                .into(),
                            error: false,
                        });
                    }
                }
            }
            Err(err) => {
                self.status = Some(StatusMessage {
                    text: format!("Cannot scan {}: {err}", base.display()),
                    error: true,
                });
            }
        }
        if self.hotplug.is_none() {
            self.hotplug = HotplugMonitor::new(base).ok();
        }
    }

    fn registry_warnings(&mut self) {
        if !self.registry.errors.is_empty() {
            let lines: Vec<String> = self
                .registry
                .errors
                .iter()
                .map(|error| format!("{}: {}", error.file.display(), error.message))
                .collect();
            self.status = Some(StatusMessage {
                text: format!("{} descriptor problem(s) — see Diagnostics", lines.len()),
                error: true,
            });
        }
    }

    pub(crate) fn connect(&mut self, det: DetectedDevice, announce: bool) {
        let Identification::Supported {
            descriptor_index, ..
        } = det.identification
        else {
            return;
        };
        let Some(descriptor) = self.registry.descriptor(descriptor_index).cloned() else {
            return;
        };
        let ids = DeviceIds {
            vendor_id: det.device.vendor_id,
            product_id: det.device.product_id,
        };
        self.access = None;
        match device::validate_interface(&det.device, &descriptor) {
            Ok(_) => {}
            Err(err) if matches!(err, device::InterfaceError::Open { .. }) => {
                let access = device::inspect(&det.device.node).ok();
                self.access = Some(device::guidance(
                    &det.device,
                    access.as_ref(),
                    &err.to_string(),
                ));
                self.status = Some(StatusMessage {
                    text: format!("Cannot open {}: {err}", det.device.node.display()),
                    error: true,
                });
            }
            Err(err) => {
                self.status = Some(StatusMessage {
                    text: format!("{} is not usable: {err}", det.device.node.display()),
                    error: true,
                });
                return;
            }
        }
        if self.device.is_none() && self.draft.is_some() && self.is_dirty() {
            self.reconnect_pending = self.draft.clone();
        }
        self.worker = Some(Worker::spawn_hardware(
            det.device.clone(),
            descriptor.clone(),
            self.paths.clone(),
            ids,
        ));
        self.descriptor = Some(descriptor);
        self.baseline = None;
        self.draft = None;
        self.unverified_acknowledged = false;
        self.device = Some(det);
        self.busy = true;
        if announce {
            self.status = Some(StatusMessage {
                text: "Reading the mouse's current settings…".into(),
                error: false,
            });
        }
        if let Some(worker) = &self.worker {
            worker.send(Request::Read);
        }
    }

    fn on_device_removed(&mut self) {
        self.device = None;
        self.worker = None;
        self.status = Some(StatusMessage {
            text: "The mouse was disconnected. Pending edits are kept; reconnect it to apply."
                .into(),
            error: true,
        });
        self.busy = false;
    }

    fn poll_hotplug(&mut self, ctx: &egui::Context) {
        if self.is_simulated() {
            return;
        }
        if self.last_hotplug.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.last_hotplug = Instant::now();
        if let Some(monitor) = &mut self.hotplug {
            match monitor.poll() {
                Ok(event) => {
                    if !event.is_empty() {
                        tracing::info!(
                            arrived = event.arrived.len(),
                            removed = event.removed.len(),
                            "hotplug event"
                        );
                        self.refresh_devices(true);
                    }
                }
                Err(err) => tracing::debug!(error = %err, "hotplug poll failed"),
            }
        } else {
            self.refresh_devices(false);
        }
        ctx.request_repaint_after(Duration::from_secs(2));
    }

    fn poll_worker(&mut self) {
        let mut pending = Vec::new();
        if let Some(worker) = &self.worker {
            while let Some(response) = worker.try_recv() {
                pending.push(response);
            }
        }
        for response in pending {
            match response {
                Response::Read(result) => match result {
                    Ok(state) => {
                        self.busy = false;
                        self.install_read(state);
                    }
                    Err(err) => {
                        self.busy = false;
                        self.status = Some(StatusMessage {
                            text: format!("Cannot read settings: {err}"),
                            error: true,
                        });
                        if let Some(det) = &self.device {
                            let access = device::inspect(&det.device.node).ok();
                            self.access =
                                Some(device::guidance(&det.device, access.as_ref(), &err));
                        }
                    }
                },
                Response::Applied(result) => {
                    self.busy = false;
                    match *result {
                        Ok(report) => {
                            self.last_report = Some(report.clone());
                            if report.fully_confirmed() {
                                self.status = Some(StatusMessage {
                                    text: format!(
                                        "Applied {} change(s); all read back as written.",
                                        report.changed
                                    ),
                                    error: false,
                                });
                            } else {
                                self.status = Some(StatusMessage {
                                    text: format!(
                                        "Applied {} change(s); {} not confirmed.",
                                        report.changed,
                                        report.mismatches.len()
                                    ),
                                    error: true,
                                });
                            }
                            self.backups = backup::list(&self.paths);
                            // Refresh so normalised values are shown.
                            if let Some(worker) = &self.worker {
                                self.busy = true;
                                worker.send(Request::Read);
                            }
                        }
                        Err(err) => {
                            if let Some(dialog) = self.dialog_from_apply_error(err) {
                                self.dialog = Some(dialog);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Install a device read, restoring pending edits made before a reconnect.
    pub(crate) fn install_read(&mut self, mut state: DeviceState) {
        backup::apply_names(
            &mut state,
            &self.macro_names.names_for(&self.descriptor_id()),
        );
        // Follow the mouse's active profile on the first read and after an
        // activation only; a read after an apply must not pull the editor away
        // from the stored profile that was being edited.
        let first_read = self.baseline.is_none();
        let expected = self.expect_profile.take();
        if first_read || expected.is_some() {
            if let Some(descriptor) = &self.descriptor {
                if state.active_profile < descriptor.profile_count() {
                    self.selected_profile = state.active_profile;
                }
            }
        }
        self.baseline = Some(state.clone());

        let reconnected = self.reconnect_pending.take();
        match reconnected {
            Some(pending) => {
                self.draft = Some(pending);
                self.status = Some(StatusMessage {
                    text: "Pending edits from before the disconnect were kept. Review them                            before applying."
                        .into(),
                    error: false,
                });
            }
            None => {
                self.draft = Some(state.clone());
                let name = self
                    .descriptor
                    .as_ref()
                    .map(|desc| desc.model.clone())
                    .unwrap_or_default();
                self.status = Some(StatusMessage {
                    text: format!("Read settings from {name}."),
                    error: false,
                });
            }
        }

        if let Some(slot) = expected {
            if state.active_profile == slot {
                self.status = Some(StatusMessage {
                    text: format!(
                        "Profile {} is now active (confirmed by read-back).",
                        slot + 1
                    ),
                    error: false,
                });
            } else {
                self.status = Some(StatusMessage {
                    text: format!(
                        "The profile switch was not confirmed: the mouse still reports profile {}.",
                        state.active_profile + 1
                    ),
                    error: true,
                });
            }
        }
    }

    fn dialog_from_apply_error(&mut self, err: ApplyError) -> Option<Dialog> {
        match err {
            ApplyError::Validation(errors) => Some(Dialog::Validation(errors)),
            ApplyError::Stale { differences } => Some(Dialog::Stale { differences }),
            ApplyError::Transport { source, rollback } => {
                let mut body = source.to_string();
                if let Some(rollback) = rollback {
                    body.push_str(&format!(
                        "\n\nRollback: attempted {}, restored {}, failures: {}",
                        rollback.attempted,
                        rollback.restored,
                        if rollback.failures.is_empty() {
                            "none".to_string()
                        } else {
                            rollback.failures.join("; ")
                        }
                    ));
                }
                Some(Dialog::Error {
                    title: "Writing to the mouse failed".into(),
                    body,
                })
            }
            ApplyError::Backup(io) => Some(Dialog::Error {
                title: "Backup could not be written".into(),
                body: format!("{io}\n\nNothing was sent to the mouse."),
            }),
            ApplyError::NoChanges => Some(Dialog::Notice {
                title: "Nothing to apply".into(),
                lines: vec!["No setting differs from the device state.".into()],
            }),
            other => Some(Dialog::Error {
                title: "Apply failed".into(),
                body: other.to_string(),
            }),
        }
    }

    pub(crate) fn descriptor_id(&self) -> String {
        self.descriptor
            .as_ref()
            .map(|desc| desc.id.clone())
            .unwrap_or_default()
    }

    pub(crate) fn baseline_ops(&self) -> Vec<WriteOp> {
        match (&self.descriptor, &self.baseline) {
            (Some(desc), Some(state)) => encode_ops(desc, state),
            _ => Vec::new(),
        }
    }

    pub(crate) fn draft_ops(&self) -> Vec<WriteOp> {
        match (&self.descriptor, &self.draft) {
            (Some(desc), Some(state)) => encode_ops(desc, state),
            _ => Vec::new(),
        }
    }

    /// One sentence per pending edit (see `ui::summary`).
    pub(crate) fn change_lines(&self) -> Vec<String> {
        match (&self.descriptor, &self.baseline, &self.draft) {
            (Some(desc), Some(baseline), Some(draft)) => {
                super::summary::describe_changes(desc, baseline, draft)
            }
            _ => Vec::new(),
        }
    }

    pub(crate) fn changes(&self) -> Vec<diff::OpChange> {
        diff::summary(&self.baseline_ops(), &self.draft_ops())
    }

    pub(crate) fn validation_errors(&self) -> Vec<FieldError> {
        match (&self.descriptor, &self.draft) {
            (Some(desc), Some(state)) => validate_state(desc, state),
            _ => Vec::new(),
        }
    }

    pub(crate) fn is_dirty(&self) -> bool {
        !self.changes().is_empty()
    }

    /// Why the Apply button is unavailable, if it is.
    pub(crate) fn apply_gate(&self) -> Option<String> {
        if self.busy {
            return Some("Waiting for the device…".into());
        }
        if self.descriptor.is_none() || self.draft.is_none() {
            return Some("No settings read yet.".into());
        }
        let errors = self.validation_errors();
        if !errors.is_empty() {
            return Some(format!(
                "{} setting(s) need attention: {}",
                errors.len(),
                errors[0].message
            ));
        }
        if !self.is_dirty() {
            return Some("No changes to apply.".into());
        }
        None
    }

    pub(crate) fn request_apply(&mut self, force_stale: bool) {
        let (Some(baseline), Some(draft), Some(worker)) =
            (&self.baseline, &self.draft, &self.worker)
        else {
            return;
        };
        self.busy = true;
        self.dialog = None;
        self.status = Some(StatusMessage {
            text: "Writing settings and reading them back…".into(),
            error: false,
        });
        worker.send(Request::Apply {
            baseline: Box::new(baseline.clone()),
            draft: Box::new(draft.clone()),
            force_stale,
            active_profile: draft.active_profile,
        });
    }

    pub(crate) fn revert(&mut self) {
        if let Some(baseline) = &self.baseline {
            self.draft = Some(baseline.clone());
            self.status = Some(StatusMessage {
                text: "Pending edits discarded.".into(),
                error: false,
            });
        }
    }

    pub(crate) fn begin_apply_dialog(&mut self) {
        let changes = self.changes();
        if changes.is_empty() {
            return;
        }
        let errors = self.validation_errors();
        if !errors.is_empty() {
            self.dialog = Some(Dialog::Validation(errors));
            return;
        }
        self.dialog = Some(Dialog::ApplySummary {
            changes,
            unverified_ack: false,
        });
    }

    pub(crate) fn needs_unverified_ack(&self) -> bool {
        self.descriptor
            .as_ref()
            .map(|desc| !desc.verified && !self.unverified_acknowledged)
            .unwrap_or(false)
    }

    pub(crate) fn save_names(&self) {
        let Some(draft) = &self.draft else {
            return;
        };
        let names = backup::capture_names(draft);
        let mut store = self.macro_names.clone();
        store.set(&self.descriptor_id(), names);
        if let Err(err) = backup::save_names(&self.paths, &store) {
            tracing::warn!(error = %err, "cannot save macro names");
        }
    }

    pub(crate) fn perform(&mut self, action: PendingAction) {
        match action {
            PendingAction::ActivateProfile(slot) => {
                if self.is_dirty() {
                    self.dialog = Some(Dialog::Notice {
                        title: "Pending edits".into(),
                        lines: vec![
                            "Apply or discard the pending edits before switching profiles.".into(),
                        ],
                    });
                    return;
                }
                if let Some(worker) = &self.worker {
                    self.busy = true;
                    self.expect_profile = Some(slot);
                    self.status = Some(StatusMessage {
                        text: format!("Activating profile {}…", slot + 1),
                        error: false,
                    });
                    worker.send(Request::Activate { profile: slot });
                }
            }
            PendingAction::CopyProfile { from, to } => {
                if let Some(draft) = &mut self.draft {
                    if let (Some(source), Some(_)) = (
                        draft.profiles.get(from as usize).cloned(),
                        draft.profiles.get(to as usize),
                    ) {
                        if let Some(dest) = draft.profiles.get_mut(to as usize) {
                            *dest = source;
                        }
                        self.status = Some(StatusMessage {
                            text: format!("Copied profile {} onto profile {}.", from + 1, to + 1),
                            error: false,
                        });
                    }
                }
            }
            PendingAction::ResetProfile(slot) => {
                if let Some(desc) = &self.descriptor {
                    let fresh = crate::settings::state::seed_state(desc);
                    if let (Some(draft), Some(default)) =
                        (&mut self.draft, fresh.profiles.get(slot as usize).cloned())
                    {
                        if let Some(target) = draft.profiles.get_mut(slot as usize) {
                            let zone_count = default
                                .lighting
                                .as_ref()
                                .map(|lighting| lighting.zone_colors.len())
                                .unwrap_or(0);
                            *target = default;
                            if let Some(lighting) = target.lighting.as_mut() {
                                lighting.zone_colors = vec![[0xFF, 0xFF, 0xFF]; zone_count];
                            }
                        }
                    }
                }
            }
            PendingAction::ResetButtons => {
                if let (Some(desc), Some(draft)) = (&self.descriptor, &mut self.draft) {
                    let fresh = crate::settings::state::seed_state(desc);
                    let profile = self.selected_profile as usize;
                    if let (Some(target), Some(default)) =
                        (draft.profiles.get_mut(profile), fresh.profiles.get(profile))
                    {
                        target.buttons = default.buttons.clone();
                    }
                }
            }
            PendingAction::DeleteMacro(slot) => {
                if let Some(draft) = &mut self.draft {
                    if let Some(macro_state) = draft.macros.iter_mut().find(|m| m.slot == slot) {
                        macro_state.actions.clear();
                        macro_state.name.clear();
                        macro_state.burst = None;
                    }
                }
            }
            PendingAction::RestoreBackup(index) => {
                let Some(info) = self.backups.get(index).cloned() else {
                    return;
                };
                match backup::load(&info.path) {
                    Ok(file) => {
                        if let (Some(worker), Some(baseline)) = (&self.worker, &self.baseline) {
                            let active = self.draft.as_ref().map(|d| d.active_profile).unwrap_or(0);
                            self.busy = true;
                            self.status = Some(StatusMessage {
                                text: format!("Restoring backup from {}…", info.created_text()),
                                error: false,
                            });
                            worker.send(Request::Restore {
                                current: Box::new(baseline.clone()),
                                backup: Box::new(file),
                                active_profile: active,
                            });
                        }
                    }
                    Err(err) => {
                        self.dialog = Some(Dialog::Error {
                            title: "Cannot read backup".into(),
                            body: err.to_string(),
                        });
                    }
                }
            }
            PendingAction::ReassignPrimary {
                profile,
                button,
                action,
            } => {
                if let Some(draft) = &mut self.draft {
                    if let Some(bindings) = draft
                        .profiles
                        .get_mut(profile as usize)
                        .and_then(|state| state.buttons.as_mut())
                    {
                        if let Some(binding) = bindings.get_mut(button) {
                            binding.action = action;
                        }
                    }
                }
            }
            PendingAction::OverwriteExport(path) => {
                self.write_export(&path);
            }
            PendingAction::OverwriteMacroExport(path) => {
                self.write_macro_export(&path);
            }
        }
    }

    fn write_export(&mut self, path: &std::path::Path) {
        let (Some(desc), Some(draft)) = (&self.descriptor, &self.draft) else {
            return;
        };
        let profile = self.selected_profile as usize;
        let Some(state) = draft.profiles.get(profile) else {
            return;
        };
        let text = crate::settings::export_profile(desc, state, &draft.device);
        match std::fs::write(path, text) {
            Ok(()) => {
                self.status = Some(StatusMessage {
                    text: format!("Exported profile {} to {}.", profile + 1, path.display()),
                    error: false,
                });
            }
            Err(err) => {
                self.dialog = Some(Dialog::Error {
                    title: "Export failed".into(),
                    body: err.to_string(),
                });
            }
        }
    }

    pub(crate) fn request_export(&mut self) {
        let Some(state) = &self.draft else { return };
        let Some(profile) = state.profiles.get(self.selected_profile as usize) else {
            return;
        };
        let _ = profile;
        let path_text = if self.export_path.trim().is_empty() {
            format!("profile-{}.prof", self.selected_profile + 1)
        } else {
            self.export_path.clone()
        };
        let path = PathBuf::from(&path_text);
        if path.exists() {
            self.dialog = Some(Dialog::Confirm {
                title: "Overwrite profile file?".into(),
                body: format!(
                    "{} already exists. Its current contents will be replaced.",
                    path.display()
                ),
                action: PendingAction::OverwriteExport(path),
                destructive: true,
            });
        } else {
            self.write_export(&path);
        }
    }

    pub(crate) fn import_profile_file(&mut self) {
        let path = PathBuf::from(self.import_path.trim());
        if path.as_os_str().is_empty() {
            self.dialog = Some(Dialog::Notice {
                title: "Import".into(),
                lines: vec!["Enter the path of a .prof file first.".into()],
            });
            return;
        }
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) => {
                self.dialog = Some(Dialog::Error {
                    title: "Cannot read profile file".into(),
                    body: format!("{}: {err}", path.display()),
                });
                return;
            }
        };
        let (Some(desc), Some(draft)) = (&self.descriptor, &self.draft) else {
            return;
        };
        let profile = self.selected_profile as usize;
        let Some(base) = draft.profiles.get(profile).cloned() else {
            return;
        };
        let base_device = draft.device.clone();
        match crate::settings::import_profile(desc, &text, &base, &base_device) {
            Ok(result) => {
                if let Some(draft) = &mut self.draft {
                    if let Some(target) = draft.profiles.get_mut(profile) {
                        *target = result.profile;
                    }
                    draft.device = result.device;
                }
                let mut lines = vec![format!(
                    "Imported {} into profile {}.",
                    path.display(),
                    profile + 1
                )];
                if !result.warnings.is_empty() {
                    lines.push(String::new());
                    lines.push("Warnings:".into());
                    lines.extend(result.warnings);
                }
                lines.push(String::new());
                lines.push("Review the change set before applying.".into());
                self.dialog = Some(Dialog::Notice {
                    title: "Profile imported".into(),
                    lines,
                });
            }
            Err(errors) => {
                self.dialog = Some(Dialog::Validation(errors));
            }
        }
    }

    pub(crate) fn import_macro_file(&mut self) {
        let path = PathBuf::from(self.macro_import_path.trim());
        if path.as_os_str().is_empty() {
            self.dialog = Some(Dialog::Notice {
                title: "Import macro".into(),
                lines: vec!["Enter the path of a macro .json file first.".into()],
            });
            return;
        }
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) => {
                self.dialog = Some(Dialog::Error {
                    title: "Cannot read macro file".into(),
                    body: format!("{}: {err}", path.display()),
                });
                return;
            }
        };
        let Some(desc) = self.descriptor.clone() else {
            return;
        };
        match crate::settings::import_macro(&desc, &text) {
            Ok(file) => {
                let slot = self.selected_macro;
                let Some(draft) = self.draft.as_mut() else {
                    return;
                };
                if let Some(macro_state) = draft.macros.iter_mut().find(|m| m.slot == slot) {
                    macro_state.name = file.name.clone();
                    macro_state.actions = file.actions.clone();
                    macro_state.burst = file.burst.clone();
                }
                self.names_dirty = true;
                self.dialog = Some(Dialog::Notice {
                    title: "Macro imported".into(),
                    lines: vec![
                        format!("Imported `{}` into slot {}.", file.name, slot as usize + 1),
                        "Review the action list and apply when ready.".into(),
                    ],
                });
            }
            Err(errors) => {
                self.dialog = Some(Dialog::Validation(errors));
            }
        }
    }

    fn write_macro_export(&mut self, path: &std::path::Path) {
        let Some(draft) = &self.draft else {
            return;
        };
        let slot = self.selected_macro;
        let Some(macro_state) = draft.macros.iter().find(|m| m.slot == slot) else {
            return;
        };
        let text = crate::settings::export_macro(macro_state);
        match std::fs::write(path, text) {
            Ok(()) => {
                self.status = Some(StatusMessage {
                    text: format!(
                        "Exported macro {} to {}.",
                        slot as usize + 1,
                        path.display()
                    ),
                    error: false,
                });
            }
            Err(err) => {
                self.dialog = Some(Dialog::Error {
                    title: "Export failed".into(),
                    body: err.to_string(),
                });
            }
        }
    }

    pub(crate) fn request_macro_export(&mut self) {
        let name = self
            .draft
            .as_ref()
            .and_then(|draft| draft.macros.iter().find(|m| m.slot == self.selected_macro))
            .map(|m| m.name.clone())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| format!("macro-{}", self.selected_macro as usize + 1));
        let path_text = if self.macro_export_path.trim().is_empty() {
            format!("{name}.json")
        } else {
            self.macro_export_path.clone()
        };
        let path = PathBuf::from(&path_text);
        if path.exists() {
            self.dialog = Some(Dialog::Confirm {
                title: "Overwrite macro file?".into(),
                body: format!(
                    "{} already exists. Its current contents will be replaced.",
                    path.display()
                ),
                action: PendingAction::OverwriteMacroExport(path),
                destructive: true,
            });
        } else {
            self.write_macro_export(&path);
        }
    }

    pub(crate) fn delete_macro(&mut self, slot: u8) {
        let bound = self
            .draft
            .as_ref()
            .map(|draft| {
                draft.profiles.iter().any(|profile| {
                    profile.buttons.as_ref().is_some_and(|buttons| {
                        buttons
                            .iter()
                            .any(|binding| binding.macro_slot == Some(slot))
                    })
                })
            })
            .unwrap_or(false);
        let body = if bound {
            "This macro is bound to a button. The binding will no longer resolve;\nchoose a new action for that button before applying.".to_string()
        } else {
            "The macro's actions will be cleared.".to_string()
        };
        self.dialog = Some(Dialog::Confirm {
            title: format!("Delete macro {}", slot as usize + 1),
            body,
            action: PendingAction::DeleteMacro(slot),
            destructive: true,
        });
    }

    pub(crate) fn handle_close_request(&mut self, ctx: &egui::Context) {
        if ctx.input(|input| input.viewport().close_requested()) {
            if self.allow_close {
                return;
            }
            if self.is_dirty() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.show_close_prompt = true;
            }
        }
    }

    fn apply_section_env(&mut self) {
        let Ok(name) = std::env::var("REDRAGON_GUI_SECTION") else {
            return;
        };
        let wanted = name.to_ascii_lowercase();
        if let Some(section) = Section::ALL
            .into_iter()
            .find(|section| section.label().to_ascii_lowercase() == wanted)
        {
            self.section = section;
        }
    }

    /// Testing aid: `REDRAGON_GUI_SCREENSHOT=/path/shot.ppm` captures one frame.
    fn maybe_screenshot(&mut self, ctx: &egui::Context) {
        let Some(path) = std::env::var_os("REDRAGON_GUI_SCREENSHOT") else {
            return;
        };
        if self.screenshot_taken {
            return;
        }
        if self.draft.is_none() && self.busy {
            ctx.request_repaint_after(Duration::from_millis(100));
            return;
        }
        self.screenshot_taken = true;
        let callback_ctx = ctx.clone();
        ctx.request_screenshot(move |image| {
            let path = PathBuf::from(&path);
            if let Err(err) = write_ppm(&path, &image) {
                eprintln!("screenshot failed: {err}");
            }
            callback_ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        });
    }

    fn draw_status(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("status").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.heading("Redragon Mouse");
                ui.separator();
                match (&self.descriptor, &self.device) {
                    (Some(desc), Some(det)) => {
                        ui.label(egui::RichText::new(&desc.model).strong());
                        ui.small(format!(
                            "{:04x}:{:04x} · {} · bcdDevice {:#06x}",
                            det.device.vendor_id,
                            det.device.product_id,
                            det.device.node.display(),
                            det.device.bcd_device
                        ));
                        if !desc.verified {
                            ui.colored_label(
                                egui::Color32::from_rgb(220, 160, 40),
                                "unverified descriptor",
                            );
                        }
                    }
                    (Some(desc), None) => {
                        ui.label(egui::RichText::new(&desc.model).strong());
                        ui.small("simulation");
                    }
                    _ => {
                        ui.label("No supported mouse connected");
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.is_dirty() {
                        ui.colored_label(
                            egui::Color32::from_rgb(220, 160, 40),
                            "● pending edits (Ctrl+Enter to apply)",
                        );
                    }
                    if self.busy {
                        ui.spinner();
                    }
                });
            });
            ui.add_space(2.0);
            if let Some(status) = &self.status {
                let color = if status.error {
                    egui::Color32::from_rgb(220, 100, 90)
                } else {
                    ui.visuals().weak_text_color()
                };
                ui.colored_label(color, &status.text);
            }
            ui.add_space(4.0);
        });
    }

    fn draw_nav(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("navigation")
            .resizable(false)
            .default_size(190.0)
            .show(ui, |ui| {
                ui.add_space(8.0);
                ui.label(egui::RichText::new("SECTIONS").small().weak());
                for section in Section::ALL {
                    let supported = self.section_supported(section);
                    let response = ui
                        .add_enabled_ui(supported, |ui| {
                            ui.selectable_label(self.section == section, section.label())
                        })
                        .inner;
                    if response.clicked() {
                        self.section = section;
                    }
                    if !supported {
                        response.on_disabled_hover_text(unsupported_reason(section));
                    }
                }
                ui.separator();
                if let Some(desc) = &self.descriptor {
                    ui.label(egui::RichText::new("PROFILE").small().weak());
                    egui::ComboBox::from_id_salt("nav-profile")
                        .width(160.0)
                        .selected_text(profile_label(
                            self.selected_profile,
                            self.draft.as_ref().map(|state| state.active_profile),
                        ))
                        .show_ui(ui, |ui| {
                            for profile in desc.profile_indices() {
                                let label = profile_label(
                                    profile,
                                    self.draft.as_ref().map(|state| state.active_profile),
                                );
                                ui.selectable_value(&mut self.selected_profile, profile, label);
                            }
                        });
                    if let Some(state) = &self.draft {
                        if state.active_profile == self.selected_profile {
                            ui.small("in use on the mouse");
                        } else {
                            ui.small("stored, not in use");
                            let enabled = !self.busy && !self.is_dirty();
                            let slot = self.selected_profile;
                            let response = ui
                                .add_enabled_ui(enabled, |ui| ui.button("Switch mouse to this"))
                                .inner;
                            if response.clicked() {
                                self.perform(PendingAction::ActivateProfile(slot));
                            }
                            if !enabled {
                                response
                                    .on_disabled_hover_text("Apply or revert pending edits first.");
                            }
                        }
                    }
                }
                ui.separator();
                let gate = self.apply_gate();
                ui.add_enabled_ui(gate.is_none(), |ui| {
                    if ui
                        .button(egui::RichText::new("Apply changes").strong())
                        .clicked()
                    {
                        self.begin_apply_dialog();
                    }
                });
                ui.add_enabled_ui(self.is_dirty() && !self.busy, |ui| {
                    if ui.button("Revert").clicked() {
                        self.revert();
                    }
                });
                if let Some(reason) = gate {
                    ui.small(reason);
                }
                ui.separator();
                if let Some(desc) = &self.descriptor {
                    ui.small(format!(
                        "{} profile(s) · {}",
                        desc.profile_count(),
                        desc.capability_list().join(", ")
                    ));
                }
            });
    }

    fn section_supported(&self, section: Section) -> bool {
        let Some(desc) = &self.descriptor else {
            return section == Section::Device || section == Section::Diagnostics;
        };
        match section {
            Section::Pointer => {
                desc.supports_dpi() || desc.supports_polling() || desc.supports_device_settings()
            }
            Section::Lighting => desc.supports_lighting(),
            Section::Profiles => true,
            Section::Buttons => desc.buttons.is_some(),
            Section::Macros => desc.supports_macros(),
            Section::Device | Section::Backups | Section::Diagnostics => true,
        }
    }

    fn draw_section(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| match self.section {
                Section::Device => self.draw_device(ui),
                Section::Pointer => self.draw_pointer(ui),
                Section::Lighting => self.draw_lighting(ui),
                Section::Profiles => self.draw_profiles(ui),
                Section::Buttons => self.draw_buttons(ui),
                Section::Macros => self.draw_macros(ui),
                Section::Backups => self.draw_backups(ui),
                Section::Diagnostics => self.draw_diagnostics(ui),
            });
        });
    }

    fn draw_dialogs(&mut self, ctx: &egui::Context) {
        if self.show_close_prompt {
            let response = egui::Modal::new(egui::Id::unique("quit-prompt")).show(ctx, |ui| {
                ui.heading("Unapplied changes");
                ui.label("Settings have been edited but not written to the mouse.");
                ui.horizontal(|ui| {
                    if ui.button("Apply and quit").clicked() {
                        self.begin_apply_dialog();
                    }
                    if ui.button("Discard and quit").clicked() {
                        self.allow_close = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    if ui.button("Keep editing").clicked() {
                        self.show_close_prompt = false;
                    }
                });
            });
            if response.should_close() {
                self.show_close_prompt = false;
            }
            return;
        }

        let Some(dialog) = self.dialog.clone() else {
            return;
        };
        let modal_id = egui::Id::unique("dialog");
        match dialog {
            Dialog::ApplySummary {
                changes,
                mut unverified_ack,
            } => {
                let mut apply = false;
                let mut cancel = false;
                let needs_ack = self.needs_unverified_ack();
                let lines = self.change_lines();
                let desc_note = self
                    .descriptor
                    .as_ref()
                    .filter(|desc| !desc.verified)
                    .map(|desc| (desc.provenance.label().to_string(), desc.model.clone()));
                egui::Modal::new(modal_id).show(ctx, |ui| {
                    ui.set_max_width(620.0);
                    ui.heading("Apply these changes?");
                    for line in &lines {
                        ui.label(format!("•  {line}"));
                    }
                    ui.add_space(4.0);
                    egui::CollapsingHeader::new(format!("Register details ({})", changes.len()))
                        .default_open(lines.is_empty())
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                                for change in &changes {
                                    ui.horizontal(|ui| {
                                        ui.label(&change.label);
                                        if let Some(before) = &change.before {
                                            ui.weak(format!(
                                                "[{}] → [{}]",
                                                diff::describe_payload(before),
                                                diff::describe_payload(&change.after)
                                            ));
                                        } else {
                                            ui.weak(format!(
                                                "→ [{}]",
                                                diff::describe_payload(&change.after)
                                            ));
                                        }
                                    });
                                }
                            });
                        });
                    if let Some((provenance, model)) = &desc_note {
                        ui.add_space(6.0);
                        ui.colored_label(
                            egui::Color32::from_rgb(220, 160, 40),
                            format!(
                                "The descriptor for {model} is unverified ({provenance}).\nA wrong write can leave the mouse unusable until the vendor software restores it."
                            ),
                        );
                        if needs_ack {
                            ui.checkbox(
                                &mut unverified_ack,
                                "I understand the risk and want to write anyway",
                            );
                        }
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let enabled = !needs_ack || unverified_ack;
                        ui.add_enabled_ui(enabled, |ui| {
                            if ui.button("Apply").clicked() {
                                apply = true;
                            }
                        });
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    });
                });
                if apply {
                    self.unverified_acknowledged = true;
                    self.request_apply(false);
                } else if cancel {
                    self.dialog = None;
                } else if needs_ack {
                    self.dialog = Some(Dialog::ApplySummary {
                        changes,
                        unverified_ack,
                    });
                }
            }
            Dialog::Stale { differences } => {
                let mut force = false;
                let mut cancel = false;
                egui::Modal::new(modal_id).show(ctx, |ui| {
                    ui.set_max_width(620.0);
                    ui.heading("The mouse changed since it was read");
                    ui.label("These registers no longer hold the values read earlier:");
                    for difference in &differences {
                        ui.label(difference);
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Write anyway").clicked() {
                            force = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    });
                });
                if force {
                    self.request_apply(true);
                } else if cancel {
                    self.dialog = None;
                }
            }
            Dialog::Validation(errors) => {
                egui::Modal::new(modal_id).show(ctx, |ui| {
                    ui.set_max_width(600.0);
                    ui.heading("These settings cannot be applied");
                    for error in &errors {
                        ui.label(format!("{}: {}", error.field, error.message));
                    }
                    if ui.button("Close").clicked() {
                        self.dialog = None;
                    }
                });
            }
            Dialog::Confirm {
                title,
                body,
                action,
                destructive,
            } => {
                let mut confirm = false;
                let mut cancel = false;
                egui::Modal::new(modal_id).show(ctx, |ui| {
                    ui.set_max_width(560.0);
                    ui.heading(&title);
                    ui.label(&body);
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let label = if destructive { "Confirm" } else { "OK" };
                        if ui.button(label).clicked() {
                            confirm = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    });
                });
                if confirm {
                    self.dialog = None;
                    self.perform(action);
                } else if cancel {
                    self.dialog = None;
                }
            }
            Dialog::Error { title, body } => {
                egui::Modal::new(modal_id).show(ctx, |ui| {
                    ui.set_max_width(640.0);
                    ui.heading(&title);
                    ui.label(&body);
                    if ui.button("Close").clicked() {
                        self.dialog = None;
                    }
                });
            }
            Dialog::Notice { title, lines } => {
                egui::Modal::new(modal_id).show(ctx, |ui| {
                    ui.set_max_width(620.0);
                    ui.heading(&title);
                    for line in &lines {
                        ui.label(line);
                    }
                    if ui.button("Close").clicked() {
                        self.dialog = None;
                    }
                });
            }
        }
    }
}

/// "Profile 2 (in use)" for the profile the mouse is using.
fn profile_label(profile: u8, active: Option<u8>) -> String {
    if active == Some(profile) {
        format!("Profile {} (in use)", profile + 1)
    } else {
        format!("Profile {}", profile + 1)
    }
}

fn unsupported_reason(section: Section) -> &'static str {
    match section {
        Section::Buttons => "Button remapping is not available for this mouse yet.",
        Section::Macros => "Macros are not available for this mouse yet.",
        Section::Pointer => "This mouse declares no pointer settings.",
        Section::Lighting => "This mouse declares no lighting settings.",
        _ => "Not available.",
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.apply_section_env();
        if self.busy {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        self.maybe_screenshot(&ctx);
        self.poll_hotplug(&ctx);
        self.poll_worker();
        self.handle_close_request(&ctx);
        let apply_shortcut =
            ctx.input(|input| input.modifiers.command && input.key_pressed(egui::Key::Enter));
        if apply_shortcut && self.dialog.is_none() && self.apply_gate().is_none() {
            self.begin_apply_dialog();
        }
        self.draw_status(ui);
        self.draw_nav(ui);
        self.draw_section(ui);
        self.draw_dialogs(&ctx);
        if self.names_dirty {
            self.save_names();
            self.names_dirty = false;
        }
    }
}

fn write_ppm(path: &std::path::Path, image: &egui::ColorImage) -> std::io::Result<()> {
    use std::io::Write;
    let [width, height] = image.size;
    let mut out = Vec::with_capacity(width * height * 3 + 32);
    write!(&mut out, "P6\n{width} {height}\n255\n")?;
    for pixel in &image.pixels {
        out.extend_from_slice(&[pixel.r(), pixel.g(), pixel.b()]);
    }
    std::fs::write(path, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with_fixture() -> App {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = Paths {
            state_dir: tmp.path().to_path_buf(),
            config_dir: tmp.path().join("config"),
            runtime_dir: tmp.path().join("run"),
        };
        let registry = Registry {
            descriptors: vec![crate::models::fixture::descriptor()],
            ..Default::default()
        };
        let mut app = App::new(paths, registry, PathBuf::new(), None);
        let desc = crate::models::fixture::descriptor();
        let state = crate::settings::seed_state(&desc);
        app.selected_profile = 0;
        app.descriptor = Some(desc);
        app.baseline = Some(state.clone());
        app.draft = Some(state);
        drop(tmp);
        app
    }

    #[test]
    fn apply_is_blocked_without_changes() {
        let app = app_with_fixture();
        assert_eq!(app.apply_gate().as_deref(), Some("No changes to apply."));
    }

    #[test]
    fn apply_gate_reports_validation_problems() {
        let mut app = app_with_fixture();
        app.draft.as_mut().unwrap().profiles[0].polling_hz = Some(250);
        let gate = app.apply_gate().expect("blocked");
        assert!(gate.contains("need attention"), "{gate}");
    }

    #[test]
    fn unverified_descriptors_need_an_extra_acknowledgement() {
        let mut app = app_with_fixture();
        assert!(!app.needs_unverified_ack(), "fixture is verified");
        app.descriptor.as_mut().unwrap().verified = false;
        assert!(app.needs_unverified_ack());
        app.unverified_acknowledged = true;
        assert!(!app.needs_unverified_ack());
    }

    #[test]
    fn pending_edits_survive_a_disconnect() {
        let mut app = app_with_fixture();
        app.draft.as_mut().unwrap().profiles[0]
            .lighting
            .as_mut()
            .unwrap()
            .brightness = 0;
        assert!(app.is_dirty());
        app.on_device_removed();
        assert!(app.device.is_none());
        assert!(app.is_dirty(), "edits must be retained");
    }

    #[test]
    fn reconnect_keeps_pending_edits_and_warns() {
        let mut app = app_with_fixture();
        let baseline = app.baseline.clone().expect("baseline");
        app.draft.as_mut().unwrap().profiles[0]
            .lighting
            .as_mut()
            .unwrap()
            .brightness = 0;
        let pending = app.draft.clone().expect("draft");
        app.device = None;
        app.reconnect_pending = Some(pending);
        app.install_read(baseline);
        assert_eq!(
            app.draft.as_ref().unwrap().profiles[0]
                .lighting
                .as_ref()
                .unwrap()
                .brightness,
            0,
            "pending edits must survive a reconnect"
        );
        assert!(app.status.as_ref().unwrap().text.contains("Pending edits"));
    }

    #[test]
    fn activation_is_confirmed_by_read_back() {
        let mut app = app_with_fixture();
        let mut state = app.baseline.clone().expect("baseline");
        state.active_profile = 2;
        app.expect_profile = Some(2);
        app.install_read(state);
        let status = app.status.as_ref().expect("status");
        assert!(status.text.contains("now active"), "{}", status.text);
        assert!(!status.error);
    }

    #[test]
    fn unconfirmed_activation_is_reported() {
        let mut app = app_with_fixture();
        let state = app.baseline.clone().expect("baseline");
        app.expect_profile = Some(3);
        app.install_read(state);
        let status = app.status.as_ref().expect("status");
        assert!(status.text.contains("not confirmed"), "{}", status.text);
        assert!(status.error);
    }

    #[test]
    fn copying_a_profile_copies_its_settings() {
        let mut app = app_with_fixture();
        app.draft.as_mut().unwrap().profiles[0]
            .lighting
            .as_mut()
            .unwrap()
            .brightness = 0;
        app.perform(PendingAction::CopyProfile { from: 0, to: 3 });
        assert_eq!(
            app.draft.as_ref().unwrap().profiles[3]
                .lighting
                .as_ref()
                .unwrap()
                .brightness,
            0
        );
    }

    #[test]
    fn resetting_a_profile_restores_defaults() {
        let mut app = app_with_fixture();
        app.draft.as_mut().unwrap().profiles[0]
            .dpi
            .as_mut()
            .unwrap()
            .active = 3;
        app.perform(PendingAction::ResetProfile(0));
        assert_eq!(
            app.draft.as_ref().unwrap().profiles[0]
                .dpi
                .as_ref()
                .unwrap()
                .active,
            0
        );
    }
    #[test]
    fn every_section_renders_headless() {
        let mut app = app_with_fixture();
        let ctx = egui::Context::default();
        for section in Section::ALL {
            app.section = section;
            let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                app.draw_status(ui);
                app.draw_nav(ui);
                app.draw_section(ui);
            });
            output.drop_without_applying_deltas();
        }
    }

    #[test]
    fn dialogs_render_headless() {
        let mut app = app_with_fixture();
        let ctx = egui::Context::default();
        let changes = app.changes();
        let _ = changes;
        app.draft.as_mut().unwrap().profiles[0]
            .lighting
            .as_mut()
            .unwrap()
            .brightness = 0;
        let changes = app.changes();
        let dialogs = vec![
            Dialog::ApplySummary {
                changes,
                unverified_ack: false,
            },
            Dialog::Stale {
                differences: vec!["LED brightness: device holds [2]".into()],
            },
            Dialog::Validation(vec![FieldError {
                field: "polling".into(),
                message: "not supported".into(),
            }]),
            Dialog::Confirm {
                title: "Confirm".into(),
                body: "Body".into(),
                action: PendingAction::ResetProfile(0),
                destructive: true,
            },
            Dialog::Error {
                title: "Error".into(),
                body: "Body".into(),
            },
            Dialog::Notice {
                title: "Notice".into(),
                lines: vec!["line".into()],
            },
        ];
        for dialog in dialogs {
            app.dialog = Some(dialog);
            let output = ctx.run_pass(egui::RawInput::default(), |ctx| {
                app.draw_dialogs(ctx);
            });
            output.drop_without_applying_deltas();
        }
    }

    #[test]
    fn a_read_after_an_apply_keeps_the_profile_being_edited() {
        let mut app = app_with_fixture();
        app.selected_profile = 3;
        let state = app.baseline.clone().expect("baseline");
        app.install_read(state);
        assert_eq!(
            app.selected_profile, 3,
            "the editor must not be pulled away"
        );
    }

    #[test]
    fn the_first_read_selects_the_active_profile() {
        let mut app = app_with_fixture();
        app.baseline = None;
        app.draft = None;
        let mut state = crate::settings::seed_state(app.descriptor.as_ref().unwrap());
        state.active_profile = 2;
        app.install_read(state);
        assert_eq!(app.selected_profile, 2);
    }

    #[test]
    fn activating_a_profile_moves_the_editor_to_it() {
        let mut app = app_with_fixture();
        let mut state = app.baseline.clone().expect("baseline");
        state.active_profile = 4;
        app.expect_profile = Some(4);
        app.install_read(state);
        assert_eq!(app.selected_profile, 4);
    }

    #[test]
    fn pending_edits_are_listed_in_plain_language() {
        let mut app = app_with_fixture();
        assert!(app.change_lines().is_empty());
        app.draft.as_mut().unwrap().profiles[0].polling_hz = Some(1000);
        let lines = app.change_lines();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].starts_with("Profile 1 · Polling rate"),
            "{lines:?}"
        );
    }
}
