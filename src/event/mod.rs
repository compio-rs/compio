//! Asynchronous events.
//!
//! Only for waking up the driver.

cfg_if::cfg_if! {
    if #[cfg(target_os = "windows")] {
        mod iocp;
        pub use iocp::*;
    } else if #[cfg(any(
        target_os = "android",
        target_os = "freebsd",
        target_os = "illumos",
        target_os = "linux",
    ))] {
        mod eventfd;
        pub use eventfd::*;
    } else if #[cfg(unix)] {
        mod pipe;
        pub use pipe::*;
    }
}
