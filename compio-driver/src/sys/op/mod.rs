//! The async operations.
//!
//! Types in this mod represents the low-level operations passed to kernel.
//! The operation itself doesn't perform anything.
//! You need to pass them to [`crate::Proactor`], and poll the driver.

use crate::sys::prelude::*;

mod_use![
    asyncify, general, ext, flag, socket, fs, managed, multishot, zerocopy
];

cfg_select! {
    unix => {
        mod_use![unix];

        pub use crate::sys::pal::{CurrentDir, Interest};
        pub use rustix::fs::{Mode, OFlags, Stat};
    }
    _ => {}
}

cfg_select! {
    any(target_os = "linux", target_os = "android") => {
        pub use rustix::pipe::SpliceFlags;
    }
    _ => {}
}

pub use rustix::net::{RecvFlags, ReturnFlags, SendFlags};
