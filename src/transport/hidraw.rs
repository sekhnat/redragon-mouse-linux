//! Real hidraw transport: `HIDIOCGFEATURE` / `HIDIOCSFEATURE` on a device node.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

use super::{Transport, TransportError};

// Linux ioctl encoding (asm-generic/ioctl.h).
const IOC_NRBITS: u32 = 8;
const IOC_TYPEBITS: u32 = 8;
const IOC_SIZEBITS: u32 = 14;
const IOC_NRSHIFT: u32 = 0;
const IOC_TYPESHIFT: u32 = IOC_NRSHIFT + IOC_NRBITS;
const IOC_SIZESHIFT: u32 = IOC_TYPESHIFT + IOC_TYPEBITS;
const IOC_DIRSHIFT: u32 = IOC_SIZESHIFT + IOC_SIZEBITS;
const IOC_WRITE: u32 = 1;
const IOC_READ: u32 = 2;

const fn ioc(dir: u32, ty: u8, nr: u8, size: usize) -> libc::c_ulong {
    ((dir as libc::c_ulong) << IOC_DIRSHIFT)
        | ((ty as libc::c_ulong) << IOC_TYPESHIFT)
        | ((nr as libc::c_ulong) << IOC_NRSHIFT)
        | ((size as libc::c_ulong) << IOC_SIZESHIFT)
}

/// `_IOC(_IOC_READ|_IOC_WRITE, 'H', 0x06, len)` — HIDIOCSFEATURE
pub const fn hidiocsfeature(len: usize) -> libc::c_ulong {
    ioc(IOC_READ | IOC_WRITE, b'H', 0x06, len)
}

/// `_IOC(_IOC_READ|_IOC_WRITE, 'H', 0x07, len)` — HIDIOCGFEATURE
pub const fn hidhiocgfeature(len: usize) -> libc::c_ulong {
    ioc(IOC_READ | IOC_WRITE, b'H', 0x07, len)
}

/// `_IOR('H', 0x03, struct hidraw_devinfo)` — HIDIOCGRAWINFO
pub const fn hidraw_grawinfo() -> libc::c_ulong {
    ioc(IOC_READ, b'H', 0x03, std::mem::size_of::<HidrawDevinfo>())
}

/// `_IOR('H', 0x01, int)` — HIDIOCGRDESCSIZE
pub const fn hidraw_grdescsize() -> libc::c_ulong {
    ioc(IOC_READ, b'H', 0x01, std::mem::size_of::<libc::c_int>())
}

/// `_IOR('H', 0x02, struct hidraw_report_descriptor)` — HIDIOCGRDESC
pub const fn hidraw_grdesc(size: usize) -> libc::c_ulong {
    ioc(IOC_READ, b'H', 0x02, size)
}

/// `struct hidraw_devinfo` from linux/hidraw.h.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct HidrawDevinfo {
    pub bustype: u32,
    pub vendor: i16,
    pub product: i16,
}

/// Maximum report-descriptor size accepted by the kernel.
pub const HID_MAX_DESCRIPTOR_SIZE: usize = 4096;

/// `struct hidraw_report_descriptor` from linux/hidraw.h (size + 4096 bytes).
#[repr(C)]
pub struct HidrawReportDescriptor {
    pub size: u32,
    pub value: [u8; HID_MAX_DESCRIPTOR_SIZE],
}

impl Default for HidrawReportDescriptor {
    fn default() -> Self {
        HidrawReportDescriptor {
            size: 0,
            value: [0u8; HID_MAX_DESCRIPTOR_SIZE],
        }
    }
}

/// A hidraw node opened for configuration traffic.
#[derive(Debug)]
pub struct HidrawTransport {
    file: File,
    node: PathBuf,
}

