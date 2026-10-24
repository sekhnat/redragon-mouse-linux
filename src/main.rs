//! Command-line entry points and GUI bootstrap.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use redragon_gui::device;
use redragon_gui::paths::{LogFile, Paths};
use redragon_gui::settings;
use redragon_gui::transport::{hidraw::HidrawTransport, Pacer};
use redragon_gui::ui::{self, RunOptions};
use redragon_gui::{exit, models};

fn main() -> ExitCode {
    // Behave like a normal Unix tool when stdout is piped into `head`.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    match dispatch() {
        Ok(code) => ExitCode::from(code as u8),
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::from(exit::ERROR as u8)
        }
    }
}

enum Command {
    Gui,
    ListDevices,
    DumpState,
    ShowDescriptor(Option<String>),
    ReportDescriptor,
    DumpRaw,
    Simulate(String),
    Help,
    Version,
}

struct Options {
    command: Command,
    models_dir: Option<PathBuf>,
}

fn dispatch() -> Result<i32, String> {
    let options = parse_args(std::env::args().skip(1).collect())?;
    let paths = Paths::from_env();
    let log = LogFile::create(&paths).map_err(|err| format!("cannot create log: {err}"))?;
    init_tracing(&log);
    let models_dir = options
        .models_dir
        .clone()
        .unwrap_or_else(|| paths.models_dir());
    tracing::info!(
        command = ?std::mem::discriminant(&options.command),
        models = %models_dir.display(),
        log = %log.path().display(),
        "starting"
    );

    match options.command {
        Command::Help => {
            print_help();
            Ok(exit::OK)
        }
        Command::Version => {
            println!("redragon-gui {}", env!("CARGO_PKG_VERSION"));
            Ok(exit::OK)
        }
        Command::ListDevices => list_devices(&models_dir),
        Command::ShowDescriptor(id) => show_descriptor(&models_dir, id.as_deref()),
        Command::ReportDescriptor => report_descriptor(&models_dir),
        Command::DumpRaw => dump_raw(&models_dir),
        Command::DumpState => dump_state(&models_dir, log.path()),
        Command::Simulate(model) => ui::run(RunOptions {
            simulation: Some(model),
            models_dir: Some(models_dir),
            log_path: Some(log.path().to_path_buf()),
        })
        .map(|()| exit::OK),
        Command::Gui => ui::run(RunOptions {
            simulation: None,
            models_dir: Some(models_dir),
            log_path: Some(log.path().to_path_buf()),
        })
        .map(|()| exit::OK),
    }
}

fn parse_args(args: Vec<String>) -> Result<Options, String> {
    let mut command = Command::Gui;
    let mut models_dir = None;
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        match arg {
            "-h" | "--help" => command = Command::Help,
            "-V" | "--version" => command = Command::Version,
            "--list-devices" => command = Command::ListDevices,
            "--dump-state" => command = Command::DumpState,
            "--report-descriptor" => command = Command::ReportDescriptor,
            "--dump-raw" => command = Command::DumpRaw,
            "--show-descriptor" => {
                let id = args
                    .get(index + 1)
                    .filter(|next| !next.starts_with('-'))
                    .cloned();
                if id.is_some() {
                    index += 1;
                }
                command = Command::ShowDescriptor(id);
            }
            "--simulate" => {
                let model = args
                    .get(index + 1)
                    .ok_or_else(|| "--simulate needs a model id or name".to_string())?
                    .clone();
                index += 1;
                command = Command::Simulate(model);
            }
            "--models-dir" => {
                let dir = args
                    .get(index + 1)
                    .ok_or_else(|| "--models-dir needs a path".to_string())?;
                index += 1;
                models_dir = Some(PathBuf::from(dir));
            }
            "--no-color" => {}
            "--log-level" => {
                // Accepted for compatibility with the documented CLI surface.
                index += 1;
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown option `{other}` (try --help)"))
            }
            other => return Err(format!("unexpected argument `{other}` (try --help)")),
        }
        index += 1;
    }
    Ok(Options {
        command,
        models_dir,
    })
}

fn init_tracing(log: &LogFile) {
    use tracing_subscriber::fmt;
    let _ = fmt()
        .with_writer(log.clone())
        .with_ansi(false)
        .with_target(false)
        .try_init();
}

