#[cfg(feature = "once_cell_try")]
use std::sync::OnceLock;

use io_uring::squeue::Entry;
use linux_raw_sys::io_uring::{IORING_ACCEPT_POLL_FIRST, IORING_RECVSEND_POLL_FIRST, io_uring_sqe};
#[cfg(not(feature = "once_cell_try"))]
use once_cell::sync::OnceCell as OnceLock;

use crate::{IoUringFeatures, is_feature_supported};

pub fn is_op_supported(code: u8) -> bool {
    static PROBE: OnceLock<io_uring::Probe> = OnceLock::new();

    PROBE
        .get_or_try_init(|| {
            let mut probe = io_uring::Probe::new();

            io_uring::IoUring::new(2)?
                .submitter()
                .register_probe(&mut probe)?;

            std::io::Result::Ok(probe)
        })
        .map(|probe| probe.is_supported(code))
        .unwrap_or_default()
}

/// The kernel version of Linux.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct KernelVersion {
    pub major: u8,
    pub minor: u8,
}

impl From<(u8, u8)> for KernelVersion {
    fn from((major, minor): (u8, u8)) -> Self {
        Self { major, minor }
    }
}

/// Returns the kernel version of Linux, or `None` if it cannot be determined.
fn kernel_version() -> Option<KernelVersion> {
    static VERSION: OnceLock<Option<KernelVersion>> = OnceLock::new();

    *VERSION.get_or_init(|| {
        let info = rustix::system::uname();
        let version = info.release().to_str().ok()?;
        let mut parts = version.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        Some(KernelVersion { major, minor })
    })
}

pub fn is_kernel_at_least(v: impl Into<KernelVersion>) -> bool {
    kernel_version()
        .map(|kv| kv >= v.into())
        .unwrap_or_default()
}

pub(crate) fn set_poll_first(mut entry: Entry, flag: bool) -> Entry {
    let (ioprio, feat) = match entry.get_opcode() as u8 {
        io_uring::opcode::Accept::CODE => {
            (IORING_ACCEPT_POLL_FIRST, IoUringFeatures::ACCEPT_POLL_FIRST)
        }
        _ => (
            IORING_RECVSEND_POLL_FIRST,
            IoUringFeatures::RECVSEND_POLL_FIRST,
        ),
    };
    if flag && is_feature_supported(feat) {
        let sqe = &raw mut entry as *mut io_uring_sqe;
        unsafe {
            (*sqe).ioprio |= ioprio as u16;
        }
    }
    entry
}
