use crate::sys::prelude::*;

cfg_if! {
    if #[cfg(fusion)] {
        mod iour;
        mod poll;

        crate::macros::fuse_op!(<S: AsFd> AcceptMulti(fd: S));
    } else if #[cfg(io_uring)] {
        mod_use![iour];
    } else if #[cfg(polling)] {
        mod_use![poll];
    } else if #[cfg(stub)] {
        mod_use![stub];
    }
}