fn list_devices(models_dir: &std::path::Path) -> Result<i32, String> {
    let registry = models::Registry::load(models_dir);
    for error in &registry.errors {
        eprintln!(
            "descriptor warning: {}: {}",
            error.file.display(),
            error.message
        );
    }
    let base = device::enumerate::default_base();
    let detected = device::scan_and_identify(&registry, &base)
        .map_err(|err| format!("cannot scan {}: {err}", base.display()))?;
    if detected.is_empty() {
        println!("no HID devices found in {}", base.display());
        return Ok(exit::NO_DEVICE);
    }
    let mut supported = 0usize;
    for det in &detected {
        let dev = &det.device;
        let header = format!(
            "{}  {}  interface {}  bcdDevice {:#06x}",
            dev.node.display(),
            dev.id_string(),
            dev.interface_number,
            dev.bcd_device
        );
        match &det.identification {
            device::Identification::Supported {
                descriptor_index, ..
            } => {
                supported += 1;
                let desc = &registry.descriptors[*descriptor_index];
                println!("{header}");
                println!(
                    "  model: {} ({}){}",
                    desc.model,
                    desc.id,
                    if desc.verified { "" } else { " [unverified]" }
                );
            }
            device::Identification::Unsupported => {
                println!(
                    "{}  unsupported: {}",
                    header,
                    device::unsupported_reason(&det.identification)
                );
            }
            device::Identification::Conflict(ids) => {
                println!(
                    "{}  conflict: several descriptors claim this release ({})",
                    header,
                    ids.join(", ")
                );
            }
        }
    }
    if supported == 0 {
        Ok(exit::NO_DEVICE)
    } else {
        Ok(exit::OK)
    }
}

fn resolve_model<'a>(
    registry: &'a models::Registry,
    needle: &str,
) -> Result<&'a models::Descriptor, String> {
    registry
        .descriptors
        .iter()
        .find(|d| d.id == needle)
        .or_else(|| {
            registry
                .descriptors
                .iter()
                .find(|d| d.model.to_lowercase().contains(&needle.to_lowercase()))
        })
        .ok_or_else(|| format!("no descriptor matches `{needle}`"))
}

fn show_descriptor(models_dir: &std::path::Path, id: Option<&str>) -> Result<i32, String> {
    let registry = models::Registry::load(models_dir);
    for error in &registry.errors {
        eprintln!(
            "descriptor warning: {}: {}",
            error.file.display(),
            error.message
        );
    }
    let desc = match id {
        Some(id) => resolve_model(&registry, id)?,
        None => registry
            .descriptors
            .first()
            .ok_or_else(|| "no descriptors loaded".to_string())?,
    };
    println!("id:          {}", desc.id);
    println!("model:       {}", desc.model);
    println!("schema:      {}", desc.schema);
    println!(
        "provenance:  {} ({})",
        desc.provenance.label(),
        if desc.verified {
            "verified"
        } else {
            "unverified"
        }
    );
    println!(
        "collection:  usage page {:#06x} / usage {:#06x}",
        desc.collections.usage_page, desc.collections.usage
    );
    let frames: Vec<String> = desc
        .frames
        .iter()
        .map(|frame| format!("id {} = {} B", frame.id, frame.len))
        .collect();
    println!("frames:      {}", frames.join(", "));
    println!("profiles:    {}", desc.profile_count());
    println!("capabilities: {}", desc.capability_list().join(", "));
    println!("read windows:");
    for instance in desc.window_instances().unwrap_or_default() {
        println!(
            "  {:<14} profile {}  report {}  addr {:#06x}  len {}",
            instance.id, instance.profile, instance.report, instance.addr, instance.len
        );
    }
    println!("write regions: {}", desc.write_footprint().len());
    println!(
        "tables:      {} DPI codes, {} keys, {} actions",
        desc.tables.dpi_codes.len(),
        desc.tables.keys.len(),
        desc.tables.actions.len()
    );
    if let Some(notes) = &desc.notes {
        println!("notes:\n{notes}");
    }
    Ok(exit::OK)
}

/// Report an interface error: access failures get udev guidance and exit 3,
/// model/hardware mismatches are plain errors with exit 1.
fn interface_failure(
    device: &device::HidrawDevice,
    err: &device::InterfaceError,
) -> Result<i32, String> {
    if err.is_access() {
        let access = device::inspect(&device.node).ok();
        let guidance = device::guidance(device, access.as_ref(), &err.to_string());
        println!("{}", guidance.detail);
        println!();
        println!("{}", guidance.instructions());
        Ok(exit::ACCESS)
    } else {
        eprintln!(
            "{} does not match the model descriptor: {err}",
            device.node.display()
        );
        Ok(exit::ERROR)
    }
}

