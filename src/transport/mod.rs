//! Feature-report transports: a real hidraw node or an in-memory simulation.

pub mod frames;
pub mod hidraw;
pub mod sim;

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use frames::{Frame, FrameError, FrameHeader, PAYLOAD_OFFSET};

pub use frames::{CMD_BEGIN, CMD_COMMIT, CMD_READ, CMD_WRITE};

/// Report sizes declared by a model descriptor, keyed by report ID.
///
/// The descriptor is not limited to three reports: devices expose different
/// feature-report tables (this project's M721-Pro has 16/64/256/1024/2048-byte
/// reports with IDs 2-6), so the map is data.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FrameSizes {
    sizes: BTreeMap<u8, usize>,
}

impl FrameSizes {
    pub fn new(entries: impl IntoIterator<Item = (u8, usize)>) -> Self {
        FrameSizes {
            sizes: entries.into_iter().collect(),
        }
    }

    pub fn for_report(&self, report_id: u8) -> Option<usize> {
        self.sizes.get(&report_id).copied()
    }

    /// Largest payload a report can carry.
    pub fn payload_capacity(&self, report_id: u8) -> Option<usize> {
        self.for_report(report_id)
            .map(|total| total.saturating_sub(PAYLOAD_OFFSET))
    }

    pub fn iter(&self) -> impl Iterator<Item = (&u8, &usize)> {
        self.sizes.iter()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("permission denied opening {path}")]
    PermissionDenied {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("device node {0} does not exist")]
    NotFound(PathBuf),
    #[error("device node {0} is in use by another process")]
    Busy(PathBuf),
    #[error("hidraw transfer failed: {0}")]
    Io(#[from] io::Error),
    #[error("short feature report: expected {expected} bytes, got {got}")]
    ShortReport { expected: usize, got: usize },
    #[error("feature report {0} is not declared by the model")]
    UnknownReport(u8),
    #[error("{0}")]
    Frame(#[from] FrameError),
    #[error("simulated transport failure at frame {0}")]
    Simulated(usize),
    #[error("simulated read failure at read {0}")]
    SimulatedRead(usize),
}

/// The single seam through which every device operation goes.
pub trait Transport: Send {
    /// Fetch a feature report. The returned buffer is the whole report,
    /// including the report ID in byte 0.
    fn get_feature(&mut self, report_id: u8, len: usize) -> Result<Vec<u8>, TransportError>;
    /// Send a complete feature report (report ID in byte 0).
    fn set_feature(&mut self, bytes: &[u8]) -> Result<(), TransportError>;
    /// Node path for diagnostics, if backed by real hardware.
    fn node_path(&self) -> Option<&Path> {
        None
    }
    /// Human-readable description for logs and the diagnostics screen.
    fn describe(&self) -> String;
    fn is_simulated(&self) -> bool {
        false
    }
}

/// Inter-frame pacing. Production values come from the reverse-engineering
/// report: 10 ms between frames, 100 ms after the scroll-speed block.
#[derive(Clone, Copy, Debug)]
pub struct Pacer {
    pub frame_delay: Duration,
    pub block_delay: Duration,
}

impl Pacer {
    pub fn production() -> Self {
        Pacer {
            frame_delay: Duration::from_millis(10),
            block_delay: Duration::from_millis(100),
        }
    }

    pub fn instant() -> Self {
        Pacer {
            frame_delay: Duration::ZERO,
            block_delay: Duration::ZERO,
        }
    }

    pub fn wait(&self, report_id: u8) {
        let delay = if report_id == frames::REPORT_BLOCK {
            self.block_delay
        } else {
            self.frame_delay
        };
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
    }
}

/// Send one frame and apply pacing.
pub fn send_frame<T: Transport + ?Sized>(
    transport: &mut T,
    frame: &Frame,
    pacer: &Pacer,
) -> Result<(), TransportError> {
    transport.set_feature(&frame.bytes)?;
    pacer.wait(frame.report_id);
    Ok(())
}

/// Read a register window: send the `0xF2` selector, then fetch the report.
///
/// Returns the requested bytes (payload of the response frame).
pub fn read_window<T: Transport + ?Sized>(
    transport: &mut T,
    sizes: &FrameSizes,
    read_cmd: u8,
    report_id: u8,
    addr: u16,
    len: u16,
    pacer: &Pacer,
) -> Result<Vec<u8>, TransportError> {
    let total = sizes
        .for_report(report_id)
        .ok_or(TransportError::UnknownReport(report_id))?;
    if len as usize > total - PAYLOAD_OFFSET {
        return Err(TransportError::Frame(FrameError::LenTooLarge {
            len,
            capacity: total - PAYLOAD_OFFSET,
        }));
    }
    let request = Frame::read_request(total, read_cmd, addr, len as u8, report_id)?;
    send_frame(transport, &request, pacer)?;
    let response = transport.get_feature(report_id, total)?;
    let needed = PAYLOAD_OFFSET + len as usize;
    if response.len() < needed {
        return Err(TransportError::ShortReport {
            expected: needed,
            got: response.len(),
        });
    }
    Ok(frames::response_payload(&response, len as usize).to_vec())
}

/// Read the whole register space described by a set of window instances.
///
/// The first frame selects the profile page with `0xF5` when `begin` is given,
/// mirroring the sequence the vendor application uses before reading.
pub fn begin_profile<T: Transport + ?Sized>(
    transport: &mut T,
    sizes: &FrameSizes,
    report_id: u8,
    begin_cmd: u8,
    profile: u8,
    pacer: &Pacer,
) -> Result<(), TransportError> {
    let total = sizes
        .for_report(report_id)
        .ok_or(TransportError::UnknownReport(report_id))?;
    let frame = Frame::begin(total, report_id, begin_cmd, profile)?;
    send_frame(transport, &frame, pacer)
}

/// Send the commit frame that finalises a write batch.
pub fn commit<T: Transport + ?Sized>(
    transport: &mut T,
    sizes: &FrameSizes,
    commit_cmd: u8,
    addr: u16,
    len: u8,
    report_id: u8,
    pacer: &Pacer,
) -> Result<(), TransportError> {
    let total = sizes
        .for_report(report_id)
        .ok_or(TransportError::UnknownReport(report_id))?;
    let frame = Frame::commit(total, commit_cmd, addr, len, report_id)?;
    send_frame(transport, &frame, pacer)
}

/// Send every commit frame of a descriptor's sequence, in order.
pub fn commit_all<T: Transport + ?Sized>(
    transport: &mut T,
    sizes: &FrameSizes,
    desc: &crate::models::Descriptor,
    addresses: &[u16],
    pacer: &Pacer,
) -> Result<(), TransportError> {
    for addr in addresses {
        commit(
            transport,
            sizes,
            desc.commands.commit,
            *addr,
            0,
            desc.commands.commit_report,
            pacer,
        )?;
    }
    Ok(())
}

/// Parse a response header for diagnostic output.
pub fn describe_response(bytes: &[u8]) -> String {
    match FrameHeader::parse(bytes) {
        Ok(header) => header.to_string(),
        Err(err) => format!("unparsable response: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_capacity_follows_report_size() {
        let sizes = FrameSizes::new([(2, 16), (3, 64), (4, 1024)]);
        assert_eq!(sizes.payload_capacity(2), Some(8));
        assert_eq!(sizes.payload_capacity(3), Some(56));
        assert_eq!(sizes.payload_capacity(4), Some(1016));
        assert_eq!(sizes.payload_capacity(9), None);
    }
}
