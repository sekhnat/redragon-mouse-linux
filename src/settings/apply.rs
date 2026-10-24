//! Reading device state and applying edits as a verified transaction.

use std::path::PathBuf;

use crate::models::Descriptor;
use crate::paths::Paths;
use crate::transport::frames::{Frame, FrameError};
use crate::transport::{
    begin_profile, commit_all, read_window, send_frame, Pacer, Transport, TransportError,
};

use super::backup::{self, BackupFile};
use super::diff::changed_ops;
use super::state::{self, DeviceState, FieldError, ReadImage, WindowData, WriteOp};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceIds {
    pub vendor_id: u16,
    pub product_id: u16,
}

#[derive(Clone, Debug)]
pub struct ApplySettings {
    pub pacer: Pacer,
    /// Write even when the targeted registers changed since the baseline read.
    pub force_stale: bool,
    pub active_profile: u8,
    pub label: String,
}

impl Default for ApplySettings {
    fn default() -> Self {
        ApplySettings {
            pacer: Pacer::production(),
            force_stale: false,
            active_profile: 0,
            label: "apply".into(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rollback {
    pub attempted: usize,
    pub restored: usize,
    pub failures: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplyReport {
    pub changed: usize,
    pub confirmed: usize,
    pub mismatches: Vec<String>,
    pub rollback: Option<Rollback>,
    pub backup: Option<PathBuf>,
    /// Present when the stale check could not read a register (best effort).
    pub stale_check_errors: Vec<String>,
    /// Non-fatal problems after the writes, e.g. failing to switch back.
    pub warnings: Vec<String>,
}

impl ApplyReport {
    pub fn fully_confirmed(&self) -> bool {
        self.confirmed == self.changed && self.mismatches.is_empty() && self.rollback.is_none()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error("device read is not consistent: {0}")]
    Inconsistent(String),
    #[error("cannot decode device state: {0}")]
    Decode(#[from] state::DecodeError),
    #[error("read plan is invalid: {0}")]
    Plan(String),
}

#[derive(Debug, thiserror::Error)]
pub enum ApplyError {
    #[error("the pending settings are invalid")]
    Validation(Vec<FieldError>),
    #[error("no setting differs from the device state")]
    NoChanges,
    #[error("device state changed since it was read")]
    Stale { differences: Vec<String> },
    #[error("cannot write a backup before applying: {0}")]
    Backup(std::io::Error),
    #[error("transport failure while writing: {source}")]
    Transport {
        #[source]
        source: TransportError,
        rollback: Option<Rollback>,
    },
    #[error("frame could not be built: {0}")]
    Frame(#[from] FrameError),
    #[error("read plan is invalid: {0}")]
    Plan(String),
}

/// Read the whole register image described by the model.
pub fn read_image<T: Transport + ?Sized>(
    transport: &mut T,
    desc: &Descriptor,
    pacer: &Pacer,
) -> Result<ReadImage, ReadError> {
    let sizes = desc.frame_sizes();
    begin_profile(
        transport,
        &sizes,
        desc.commands.begin_report,
        desc.commands.begin,
        0,
        pacer,
    )?;
    let mut image = ReadImage::default();
    let mut read_result: Result<(), ReadError> = Ok(());
    for instance in desc.window_instances().map_err(ReadError::Plan)? {
        let bytes = match read_window(
            transport,
            &sizes,
            desc.commands.read,
            instance.report,
            instance.addr,
            instance.len,
            pacer,
        ) {
            Ok(bytes) => bytes,
            Err(err) => {
                read_result = Err(ReadError::Transport(err));
                break;
            }
        };
        image.insert(
            &instance.id,
            instance.profile,
            WindowData {
                report: instance.report,
                addr: instance.addr,
                bytes,
            },
        );
    }
    // The vendor's read sequence ends by selecting profile 1's page; without
    // it the mouse stays in its read/configuration state and stops reporting
    // pointer input. Always send it, even if a read failed.
    let exit_result = begin_profile(
        transport,
        &sizes,
        desc.commands.begin_report,
        desc.commands.begin,
        1,
        pacer,
    )
    .map_err(ReadError::Transport);
    read_result?;
    exit_result?;
    Ok(image)
}

pub fn read_state<T: Transport + ?Sized>(
    transport: &mut T,
    desc: &Descriptor,
    pacer: &Pacer,
) -> Result<DeviceState, ReadError> {
    let image = read_image(transport, desc, pacer)?;
    Ok(state::decode(desc, &image)?)
}

/// Read twice and require identical results; this is the gate that keeps a
/// device whose reads are unusable in read-only mode.
pub fn read_state_consistent<T: Transport + ?Sized>(
    transport: &mut T,
    desc: &Descriptor,
    pacer: &Pacer,
) -> Result<DeviceState, ReadError> {
    let first = read_state(transport, desc, pacer)?;
    if first.image.is_blank() {
        return Err(ReadError::Inconsistent(
            "every register read returned only 0x00 or 0xFF filler".into(),
        ));
    }
    let second = read_state(transport, desc, pacer)?;
    if !first.image.same_content(&second.image) {
        let differing: Vec<String> = first
            .image
            .iter()
            .filter(|(key, value)| {
                second
                    .image
                    .window(&key.0, key.1)
                    .map(|other| other.bytes != value.bytes)
                    .unwrap_or(true)
            })
            .map(|((id, profile), _)| format!("{id}[{profile}]"))
            .collect();
        return Err(ReadError::Inconsistent(format!(
            "two consecutive reads differ in: {}",
            differing.join(", ")
        )));
    }
    Ok(first)
}

fn payload_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn send_op<T: Transport + ?Sized>(
    transport: &mut T,
    desc: &Descriptor,
    op: &WriteOp,
    pacer: &Pacer,
) -> Result<(), TransportError> {
    let sizes = desc.frame_sizes();
    let total = sizes
        .for_report(op.report)
        .ok_or(TransportError::UnknownReport(op.report))?;
    let frame = Frame::block(
        total,
        desc.commands.write,
        op.addr,
        op.payload.len() as u8,
        op.report,
        &op.payload,
    )?;
    send_frame(transport, &frame, pacer)
}

/// Write one profile's operations with begin/commit framing.
fn write_group<T: Transport + ?Sized>(
    transport: &mut T,
    desc: &Descriptor,
    ops: &[&WriteOp],
    profile: u8,
    pacer: &Pacer,
    written: &mut Vec<WriteOp>,
) -> Result<(), TransportError> {
    let sizes = desc.frame_sizes();
    begin_profile(
        transport,
        &sizes,
        desc.commands.begin_report,
        desc.commands.begin,
        profile,
        pacer,
    )?;
    for op in ops {
        send_op(transport, desc, op, pacer)?;
        written.push((*op).clone());
    }
    commit_all(transport, &sizes, desc, &desc.commands.commit_frames, pacer)
}

fn rollback<T: Transport + ?Sized>(
    transport: &mut T,
    desc: &Descriptor,
    written: &[WriteOp],
    baseline: &[WriteOp],
    pacer: &Pacer,
) -> Rollback {
    let mut report = Rollback {
        attempted: written.len(),
        ..Default::default()
    };
    for op in written.iter().rev() {
        let Some(base) = baseline.iter().find(|base| base.id == op.id) else {
            report
                .failures
                .push(format!("{}: no baseline value to restore", op.describe()));
            continue;
        };
        match send_op(transport, desc, base, pacer) {
            Ok(()) => report.restored += 1,
            Err(err) => report.failures.push(format!("{}: {err}", op.describe())),
        }
    }
    report
}

fn verify<T: Transport + ?Sized>(
    transport: &mut T,
    desc: &Descriptor,
    changed: &[WriteOp],
    pacer: &Pacer,
) -> (usize, Vec<String>) {
    let sizes = desc.frame_sizes();
    let mut confirmed = 0usize;
    let mut mismatches = Vec::new();
    for op in changed {
        let read = read_window(
            transport,
            &sizes,
            desc.commands.read,
            op.report,
            op.addr,
            op.payload.len() as u16,
            pacer,
        );
        match read {
            Ok(actual) if actual == op.payload => confirmed += 1,
            Ok(actual) => mismatches.push(format!(
                "{}: wrote [{}] but read [{}]",
                op.describe(),
                payload_hex(&op.payload),
                payload_hex(&actual)
            )),
            Err(err) => mismatches.push(format!("{}: read-back failed: {err}", op.describe())),
        }
    }
    (confirmed, mismatches)
}

/// The transaction engine: stale check, backup, writes, verification, rollback.
pub fn apply_ops<T: Transport + ?Sized>(
    transport: &mut T,
    desc: &Descriptor,
    baseline_ops: &[WriteOp],
    target_ops: &[WriteOp],
    ids: DeviceIds,
    paths: &Paths,
    settings: &ApplySettings,
) -> Result<ApplyReport, ApplyError> {
    let changed: Vec<WriteOp> = changed_ops(baseline_ops, target_ops)
        .into_iter()
        .cloned()
        .collect();
    if changed.is_empty() {
        return Err(ApplyError::NoChanges);
    }

    // Stale check: re-read the registers about to change.
    let sizes = desc.frame_sizes();
    let mut differences = Vec::new();
    let mut stale_check_errors = Vec::new();
    for op in &changed {
        let Some(expected) = baseline_ops
            .iter()
            .find(|base| base.id == op.id)
            .map(|base| base.payload.clone())
        else {
            continue;
        };
        match read_window(
            transport,
            &sizes,
            desc.commands.read,
            op.report,
            op.addr,
            op.payload.len() as u16,
            &settings.pacer,
        ) {
            Ok(actual) if actual != expected => differences.push(format!(
                "{}: device holds [{}] but the baseline read was [{}]",
                op.describe(),
                payload_hex(&actual),
                payload_hex(&expected)
            )),
            Ok(_) => {}
            Err(err) => stale_check_errors.push(format!("{}: {err}", op.describe())),
        }
    }
    if !differences.is_empty() && !settings.force_stale {
        return Err(ApplyError::Stale { differences });
    }

    // Backup before the first frame.
    let backup_path = backup::write_backup(
        paths,
        desc,
        ids.vendor_id,
        ids.product_id,
        settings.active_profile,
        &settings.label,
        baseline_ops,
    )
    .map_err(ApplyError::Backup)?;

    // Write profile-scoped operations first, then device-level ones.
    let mut profiles: Vec<u8> = changed.iter().filter_map(|op| op.profile).collect();
    profiles.sort_unstable();
    profiles.dedup();

    let mut written: Vec<WriteOp> = Vec::new();
    let mut failure: Option<TransportError> = None;
    for profile in profiles {
        let group: Vec<&WriteOp> = changed
            .iter()
            .filter(|op| op.profile == Some(profile))
            .collect();
        if group.is_empty() {
            continue;
        }
        if let Err(err) = write_group(
            transport,
            desc,
            &group,
            profile,
            &settings.pacer,
            &mut written,
        ) {
            failure = Some(err);
            break;
        }
    }
    if failure.is_none() {
        let group: Vec<&WriteOp> = changed.iter().filter(|op| op.profile.is_none()).collect();
        if !group.is_empty() {
            if let Err(err) = write_group(
                transport,
                desc,
                &group,
                settings.active_profile,
                &settings.pacer,
                &mut written,
            ) {
                failure = Some(err);
            }
        }
    }

    if let Some(source) = failure {
        let report = rollback(transport, desc, &written, baseline_ops, &settings.pacer);
        return Err(ApplyError::Transport {
            source,
            rollback: Some(report),
        });
    }

    // Writing a stored profile can make it active. Switch back so editing a
    // stored profile does not silently change which one the mouse uses.
    let mut warnings = Vec::new();
    let touched_other = written
        .iter()
        .filter_map(|op| op.profile)
        .any(|profile| profile != settings.active_profile);
    if touched_other {
        let restored = begin_profile(
            transport,
            &sizes,
            desc.commands.begin_report,
            desc.commands.begin,
            settings.active_profile,
            &settings.pacer,
        )
        .and_then(|()| {
            commit_all(
                transport,
                &sizes,
                desc,
                &desc.commands.commit_frames,
                &settings.pacer,
            )
        });
        if let Err(err) = restored {
            warnings.push(format!(
                "could not switch back to profile {} after writing another profile: {err}",
                settings.active_profile + 1
            ));
        }
    }

    let (confirmed, mismatches) = verify(transport, desc, &changed, &settings.pacer);
    tracing::info!(
        changed = changed.len(),
        confirmed,
        mismatches = mismatches.len(),
        backup = %backup_path.display(),
        "apply finished"
    );
    Ok(ApplyReport {
        changed: changed.len(),
        confirmed,
        mismatches,
        rollback: None,
        backup: Some(backup_path),
        stale_check_errors,
        warnings,
    })
}

/// Validate the draft, then apply the difference from the baseline.
pub fn apply<T: Transport + ?Sized>(
    transport: &mut T,
    desc: &Descriptor,
    baseline: &DeviceState,
    draft: &DeviceState,
    ids: DeviceIds,
    paths: &Paths,
    settings: &ApplySettings,
) -> Result<ApplyReport, ApplyError> {
    let errors = state::validate_state(desc, draft);
    if !errors.is_empty() {
        return Err(ApplyError::Validation(errors));
    }
    let baseline_ops = state::encode_ops(desc, baseline);
    let target_ops = state::encode_ops(desc, draft);
    apply_ops(
        transport,
        desc,
        &baseline_ops,
        &target_ops,
        ids,
        paths,
        settings,
    )
}

/// Replay a backup through the same transaction engine.
pub fn apply_backup<T: Transport + ?Sized>(
    transport: &mut T,
    desc: &Descriptor,
    current: &DeviceState,
    backup: &BackupFile,
    ids: DeviceIds,
    paths: &Paths,
    settings: &ApplySettings,
) -> Result<ApplyReport, ApplyError> {
    if backup.descriptor_id != desc.id {
        return Err(ApplyError::Validation(vec![FieldError {
            field: "backup".into(),
            message: format!(
                "backup was taken for `{}`, the connected model is `{}`",
                backup.descriptor_id, desc.id
            ),
        }]));
    }
    let baseline = state::encode_ops(desc, current);
    let mut target = baseline.clone();
    for op in backup.ops() {
        match target.iter_mut().find(|existing| existing.id == op.id) {
            Some(slot) => *slot = op,
            None => target.push(op),
        }
    }
    apply_ops(transport, desc, &baseline, &target, ids, paths, settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::state::{seed_image, seed_state};
    use crate::transport::frames::{FrameHeader, PAYLOAD_OFFSET, REPORT_REGISTER};
    use crate::transport::sim::SimTransport;
    use crate::transport::FrameSizes;
    use std::sync::{Arc, Mutex};

    fn desc() -> Descriptor {
        Descriptor::from_toml(include_str!("../../models/holtek-04d9-fc5c.toml"))
            .expect("descriptor")
    }

    fn paths() -> (tempfile::TempDir, Paths) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = Paths {
            state_dir: tmp.path().to_path_buf(),
            config_dir: tmp.path().join("config"),
            runtime_dir: tmp.path().join("run"),
        };
        (tmp, paths)
    }

    fn sim_with(desc: &Descriptor) -> SimTransport {
        let image = seed_image(desc);
        let mut sim = SimTransport::new(desc.frame_sizes());
        for ((id, profile), window) in image.iter() {
            let _ = id;
            let _ = profile;
            sim.set_bytes(window.addr, &window.bytes);
        }
        sim
    }

    fn ids() -> DeviceIds {
        DeviceIds {
            vendor_id: 0x04d9,
            product_id: 0xfc5c,
        }
    }

    fn settings() -> ApplySettings {
        ApplySettings {
            pacer: Pacer::instant(),
            force_stale: false,
            active_profile: 0,
            label: "test".into(),
        }
    }

    #[test]
    fn confirmed_apply_writes_only_the_changed_register() {
        let desc = desc();
        let (_tmp, paths) = paths();
        let mut sim = sim_with(&desc);
        let baseline = seed_state(&desc);
        let mut draft = baseline.clone();
        draft.profiles[0].lighting.as_mut().unwrap().brightness = 0;
        let report = apply(
            &mut sim,
            &desc,
            &baseline,
            &draft,
            ids(),
            &paths,
            &settings(),
        )
        .expect("apply");
        assert_eq!(report.changed, 1);
        assert_eq!(report.confirmed, 1);
        assert!(report.fully_confirmed(), "{report:?}");
        assert!(report.backup.as_ref().map(|p| p.is_file()).unwrap_or(false));
        let writes: Vec<FrameHeader> = sim
            .frame_headers()
            .into_iter()
            .filter(|h| h.cmd == desc.commands.write)
            .collect();
        assert_eq!(writes.len(), 1, "only one register should be written");
        assert_eq!(writes[0].addr, 0x044F);
    }

    #[test]
    fn read_back_mismatch_is_reported_not_hidden() {
        let desc = desc();
        let (_tmp, paths) = paths();
        let sim = sim_with(&desc);
        let shared = Arc::new(Mutex::new(sim));
        let mut transport = CorruptingTransport {
            inner: shared,
            writes_before_corruption: 1,
            written: 0,
        };
        let baseline = seed_state(&desc);
        let mut draft = baseline.clone();
        draft.profiles[0].lighting.as_mut().unwrap().brightness = 0;
        let report = apply(
            &mut transport,
            &desc,
            &baseline,
            &draft,
            ids(),
            &paths,
            &settings(),
        )
        .expect("apply completes with warnings");
        assert_eq!(report.changed, 1);
        assert_eq!(report.confirmed, 0);
        assert!(report.mismatches[0].contains("read"), "{report:?}");
        assert!(!report.fully_confirmed());
    }

    #[test]
    fn stale_state_is_refused_unless_forced() {
        let desc = desc();
        let (_tmp, paths) = paths();
        let mut sim = sim_with(&desc);
        let baseline = seed_state(&desc);
        let mut draft = baseline.clone();
        draft.profiles[0].lighting.as_mut().unwrap().brightness = 0;
        // Someone else changes the brightness behind our back.
        sim.set_bytes(0x044F, &[2]);
        let err = apply(
            &mut sim,
            &desc,
            &baseline,
            &draft,
            ids(),
            &paths,
            &settings(),
        )
        .expect_err("stale");
        match err {
            ApplyError::Stale { differences } => {
                assert!(differences[0].contains("LED brightness"), "{differences:?}");
            }
            other => panic!("unexpected error: {other}"),
        }
        let mut forced = settings();
        forced.force_stale = true;
        let report = apply(&mut sim, &desc, &baseline, &draft, ids(), &paths, &forced)
            .expect("forced apply");
        assert_eq!(report.changed, 1);
    }

    #[test]
    fn mid_transaction_failure_rolls_back_written_registers() {
        let desc = desc();
        let (_tmp, paths) = paths();
        let mut sim = sim_with(&desc);
        sim.fail_write_at = Some(4); // begin, 2 writes, then fail
        let baseline = seed_state(&desc);
        let mut draft = baseline.clone();
        draft.profiles[0].lighting.as_mut().unwrap().brightness = 0;
        draft.profiles[0].polling_hz = Some(1000);
        let err = apply(
            &mut sim,
            &desc,
            &baseline,
            &draft,
            ids(),
            &paths,
            &settings(),
        )
        .expect_err("transport failure");
        match err {
            ApplyError::Transport { rollback, .. } => {
                let rollback = rollback.expect("rollback attempted");
                assert!(rollback.attempted > 0);
                assert!(rollback.restored > 0, "{rollback:?}");
            }
            other => panic!("unexpected error: {other}"),
        }
        // The first register written must be back at its baseline value.
        assert_eq!(sim.bytes_at(0x044F, 1), vec![3]);
    }

    #[test]
    fn validation_error_sends_nothing() {
        let desc = desc();
        let (_tmp, paths) = paths();
        let mut sim = sim_with(&desc);
        let baseline = seed_state(&desc);
        let mut draft = baseline.clone();
        draft.profiles[0].polling_hz = Some(2000);
        let err = apply(
            &mut sim,
            &desc,
            &baseline,
            &draft,
            ids(),
            &paths,
            &settings(),
        )
        .expect_err("validation");
        assert!(matches!(err, ApplyError::Validation(_)), "{err}");
        assert_eq!(sim.writes(), 0);
    }

    #[test]
    fn no_changes_is_not_an_apply() {
        let desc = desc();
        let (_tmp, paths) = paths();
        let mut sim = sim_with(&desc);
        let baseline = seed_state(&desc);
        let err = apply(
            &mut sim,
            &desc,
            &baseline,
            &baseline,
            ids(),
            &paths,
            &settings(),
        )
        .expect_err("no changes");
        assert!(matches!(err, ApplyError::NoChanges), "{err}");
    }

    #[test]
    fn profile_isolation_after_apply() {
        let desc = desc();
        let (_tmp, paths) = paths();
        let mut sim = sim_with(&desc);
        let baseline = seed_state(&desc);
        let other_before = sim.bytes_at(0x0042 + 0x00c0 * 2, 32);
        let mut draft = baseline.clone();
        draft.profiles[1].lighting.as_mut().unwrap().brightness = 1;
        apply(
            &mut sim,
            &desc,
            &baseline,
            &draft,
            ids(),
            &paths,
            &settings(),
        )
        .expect("apply");
        let other_after = sim.bytes_at(0x0042 + 0x00c0 * 2, 32);
        assert_eq!(other_before, other_after, "profile 3 must not change");
        assert_eq!(
            sim.profile(),
            baseline.active_profile,
            "the previously active profile must be restored"
        );
    }

    #[test]
    fn backup_replay_uses_the_same_engine() {
        let desc = desc();
        let (_tmp, paths) = paths();
        let mut sim = sim_with(&desc);
        let baseline = seed_state(&desc);
        let mut draft = baseline.clone();
        draft.profiles[0].lighting.as_mut().unwrap().brightness = 0;
        let report = apply(
            &mut sim,
            &desc,
            &baseline,
            &draft,
            ids(),
            &paths,
            &settings(),
        )
        .expect("apply");
        let backup = backup::load(report.backup.as_ref().unwrap()).expect("backup");
        let current = read_state(&mut sim, &desc, &Pacer::instant()).expect("read state");
        let restored = apply_backup(
            &mut sim,
            &desc,
            &current,
            &backup,
            ids(),
            &paths,
            &settings(),
        )
        .expect("restore");
        assert_eq!(restored.changed, 1);
        assert_eq!(sim.bytes_at(0x044F, 1), vec![3]);
    }

    #[test]
    fn inconsistent_reads_are_rejected() {
        let desc = desc();
        let sim = sim_with(&desc);
        let shared = Arc::new(Mutex::new(sim));
        let mut transport = DivergingTransport {
            inner: shared,
            reads: 0,
        };
        let err = read_state_consistent(&mut transport, &desc, &Pacer::instant())
            .expect_err("must be inconsistent");
        assert!(matches!(err, ReadError::Inconsistent(_)), "{err}");
    }

    #[test]
    fn blank_reads_are_rejected() {
        let desc = desc();
        let mut sim = SimTransport::new(desc.frame_sizes());
        let err = read_state_consistent(&mut sim, &desc, &Pacer::instant()).expect_err("blank");
        match err {
            ReadError::Inconsistent(message) => assert!(message.contains("filler"), "{message}"),
            other => panic!("unexpected error: {other}"),
        }
    }

    struct CorruptingTransport {
        inner: Arc<Mutex<SimTransport>>,
        writes_before_corruption: usize,
        written: usize,
    }

    impl Transport for CorruptingTransport {
        fn get_feature(&mut self, report_id: u8, len: usize) -> Result<Vec<u8>, TransportError> {
            let mut bytes = self
                .inner
                .lock()
                .expect("sim")
                .get_feature(report_id, len)?;
            if self.written >= self.writes_before_corruption && bytes.len() > PAYLOAD_OFFSET {
                bytes[PAYLOAD_OFFSET] ^= 0xFF;
            }
            Ok(bytes)
        }

        fn set_feature(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
            let header = FrameHeader::parse(bytes)?;
            if header.cmd == desc_write_cmd() {
                self.written += 1;
            }
            self.inner.lock().expect("sim").set_feature(bytes)
        }

        fn describe(&self) -> String {
            "corrupting simulation".into()
        }

        fn is_simulated(&self) -> bool {
            true
        }
    }

    fn desc_write_cmd() -> u8 {
        crate::transport::frames::CMD_WRITE
    }

    struct DivergingTransport {
        inner: Arc<Mutex<SimTransport>>,
        reads: usize,
    }

    impl Transport for DivergingTransport {
        fn get_feature(&mut self, report_id: u8, len: usize) -> Result<Vec<u8>, TransportError> {
            self.reads += 1;
            let mut bytes = self
                .inner
                .lock()
                .expect("sim")
                .get_feature(report_id, len)?;
            if self.reads > 3 && bytes.len() > PAYLOAD_OFFSET {
                bytes[PAYLOAD_OFFSET] = bytes[PAYLOAD_OFFSET].wrapping_add(1);
            }
            Ok(bytes)
        }

        fn set_feature(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
            self.inner.lock().expect("sim").set_feature(bytes)
        }

        fn describe(&self) -> String {
            "diverging simulation".into()
        }
    }

    #[test]
    fn register_report_is_used_for_writes() {
        let _ = REPORT_REGISTER;
        let desc = desc();
        assert_eq!(
            desc.frame_sizes(),
            FrameSizes::new([(2, 16), (3, 64), (4, 256)])
        );
    }
    #[test]
    fn fixture_apply_writes_combos_and_burst_fire() {
        use crate::settings::state::Burst;
        let desc = crate::models::fixture::descriptor();
        let (_tmp, paths) = paths();
        let image = seed_image(&desc);
        let mut sim = SimTransport::new(desc.frame_sizes());
        for (_, window) in image.iter() {
            sim.set_bytes(window.addr, &window.bytes);
        }
        let baseline = seed_state(&desc);
        let mut draft = baseline.clone();
        draft.profiles[0].buttons.as_mut().unwrap()[1].combo = vec!["ctrl".into(), "c".into()];
        draft.macros[0].burst = Some(Burst {
            bullets: 3,
            delay_ms: 40,
        });
        let report = apply(
            &mut sim,
            &desc,
            &baseline,
            &draft,
            ids(),
            &paths,
            &settings(),
        )
        .expect("apply");
        assert!(report.fully_confirmed(), "{report:?}");
        // Two combo slots and eight burst registers.
        assert_eq!(report.changed, 10);
        assert_eq!(sim.bytes_at(0x0c40 + 6, 2), vec![0xe0, 0x00]);
        assert_eq!(sim.bytes_at(0x0060, 2), vec![3, 4]);
    }
}
