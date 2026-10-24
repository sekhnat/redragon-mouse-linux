//! In-memory simulator with the same framing semantics as the hidraw transport.
//!
//! Memory is byte-addressed. A `0xF3` frame writes into that memory; a `0xF2`
//! request records the window, and the following `get_feature` returns the
//! window's bytes at the payload offset — exactly the behaviour the
//! read-consistency gate and apply verification rely on.
//!
//! The simulator also enforces the session rules the real firmware is silent
//! about: reads and writes must sit between `F5 00` and `F5 01`, a refresh
//! frame must be `{F1 02 <single-bit flag>}`, and writes that are never
//! followed by a refresh are counted in [`SimTransport::lost_writes`] because
//! the real mouse stores them but does not apply them.

use std::collections::BTreeMap;
use std::path::Path;

use super::frames::{Frame, FrameHeader, CMD_BEGIN, CMD_COMMIT, CMD_READ, CMD_WRITE};
use super::{FrameSizes, Transport, TransportError};

#[derive(Clone, Debug, Default)]
pub struct SimTransport {
    pub sizes: FrameSizes,
    /// Address -> byte. The device exposes one register space; the report ID
    /// only frames the transfer, so writes through report 2 are visible to a
    /// read through report 3 at the same address.
    pub memory: BTreeMap<u16, u8>,
    /// Every frame sent, in order (useful for assertions).
    pub frames: Vec<Frame>,
    /// Fail the write at this zero-based frame index.
    pub fail_write_at: Option<usize>,
    /// Fail the read at this zero-based read index.
    pub fail_read_at: Option<usize>,
    /// Writes made in a session that closed without a refresh frame.
    pub lost_writes: usize,
    /// Refresh flags received, in order.
    pub refresh_flags: Vec<u8>,
    /// Number of sessions opened.
    pub sessions: usize,
    writes: usize,
    reads: usize,
    pending_reads: BTreeMap<u8, (u16, u16)>,
    in_session: bool,
    dirty: bool,
}

impl SimTransport {
    pub fn new(sizes: FrameSizes) -> Self {
        SimTransport {
            sizes,
            ..Default::default()
        }
    }

    pub fn with_memory(sizes: FrameSizes, memory: BTreeMap<u16, u8>) -> Self {
        SimTransport {
            sizes,
            memory,
            ..Default::default()
        }
    }

    pub fn set_bytes(&mut self, addr: u16, bytes: &[u8]) {
        for (i, byte) in bytes.iter().enumerate() {
            self.memory.insert(addr.wrapping_add(i as u16), *byte);
        }
    }

    pub fn bytes_at(&self, addr: u16, len: usize) -> Vec<u8> {
        (0..len)
            .map(|i| {
                self.memory
                    .get(&addr.wrapping_add(i as u16))
                    .copied()
                    .unwrap_or(0)
            })
            .collect()
    }

    pub fn frame_headers(&self) -> Vec<FrameHeader> {
        self.frames.iter().filter_map(|f| f.header().ok()).collect()
    }

    pub fn writes(&self) -> usize {
        self.writes
    }

    pub fn in_session(&self) -> bool {
        self.in_session
    }
}

impl Transport for SimTransport {
    fn get_feature(&mut self, report_id: u8, len: usize) -> Result<Vec<u8>, TransportError> {
        let expected = self
            .sizes
            .for_report(report_id)
            .ok_or(TransportError::UnknownReport(report_id))?;
        if len != expected {
            return Err(TransportError::ShortReport { expected, got: len });
        }
        if self.fail_read_at == Some(self.reads) {
            return Err(TransportError::SimulatedRead(self.reads));
        }
        self.reads += 1;
        let (addr, req_len) = self
            .pending_reads
            .get(&report_id)
            .copied()
            .unwrap_or((0, 0));
        let mut buf = vec![0u8; len];
        buf[0] = report_id;
        let data = self.bytes_at(addr, req_len as usize);
        let end = (super::frames::PAYLOAD_OFFSET + data.len()).min(len);
        buf[super::frames::PAYLOAD_OFFSET..end]
            .copy_from_slice(&data[..end - super::frames::PAYLOAD_OFFSET]);
        Ok(buf)
    }