impl HidrawTransport {
    pub fn open(node: &Path) -> Result<Self, TransportError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(node)
            .map_err(|err| match err.kind() {
                io::ErrorKind::PermissionDenied => TransportError::PermissionDenied {
                    path: node.to_path_buf(),
                    source: err,
                },
                io::ErrorKind::NotFound => TransportError::NotFound(node.to_path_buf()),
                _ if err.raw_os_error() == Some(libc::EBUSY) => {
                    TransportError::Busy(node.to_path_buf())
                }
                _ => TransportError::Io(err),
            })?;
        Ok(HidrawTransport {
            file,
            node: node.to_path_buf(),
        })
    }

    fn ioctl(&self, request: libc::c_ulong, buf: *mut u8) -> io::Result<libc::c_int> {
        let rc = unsafe { libc::ioctl(self.file.as_raw_fd(), request as _, buf) };
        if rc < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(rc)
        }
    }

    /// Read `HIDIOCGRAWINFO` for a node (vendor/product id, bus type).
    pub fn raw_info(node: &Path) -> io::Result<HidrawDevinfo> {
        let file = OpenOptions::new().read(true).open(node)?;
        let mut info = HidrawDevinfo::default();
        let req = hidraw_grawinfo();
        let rc =
            unsafe { libc::ioctl(file.as_raw_fd(), req as _, &mut info as *mut HidrawDevinfo) };
        if rc < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(info)
        }
    }

    /// Read the report descriptor exposed by the node.
    pub fn report_descriptor(node: &Path) -> io::Result<Vec<u8>> {
        let file = OpenOptions::new().read(true).open(node)?;
        let mut size: libc::c_int = 0;
        let req = hidraw_grdescsize();
        let rc = unsafe { libc::ioctl(file.as_raw_fd(), req as _, &mut size) };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        if size <= 0 || size as usize > HID_MAX_DESCRIPTOR_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("implausible report descriptor size {size}"),
            ));
        }
        let mut desc = HidrawReportDescriptor {
            size: size as u32,
            ..Default::default()
        };
        let req = hidraw_grdesc(std::mem::size_of::<HidrawReportDescriptor>());
        let rc = unsafe { libc::ioctl(file.as_raw_fd(), req as _, &mut desc) };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(desc.value[..desc.size as usize].to_vec())
    }
}

impl Transport for HidrawTransport {
    fn get_feature(&mut self, report_id: u8, len: usize) -> Result<Vec<u8>, TransportError> {
        if len == 0 {
            return Err(TransportError::Frame(super::FrameError::TooShort {
                len,
                needed: 1,
            }));
        }
        let mut buf = vec![0u8; len];
        buf[0] = report_id;
        let req = hidhiocgfeature(len);
        let rc = self.ioctl(req, buf.as_mut_ptr())?;
        let got = rc.max(0) as usize;
        if got == 0 {
            return Err(TransportError::ShortReport { expected: len, got });
        }
        // The device answers with a report sized to the requested window
        // (header + payload), not the full declared report. Keep exactly the
        // bytes the kernel reports as valid.
        buf.truncate(got.min(len));
        Ok(buf)
    }

    fn set_feature(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
        if bytes.is_empty() {
            return Err(TransportError::Frame(super::FrameError::TooShort {
                len: 0,
                needed: 1,
            }));
        }
        let mut buf = bytes.to_vec();
        let req = hidiocsfeature(buf.len());
        self.ioctl(req, buf.as_mut_ptr())?;
        Ok(())
    }

    fn node_path(&self) -> Option<&Path> {
        Some(&self.node)
    }

    fn describe(&self) -> String {
        format!("hidraw {}", self.node.display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ioctl_numbers_match_linux_hidraw_header() {
        // Values computed from the macros in linux/hidraw.h.
        assert_eq!(hidraw_grawinfo(), 0x80084803);
        assert_eq!(hidraw_grdescsize(), 0x80044801);
        assert_eq!(hidiocsfeature(16), 0xc0104806);
        assert_eq!(hidhiocgfeature(16), 0xc0104807);
        assert_eq!(hidhiocgfeature(1024), 0xc4004807);
    }

    #[test]
    fn missing_node_is_reported_as_not_found() {
        let err = HidrawTransport::open(Path::new("/dev/definitely-not-a-hidraw-node"))
            .expect_err("must fail");
        assert!(matches!(err, TransportError::NotFound(_)), "{err}");
    }
}
