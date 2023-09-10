//! Asynchronous events.
//!
//! Only for waking up the driver.

cfg_if::cfg_if! {
    if #[cfg(target_os = "windows")] {
        mod iocp;
        pub use iocp::*;
    } else if #[cfg(target_os = "linux")] {
        mod iour;
        pub use iour::*;
    } else if #[cfg(unix)] {
        mod mio;
        pub use self::mio::*;
    }
}
