//! HID feature-report frame construction and parsing.
//!
//! Layout (verified against the reverse-engineering report §5.2 and the read
//! request pattern used by upstream tooling):
//!
//! ```text
//! byte:  0     1     2        3        4     5 6 7    8 .. N
//!      +-----+-----+--------+--------+-----+------+---------+
//!      | ID  | cmd | addr_lo| addr_hi| len | 0 0 0| payload |
//!      +-----+-----+--------+--------+-----+------+---------+
//! ```
//!
//! Commands: `0xF5` begin (low address byte is the profile), `0xF3` write,
//! `0xF2` read request, `0xF1` commit/switch. Reads replay the request's
//! header and put the requested bytes at the payload offset.

use std::fmt;

/// Payload begins after the eight-byte header in every report class.
pub const PAYLOAD_OFFSET: usize = 8;

pub const CMD_BEGIN: u8 = 0xF5;
pub const CMD_WRITE: u8 = 0xF3;
pub const CMD_READ: u8 = 0xF2;
pub const CMD_COMMIT: u8 = 0xF1;

pub const REPORT_REGISTER: u8 = 2;
pub const REPORT_BLOCK: u8 = 3;
pub const REPORT_MACRO: u8 = 4;

/// A complete feature report: report ID in byte 0, then the report body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub report_id: u8,
    pub bytes: Vec<u8>,
}

/// The eight-byte header common to all register-class frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameHeader {
    pub report_id: u8,
    pub cmd: u8,
    pub addr: u16,
    pub len: u8,
}

impl FrameHeader {
    pub fn parse(bytes: &[u8]) -> Result<Self, FrameError> {
        if bytes.len() < PAYLOAD_OFFSET {
            return Err(FrameError::TooShort {
                len: bytes.len(),
                needed: PAYLOAD_OFFSET,
            });
        }
        Ok(FrameHeader {
            report_id: bytes[0],
            cmd: bytes[1],
            addr: u16::from_le_bytes([bytes[2], bytes[3]]),
            len: bytes[4],
        })
    }

    /// Payload bytes carried by this frame (clamped to the frame length).
    pub fn payload<'a>(&self, bytes: &'a [u8]) -> &'a [u8] {
        let start = PAYLOAD_OFFSET.min(bytes.len());
        let end = (start + self.len as usize).min(bytes.len());
        &bytes[start..end]
    }
}

impl fmt::Display for FrameHeader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "report {} cmd {:#04x} addr {:#06x} len {}",
            self.report_id, self.cmd, self.addr, self.len
        )
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FrameError {
    #[error("frame too short: {len} bytes, need at least {needed}")]
    TooShort { len: usize, needed: usize },
    #[error("payload of {payload} bytes does not fit in a {total}-byte report")]
    PayloadTooLarge { payload: usize, total: usize },
    #[error("length marker {len} exceeds the payload capacity {}", .capacity)]
    LenTooLarge { len: u16, capacity: usize },
    #[error("unexpected report id {0}")]
    UnexpectedReport(u8),
    #[error("unknown command {0:#04x}")]
    UnknownCommand(u8),
    #[error("frame length {actual} does not match the report size {expected}")]
    LengthMismatch { actual: usize, expected: usize },
}

impl Frame {
    fn blank(total: usize, report_id: u8) -> Vec<u8> {
        let mut buf = vec![0u8; total];
        buf[0] = report_id;
        buf
    }

    fn finish(
        total: usize,
        report_id: u8,
        cmd: u8,
        addr: u16,
        len: u8,
        payload: &[u8],
    ) -> Result<Frame, FrameError> {
        if total <= PAYLOAD_OFFSET {
            return Err(FrameError::TooShort {
                len: total,
                needed: PAYLOAD_OFFSET + 1,
            });
        }
        if len as usize > total - PAYLOAD_OFFSET {
            return Err(FrameError::LenTooLarge {
                len: len as u16,
                capacity: total - PAYLOAD_OFFSET,
            });
        }
        if payload.len() > total - PAYLOAD_OFFSET {
            return Err(FrameError::PayloadTooLarge {
                payload: payload.len(),
                total,
            });
        }
        if payload.len() > len as usize {
            return Err(FrameError::PayloadTooLarge {
                payload: payload.len(),
                total: len as usize,
            });
        }
        let mut bytes = Self::blank(total, report_id);
        bytes[1] = cmd;
        bytes[2..4].copy_from_slice(&addr.to_le_bytes());
        bytes[4] = len;
        bytes[PAYLOAD_OFFSET..PAYLOAD_OFFSET + payload.len()].copy_from_slice(payload);
        Ok(Frame { report_id, bytes })
    }