fn report_descriptor(models_dir: &std::path::Path) -> Result<i32, String> {
    let registry = models::Registry::load(models_dir);
    let base = device::enumerate::default_base();
    let detected = device::scan_and_identify(&registry, &base)
        .map_err(|err| format!("cannot scan {}: {err}", base.display()))?;
    let Some(det) = detected.iter().find(|d| d.is_supported()) else {
        println!("no supported device found");
        return Ok(exit::NO_DEVICE);
    };
    let bytes =
        match redragon_gui::transport::hidraw::HidrawTransport::report_descriptor(&det.device.node)
        {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
                let access = device::inspect(&det.device.node).ok();
                let guidance = device::guidance(&det.device, access.as_ref(), &err.to_string());
                println!("{}", guidance.detail);
                println!();
                println!("{}", guidance.instructions());
                return Ok(exit::ACCESS);
            }
            Err(err) => {
                eprintln!(
                    "cannot read the report descriptor from {}: {err}",
                    det.device.node.display()
                );
                return Ok(exit::ERROR);
            }
        };
    println!("device: {}", det.device.node.display());
    println!("interface: {}", det.device.interface_number);
    println!("descriptor: {} bytes", bytes.len());
    for chunk in bytes.chunks(16) {
        let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
        println!("  {}", hex.join(" "));
    }
    match device::ReportDescriptor::parse(&bytes) {
        Ok(parsed) => {
            println!("top-level collections:");
            for collection in &parsed.collections {
                println!(
                    "  usage page {:#06x} / usage {:#04x}",
                    collection.usage_page, collection.usage
                );
            }
            println!("feature reports:");
            for (id, bits) in &parsed.feature_reports {
                println!("  id {id}: {} bytes", bits.div_ceil(8));
            }
        }
        Err(err) => println!("parse error: {err}"),
    }
    Ok(exit::OK)
}

fn dump_raw(models_dir: &std::path::Path) -> Result<i32, String> {
    let registry = models::Registry::load(models_dir);
    let base = device::enumerate::default_base();
    let detected = device::scan_and_identify(&registry, &base)
        .map_err(|err| format!("cannot scan {}: {err}", base.display()))?;
    let Some(det) = detected.iter().find(|d| d.is_supported()) else {
        println!("no supported device found");
        return Ok(exit::NO_DEVICE);
    };
    let descriptor_index = match det.identification {
        device::Identification::Supported {
            descriptor_index, ..
        } => descriptor_index,
        _ => unreachable!(),
    };
    let desc = &registry.descriptors[descriptor_index];
    if let Err(err) = device::validate_interface(&det.device, desc) {
        return interface_failure(&det.device, &err);
    }
    let mut transport = redragon_gui::transport::hidraw::HidrawTransport::open(&det.device.node)
        .map_err(|err| err.to_string())?;
    let state = match settings::read_state_consistent(&mut transport, desc, &Pacer::production()) {
        Ok(state) => state,
        Err(err) => {
            println!("cannot read a consistent state: {err}");
            return Ok(exit::ERROR);
        }
    };
    println!(
        "# device {} model {} descriptor {}",
        det.device.node.display(),
        desc.model,
        desc.id
    );
    for ((id, profile), window) in state.image.iter() {
        let hex: Vec<String> = window.bytes.iter().map(|b| format!("{b:02x}")).collect();
        println!(
            "{id}[{profile}] report {} addr {:#06x} len {}: {}",
            window.report,
            window.addr,
            window.bytes.len(),
            hex.join(" ")
        );
    }
    Ok(exit::OK)
}

