#[cfg(feature = "once_cell_try")]
use std::sync::OnceLock;

#[cfg(not(feature = "once_cell_try"))]
use once_cell::sync::OnceCell as OnceLock;

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
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl From<(u64, u64, u64)> for KernelVersion {
    fn from((major, minor, patch): (u64, u64, u64)) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }
}

/// Returns the kernel version of Linux, or `None` if it cannot be determined.
fn kernel_version() -> Option<KernelVersion> {
    static VERSION: OnceLock<Option<KernelVersion>> = OnceLock::new();

    *VERSION.get_or_init(|| {
        let info = rustix::system::uname();
        let version = info.release().to_str().ok()?;
        let mut parts = version.splitn(3, '.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch_str = parts.next()?;
        let patch_str_len = patch_str
            .find(|c: char| !c.is_numeric())
            .unwrap_or(patch_str.len());
        let patch = patch_str[..patch_str_len].parse().ok()?;
        Some(KernelVersion {
            major,
            minor,
            patch,
        })
    })
}

pub fn is_kernel_at_least(v: impl Into<KernelVersion>) -> bool {
    kernel_version()
        .map(|kv| kv >= v.into())
        .unwrap_or_default()
}
