//! Register image, typed state, diffing, apply engine, and backups.

pub mod apply;
pub mod assign;
pub mod backup;
pub mod diff;
pub mod macro_file;
pub mod profile_file;
pub mod state;

pub use apply::{
    activate_profile, apply, apply_backup, apply_ops, read_image, read_state,
    read_state_consistent, ApplyError, ApplyReport, ApplySettings, DeviceIds, ReadError, Rollback,
};
pub use backup::{BackupFile, BackupInfo, MacroNames};
pub use macro_file::{export_macro, import_macro, MacroFile};
pub use profile_file::{export_profile, import_profile, ImportResult};
pub use state::{
    decode, encode_ops, seed_image, seed_state, validate_state, Burst, ButtonBinding,
    DeviceSettings, DeviceState, DpiStage, FieldError, Macro, MacroAction, MacroActionKind,
    ProfileDpi, ProfileLighting, ProfileState, ReadImage, WindowData, WriteOp,
};
