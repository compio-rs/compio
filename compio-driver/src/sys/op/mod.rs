//! The async operations.
//!
//! Types in this mod represents the low-level operations passed to kernel.
//! The operation itself doesn't perform anything.
//! You need to pass them to [`crate::Proactor`], and poll the driver.

use crate::sys::prelude::*;

mod_use![
    asyncify, general, ext, flag, socket, fs, managed, multishot, zerocopy
];

cfg_if! {
    if #[cfg(unix)] {
        mod_use![unix];

        pub use crate::sys::pal::{CurrentDir, Interest};
        pub use rustix::fs::{Mode, OFlags, Stat};
    }
}

cfg_if! {
    if #[cfg(target_os = "linux")] {
        pub use rustix::pipe::SpliceFlags;
    }
}

pub use rustix::net::{RecvFlags, SendFlags};
