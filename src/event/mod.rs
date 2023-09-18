//! Asynchronous events.
//!
//! Only for waking up the driver.

cfg_if::cfg_if! {
    if #[cfg(target_os = "windows")] {
        mod iocp;
        pub use iocp::*;
    } else if #[cfg(any(target_os = "linux", target_os = "android"))] {
        mod eventfd;
        pub use eventfd::*;
    } else if #[cfg(unix)] {
        mod pipe;
        pub use pipe::*;
    }
}
