#[cfg(feature = "once_cell_try")]
use std::sync::OnceLock;
#[cfg(io_uring)]
use std::sync::atomic::{AtomicU32, Ordering};

#[cfg(all(io_uring, not(feature = "once_cell_try")))]
use once_cell::sync::OnceCell as OnceLock;

#[cfg(io_uring)]
use crate::sys::pal::is_kernel_at_least;

bitflags::bitflags! {
    /// Advanced io_uring features that can be queried or forced on.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct IoUringFeatures: u32 {
        /// Network recv/send poll-first flag (>= 5.19)
        const RECVSEND_POLL_FIRST = 1 << 0;
        /// Multishot accept (>= 5.19)
        const MULTISHOT_ACCEPT    = 1 << 1;
        /// Multishot receive with buffer pool (>= 6.0)
        const MULTISHOT_RECV      = 1 << 2;
        /// Multishot recvmsg with io_uring_recvmsg_out (>= 6.0)
        const MULTISHOT_RECVMSG   = 1 << 3;
        /// Accept poll-first flag (>= 6.10)
        const ACCEPT_POLL_FIRST   = 1 << 4;
    }
}

#[cfg(io_uring)]
static FORCED_FEATURES: AtomicU32 = AtomicU32::new(0);

/// Force enable specific `io_uring` features.
///
/// This is additive: the specified features will be merged with the detected
/// kernel defaults (`effective = kernel_defaults | forced`).
/// Useful for enterprise kernels (e.g. RHEL/CentOS Stream 9 on kernel 5.14)
/// where features like multishot recv/recvmsg are backported.
///
/// On platforms without `io_uring`, this is a no-op.
pub fn force_io_uring_features(features: IoUringFeatures) {
    #[cfg(io_uring)]
    FORCED_FEATURES.fetch_or(features.bits(), Ordering::Relaxed);
    #[cfg(not(io_uring))]
    let _ = features;
}

/// Baseline kernel features inferred from `uname` release version.
#[cfg(io_uring)]
pub(crate) fn kernel_defaults() -> IoUringFeatures {
    static DEFAULTS: OnceLock<IoUringFeatures> = OnceLock::new();
    *DEFAULTS.get_or_init(|| {
        let mut feats = IoUringFeatures::empty();
        if is_kernel_at_least((5, 19)) {
            feats |= IoUringFeatures::RECVSEND_POLL_FIRST | IoUringFeatures::MULTISHOT_ACCEPT;
        }
        if is_kernel_at_least((6, 0)) {
            feats |= IoUringFeatures::MULTISHOT_RECV | IoUringFeatures::MULTISHOT_RECVMSG;
        }
        if is_kernel_at_least((6, 10)) {
            feats |= IoUringFeatures::ACCEPT_POLL_FIRST;
        }
        feats
    })
}

/// Central feature capability check.
///
/// On platforms without `io_uring`, this always returns `false`.
#[inline(always)]
pub fn is_feature_supported(feature: IoUringFeatures) -> bool {
    #[cfg(io_uring)]
    {
        let forced = IoUringFeatures::from_bits_truncate(FORCED_FEATURES.load(Ordering::Relaxed));
        (kernel_defaults() | forced).contains(feature)
    }
    #[cfg(not(io_uring))]
    {
        let _ = feature;
        false
    }
}