    fn set_feature(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
        let header = FrameHeader::parse(bytes)?;
        let expected = self
            .sizes
            .for_report(header.report_id)
            .ok_or(TransportError::UnknownReport(header.report_id))?;
        if bytes.len() != expected {
            return Err(TransportError::Frame(super::FrameError::LengthMismatch {
                actual: bytes.len(),
                expected,
            }));
        }
        let index = self.writes;
        self.writes += 1;
        if self.fail_write_at == Some(index) {
            // Only this frame fails; a rollback retry reaches the device.
            return Err(TransportError::Simulated(index));
        }
        let needs_session = matches!(header.cmd, CMD_WRITE | CMD_READ | CMD_COMMIT);
        if needs_session && !self.in_session {
            return Err(TransportError::Protocol(format!(
                "frame {:#04x} sent outside an F5 00 .. F5 01 session",
                header.cmd
            )));
        }
        self.frames.push(Frame {
            report_id: header.report_id,
            bytes: bytes.to_vec(),
        });
        match header.cmd {
            CMD_BEGIN => match bytes[2] {
                0 if self.in_session => {
                    return Err(TransportError::Protocol("session is already open".into()));
                }
                0 => {
                    self.in_session = true;
                    self.sessions += 1;
                }
                1 if !self.in_session => {
                    return Err(TransportError::Protocol(
                        "session closed without being opened".into(),
                    ));
                }
                1 => {
                    if self.dirty {
                        self.lost_writes += 1;
                    }
                    self.in_session = false;
                    self.dirty = false;
                }
                other => {
                    return Err(TransportError::Protocol(format!(
                        "unknown session argument {other:#04x}"
                    )));
                }
            },
            CMD_WRITE => {
                let payload = header.payload(bytes).to_vec();
                self.set_bytes(header.addr, &payload);
                self.dirty = true;
            }
            CMD_COMMIT => {
                let (mask, flag) = (bytes[2], bytes[3]);
                if mask != 0x02 || flag == 0 || !flag.is_power_of_two() || flag > 0x10 {
                    return Err(TransportError::Protocol(format!(
                        "malformed refresh frame {mask:#04x} {flag:#04x}"
                    )));
                }
                self.refresh_flags.push(flag);
                self.dirty = false;
            }
            CMD_READ => {
                self.pending_reads
                    .insert(header.report_id, (header.addr, header.len as u16));
            }
            _ => {}
        }
        Ok(())
    }

    fn describe(&self) -> String {
        format!(
            "simulation ({} frames recorded, {} bytes addressed)",
            self.frames.len(),
            self.memory.len()
        )
    }

    fn is_simulated(&self) -> bool {
        true
    }

    fn node_path(&self) -> Option<&Path> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::frames::{PAYLOAD_OFFSET, REPORT_REGISTER};

    fn sizes() -> FrameSizes {
        FrameSizes::new([(2, 16), (3, 64), (4, 1024)])
    }

    fn open(sim: &mut SimTransport) {
        let frame = Frame::begin(16, REPORT_REGISTER, CMD_BEGIN, 0).unwrap();
        sim.set_feature(&frame.bytes).expect("open");
    }

    fn close(sim: &mut SimTransport) {
        let frame = Frame::begin(16, REPORT_REGISTER, CMD_BEGIN, 1).unwrap();
        sim.set_feature(&frame.bytes).expect("close");
    }

    #[test]
    fn writes_then_reads_round_trip() {
        let mut sim = SimTransport::new(sizes());
        open(&mut sim);
        sim.set_feature(
            &Frame::write(16, CMD_WRITE, 0x0449, 3, &[0xAA, 0xBB, 0xCC])
                .unwrap()
                .bytes,
        )
        .expect("write");
        sim.set_feature(
            &Frame::read_request(16, CMD_READ, 0x0449, 3, REPORT_REGISTER)
                .unwrap()
                .bytes,
        )
        .expect("read request");
        let response = sim.get_feature(REPORT_REGISTER, 16).expect("read");
        assert_eq!(
            &response[PAYLOAD_OFFSET..PAYLOAD_OFFSET + 3],
            &[0xAA, 0xBB, 0xCC]
        );
    }

    #[test]
    fn injected_failure_reports_the_frame_index() {
        let mut sim = SimTransport::new(sizes());
        sim.fail_write_at = Some(2);
        open(&mut sim);
        let first = Frame::write(16, CMD_WRITE, 0x20, 1, &[1]).unwrap();
        sim.set_feature(&first.bytes).expect("first frame accepted");
        let second = Frame::write(16, CMD_WRITE, 0x21, 1, &[2]).unwrap();
        let err = sim
            .set_feature(&second.bytes)
            .expect_err("second frame fails");
        match err {
            TransportError::Simulated(idx) => assert_eq!(idx, 2),
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn writes_outside_a_session_are_rejected() {
        let mut sim = SimTransport::new(sizes());
        let write = Frame::write(16, CMD_WRITE, 0x20, 1, &[1]).unwrap();
        let err = sim.set_feature(&write.bytes).expect_err("no session");
        assert!(matches!(err, TransportError::Protocol(_)), "{err}");
    }

    #[test]
    fn refresh_must_have_a_single_bit_flag_in_byte_three() {
        let mut sim = SimTransport::new(sizes());
        open(&mut sim);
        // The old, wrong encoding: flag in the high address byte slot swapped.
        let swapped = Frame::commit(16, CMD_COMMIT, 0x0802, REPORT_REGISTER).unwrap();
        let err = sim.set_feature(&swapped.bytes).expect_err("swapped");
        assert!(matches!(err, TransportError::Protocol(_)), "{err}");
        let good = Frame::commit(16, CMD_COMMIT, 0x0208, REPORT_REGISTER).unwrap();
        sim.set_feature(&good.bytes).expect("refresh");
        assert_eq!(sim.refresh_flags, vec![0x08]);
    }

    #[test]
    fn writes_without_a_refresh_are_counted_as_lost() {
        let mut sim = SimTransport::new(sizes());
        open(&mut sim);
        let write = Frame::write(16, CMD_WRITE, 0x20, 1, &[1]).unwrap();
        sim.set_feature(&write.bytes).expect("write");
        close(&mut sim);
        assert_eq!(sim.lost_writes, 1);
        assert!(!sim.in_session());
    }
}
