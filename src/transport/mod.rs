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
    Unsupported(&'static str),
    #[error("{0}")]
    Frame(#[from] FrameError),
    #[error("simulated transport failure at frame {0}")]
    Simulated(usize),
    #[error("simulated read failure at read {0}")]
    SimulatedRead(usize),
    #[error("simulated protocol violation: {0}")]
    Protocol(String),
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
///
/// `block_delay` applies to *writes* on the 64-byte report only; read requests
/// always use `frame_delay`, because the 100 ms settle time belongs to the
/// scroll-speed block write and made every profile-page read ten times slower.
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
    transport.set_feature(&request.bytes)?;
    if !pacer.frame_delay.is_zero() {
        std::thread::sleep(pacer.frame_delay);
    }
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

/// Send one `0xF5` session frame: `closing == false` opens, `true` closes.
pub fn session_frame<T: Transport + ?Sized>(
    transport: &mut T,
    sizes: &FrameSizes,
    report_id: u8,
    begin_cmd: u8,
    closing: bool,
    pacer: &Pacer,
) -> Result<(), TransportError> {
    let total = sizes
        .for_report(report_id)
        .ok_or(TransportError::UnknownReport(report_id))?;
    let frame = Frame::begin(total, report_id, begin_cmd, closing as u8)?;
    send_frame(transport, &frame, pacer)
}

/// Run `body` inside a configuration session (`F5 00` ... `F5 01`).
///
/// The closing frame is always sent, even when `body` fails: without it the
/// mouse stays in its configuration state and stops reporting pointer input.
/// `body`'s error wins over a close error.
pub fn with_session<T: Transport + ?Sized, R>(
    transport: &mut T,
    desc: &crate::models::Descriptor,
    pacer: &Pacer,
    body: impl FnOnce(&mut T) -> Result<R, TransportError>,
) -> Result<R, TransportError> {
    let sizes = desc.frame_sizes();
    let report = desc.commands.begin_report;
    let cmd = desc.commands.begin;
    session_frame(transport, &sizes, report, cmd, false, pacer)?;
    let result = body(transport);
    let closed = session_frame(transport, &sizes, report, cmd, true, pacer);
    let value = result?;
    closed?;
    Ok(value)
}

/// Send one refresh frame (see [`Frame::commit`] for the word layout).
pub fn commit<T: Transport + ?Sized>(
    transport: &mut T,
    sizes: &FrameSizes,
    commit_cmd: u8,
    word: u16,
    report_id: u8,
    pacer: &Pacer,
) -> Result<(), TransportError> {
    let total = sizes
        .for_report(report_id)
        .ok_or(TransportError::UnknownReport(report_id))?;
    let frame = Frame::commit(total, commit_cmd, word, report_id)?;
    send_frame(transport, &frame, pacer)
}

/// Send every refresh word of a descriptor's sequence, in order.
pub fn commit_all<T: Transport + ?Sized>(
    transport: &mut T,
    sizes: &FrameSizes,
    desc: &crate::models::Descriptor,
    words: &[u16],
    pacer: &Pacer,
) -> Result<(), TransportError> {
    for word in words {
        commit(
            transport,
            sizes,
            desc.commands.commit,
            *word,
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