    /// `0xF5` begin frame: the profile index travels in the low address byte.
    pub fn begin(total: usize, report_id: u8, cmd: u8, profile: u8) -> Result<Frame, FrameError> {
        if profile > 0x0F {
            return Err(FrameError::UnknownCommand(profile));
        }
        Self::finish(total, report_id, cmd, profile as u16, 0, &[])
    }

    /// `0xF3` register write at a 16-bit address.
    pub fn write(
        total: usize,
        cmd: u8,
        addr: u16,
        len: u8,
        payload: &[u8],
    ) -> Result<Frame, FrameError> {
        Self::finish(total, REPORT_REGISTER, cmd, addr, len, payload)
    }

    /// `0xF2` read request; the response repeats this header.
    pub fn read_request(
        total: usize,
        cmd: u8,
        addr: u16,
        len: u8,
        report_id: u8,
    ) -> Result<Frame, FrameError> {
        Self::finish(total, report_id, cmd, addr, len, &[])
    }

    /// `0xF1` commit / activate-profile frame.
    pub fn commit(
        total: usize,
        cmd: u8,
        addr: u16,
        len: u8,
        report_id: u8,
    ) -> Result<Frame, FrameError> {
        Self::finish(total, report_id, cmd, addr, len, &[])
    }

    /// Large block frame (scroll block or macro record) written to `addr`.
    pub fn block(
        total: usize,
        cmd: u8,
        addr: u16,
        len: u8,
        report_id: u8,
        body: &[u8],
    ) -> Result<Frame, FrameError> {
        Self::finish(total, report_id, cmd, addr, len, body)
    }

    pub fn header(&self) -> Result<FrameHeader, FrameError> {
        FrameHeader::parse(&self.bytes)
    }
}

/// Read a register window out of a response frame.
pub fn response_payload(bytes: &[u8], len: usize) -> &[u8] {
    let start = PAYLOAD_OFFSET.min(bytes.len());
    let end = (start + len).min(bytes.len());
    &bytes[start..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn begin_frame_carries_profile_in_low_address_byte() {
        let frame = Frame::begin(16, REPORT_REGISTER, CMD_BEGIN, 3).expect("begin frame");
        assert_eq!(
            frame.bytes,
            vec![0x02, 0xF5, 0x03, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn write_frame_places_payload_after_header() {
        let frame =
            Frame::write(16, CMD_WRITE, 0x0449, 4, &[0x11, 0x22, 0x33, 0x44]).expect("write frame");
        assert_eq!(&frame.bytes[..8], &[0x02, 0xF3, 0x49, 0x04, 0x04, 0, 0, 0]);
        assert_eq!(&frame.bytes[8..12], &[0x11, 0x22, 0x33, 0x44]);
        let header = frame.header().expect("header");
        assert_eq!(header.addr, 0x0449);
        assert_eq!(header.len, 4);
        assert_eq!(header.payload(&frame.bytes), &[0x11, 0x22, 0x33, 0x44]);
    }

    #[test]
    fn block_frame_accepts_full_body() {
        let body = vec![0xA5u8; 200];
        let frame =
            Frame::block(1024, CMD_WRITE, 0x0478, 200, REPORT_MACRO, &body).expect("macro frame");
        assert_eq!(frame.bytes.len(), 1024);
        assert_eq!(frame.bytes[0], 0x04);
        assert_eq!(&frame.bytes[2..4], &[0x78, 0x04]);
        assert_eq!(&frame.bytes[8..208], &body[..]);
        assert_eq!(frame.bytes[208], 0);
    }

    #[test]
    fn oversized_payload_is_rejected() {
        let err = Frame::write(16, CMD_WRITE, 0x20, 64, &[0; 9]).unwrap_err();
        assert!(matches!(err, FrameError::LenTooLarge { .. }));
    }

    #[test]
    fn response_payload_extracts_requested_bytes() {
        let mut bytes = vec![0u8; 16];
        bytes[0] = 2;
        bytes[8..12].copy_from_slice(&[1, 2, 3, 4]);
        assert_eq!(response_payload(&bytes, 4), &[1, 2, 3, 4]);
    }
}
