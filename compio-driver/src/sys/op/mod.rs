//! The async operations.
//!
//! Types in this mod represents the low-level operations passed to kernel.
//! The operation itself doesn't perform anything.
//! You need to pass them to [`crate::Proactor`], and poll the driver.
use mod_use::mod_use;

mod_use![
    asyncify, general, ext, flag, socket, fs, managed, multishot, zerocopy
];

cfg_if::cfg_if! {
    if #[cfg(unix)] {
        mod_use![unix];
        pub use crate::sys::pal::{CurrentDir, Interest, Stat};
    }
}
