//! Linux configuration tool for Redragon/Holtek USB gaming mice.
//!
//! The library is split into three UI-free layers and one presentation layer:
//!
//! * [`transport`] — frames and the feature-report channel (real hidraw node or simulation).
//! * [`device`] — sysfs enumeration, report-descriptor parsing, access diagnostics.
//! * [`models`] — per-model descriptors and the registry that maps a device to one.
//! * [`settings`] — register image, diffing, transactional apply, backups.
//! * [`ui`] — an egui front end over the layers above.
//!
//! `main.rs` wires the CLI flags and the GUI bootstrap; everything testable lives here.

pub mod device;
pub mod models;
pub mod paths;
pub mod settings;
pub mod transport;
pub mod ui;

/// Exit codes used by the non-interactive entry points (`--list-devices`, `--dump-state`).
pub mod exit {
    /// Everything requested succeeded.
    pub const OK: i32 = 0;
    /// An unexpected error (I/O, descriptor loading, argument parsing).
    pub const ERROR: i32 = 1;
    /// No supported device was found.
    pub const NO_DEVICE: i32 = 2;
    /// A supported device exists but its configuration interface cannot be opened.
    pub const ACCESS: i32 = 3;
}