fn dump_state(models_dir: &std::path::Path, log_path: &Path) -> Result<i32, String> {
    let registry = models::Registry::load(models_dir);
    let base = device::enumerate::default_base();
    let detected = device::scan_and_identify(&registry, &base)
        .map_err(|err| format!("cannot scan {}: {err}", base.display()))?;
    let Some(det) = detected.iter().find(|d| d.is_supported()) else {
        println!("no supported device found");
        return Ok(exit::NO_DEVICE);
    };
    let device_index = match det.identification {
        device::Identification::Supported {
            descriptor_index, ..
        } => descriptor_index,
        _ => unreachable!(),
    };
    let desc = &registry.descriptors[device_index];
    if let Err(err) = device::validate_interface(&det.device, desc) {
        return interface_failure(&det.device, &err);
    }
    let mut transport = match HidrawTransport::open(&det.device.node) {
        Ok(transport) => transport,
        Err(err) => {
            let access = device::inspect(&det.device.node).ok();
            let guidance = device::guidance(&det.device, access.as_ref(), &err.to_string());
            if matches!(
                err,
                redragon_gui::transport::TransportError::PermissionDenied { .. }
                    | redragon_gui::transport::TransportError::Busy(_)
                    | redragon_gui::transport::TransportError::NotFound(_)
            ) {
                println!("{}", guidance.detail);
                println!();
                println!("{}", guidance.instructions());
                return Ok(exit::ACCESS);
            }
            return Err(format!("{}: {err}", guidance.detail));
        }
    };
    let state = match settings::read_state_consistent(&mut transport, desc, &Pacer::production()) {
        Ok(state) => state,
        Err(err) => {
            println!("cannot read a consistent state: {err}");
            println!("log: {}", log_path.display());
            return Ok(exit::ERROR);
        }
    };
    println!("device:      {}", det.device.node.display());
    println!("model:       {} ({})", desc.model, desc.id);
    println!("active profile: {}", state.active_profile + 1);
    println!("windows read:   {}", state.image.len());
    for (profile, value) in state.profiles.iter().enumerate() {
        println!("profile {}:", profile + 1);
        if let Some(dpi) = &value.dpi {
            let stages: Vec<String> = dpi
                .stages
                .iter()
                .enumerate()
                .map(|(index, stage)| {
                    let value = if stage.x == 0 {
                        format!("code:{:#06x}", stage.raw)
                    } else {
                        stage.x.to_string()
                    };
                    format!(
                        "{}{}={}",
                        if stage.enabled { "" } else { "off:" },
                        index + 1,
                        value
                    )
                })
                .collect();
            println!("  dpi: {} (active {})", stages.join(" "), dpi.active + 1);
        }
        if let Some(hz) = value.polling_hz {
            println!("  polling: {hz} Hz");
        }
        if let Some(lighting) = &value.lighting {
            println!(
                "  lighting: {} rgb {:02x}{:02x}{:02x} speed {} brightness {}",
                if lighting.mode.is_empty() {
                    format!("code {:#04x}", lighting.mode_code)
                } else {
                    lighting.mode.clone()
                },
                lighting.color[0],
                lighting.color[1],
                lighting.color[2],
                lighting.speed,
                lighting.brightness
            );
        }
        if let Some(buttons) = &value.buttons {
            let mappings: Vec<String> = buttons
                .iter()
                .map(|binding| {
                    let mut text = binding.action.clone();
                    if let Some(key) = &binding.key {
                        text.push('/');
                        text.push_str(key);
                    }
                    text
                })
                .collect();
            println!("  buttons: {}", mappings.join(" "));
        }
    }
    if !state.macros.is_empty() {
        let used = state
            .macros
            .iter()
            .filter(|m| !m.actions.is_empty())
            .count();
        println!(
            "macros:      {used} of {} slots occupied",
            state.macros.len()
        );
    }
    println!("log:         {}", log_path.display());
    Ok(exit::OK)
}

fn print_help() {
    println!(
        "redragon-gui {}\n\
         Linux configuration tool for Redragon/Holtek USB gaming mice\n\n\
         USAGE:\n    redragon-gui [OPTIONS]\n\n\
         OPTIONS:\n\
         \x20   --list-devices            List HID devices and mark supported ones\n\
         \x20   --dump-state              Read the device's current settings and print them\n\
         \x20   --report-descriptor       Dump the interface report descriptor and parse it\n\
         \x20   --dump-raw                Print the raw register windows read from the device\n\
         \x20   --show-descriptor [ID]    Print a model descriptor (default: first loaded)\n\
         \x20   --simulate <MODEL>        Run the interface against a simulated device\n\
         \x20   --models-dir <PATH>       Load descriptors from PATH\n\
         \x20   --log-level <LEVEL>       Accepted for compatibility (logs to file)\n\
         \x20   --no-color                Plain output\n\
         \x20   -h, --help                Show this help\n\
         \x20   -V, --version             Show the version\n\n\
         EXIT CODES:\n\
         \x20   0 success, 1 error, 2 no supported device, 3 access failure\n\n\
         The mouse's vendor interface needs a udev rule; run --dump-state to see it.",
        env!("CARGO_PKG_VERSION")
    );
}
