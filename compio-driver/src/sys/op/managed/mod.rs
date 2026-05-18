use crate::sys::prelude::*;

cfg_select! {
    fusion => {
        mod iour;
        mod poll;
        mod fallback;
        mod_use![fusion];
    }
    io_uring => {
        mod_use![iour];
    }
    polling => {
        mod poll;
        mod_use![fallback];
    }
    stub => {
        mod stub;
        mod_use![fallback];
    }
    windows => {
        mod iocp;
        mod_use![fallback];
    }
    _ => {}
}
