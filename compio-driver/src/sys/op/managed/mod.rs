use crate::sys::prelude::*;

cfg_if::cfg_if! {
    if #[cfg(fusion)] {
        mod iour;
        mod poll;
        mod fallback;
        mod_use![fusion];
    } else if #[cfg(io_uring)] {
        mod_use![iour];
    } else if #[cfg(polling)] {
        mod poll;
        mod_use![fallback];
    } else if #[cfg(stub)] {
        mod stub;
        mod_use![fallback];
    } else if #[cfg(windows)] {
        mod iocp;
        mod_use![fallback];
    }
}
