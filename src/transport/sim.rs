//! In-memory simulator with the same framing semantics as the hidraw transport.
//!
//! Memory is byte-addressed per report ID. A `0xF3`/`0xF5` frame writes into
//! that memory; a `0xF2` request records the window, and the following
//! `get_feature` returns the window's bytes at the payload offset — exactly the
//! behaviour the read-consistency gate and apply verification rely on.

use std::collections::BTreeMap;
use std::path::Path;

use super::frames::{Frame, FrameHeader, CMD_BEGIN, CMD_READ, CMD_WRITE};
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
    writes: usize,
    reads: usize,
    pending_reads: BTreeMap<u8, (u16, u16)>,
    profile: u8,
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

    pub fn profile(&self) -> u8 {
        self.profile
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
        self.frames.push(Frame {
            report_id: header.report_id,
            bytes: bytes.to_vec(),
        });
        match header.cmd {
            CMD_BEGIN => {
                self.profile = header.addr as u8;
            }
            CMD_WRITE => {
                let payload = header.payload(bytes).to_vec();
                self.set_bytes(header.addr, &payload);
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
    use crate::transport::frames::{CMD_COMMIT, PAYLOAD_OFFSET, REPORT_REGISTER};

    fn sizes() -> FrameSizes {
        FrameSizes::new([(2, 16), (3, 64), (4, 1024)])
    }

    #[test]
    fn writes_then_reads_round_trip() {
        let mut sim = SimTransport::new(sizes());
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
        sim.fail_write_at = Some(1);
        let first = Frame::write(16, CMD_WRITE, 0x20, 1, &[1]).unwrap();
        sim.set_feature(&first.bytes).expect("first frame accepted");
        let second = Frame::write(16, CMD_WRITE, 0x21, 1, &[2]).unwrap();
        let err = sim
            .set_feature(&second.bytes)
            .expect_err("second frame fails");
        match err {
            TransportError::Simulated(idx) => assert_eq!(idx, 1),
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn begin_sets_the_selected_profile() {
        let mut sim = SimTransport::new(sizes());
        let begin = Frame::begin(16, REPORT_REGISTER, CMD_BEGIN, 2).unwrap();
        sim.set_feature(&begin.bytes).expect("begin");
        assert_eq!(sim.profile(), 2);
        let commit = Frame::commit(16, CMD_COMMIT, 1, 0, REPORT_REGISTER).unwrap();
        sim.set_feature(&commit.bytes).expect("commit");
        assert_eq!(sim.frames.len(), 2);
    }
}
