#[allow(unused_imports)]
use crate::sys::prelude::mod_use;

cfg_select! {
    fusion => {
        mod iour;
        mod poll;

        crate::macros::fuse_op!(<S: AsFd> AcceptMulti(fd: S));
    }
    io_uring => {
        mod_use![iour];
    }
    polling => {
        mod_use![poll];
    }
    stub => {
        mod_use![stub];
    }
    _ => {}
}
