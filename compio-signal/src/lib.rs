//! Asynchronous signal handling.
//!
//! # Examples
//!
//! Print on "ctrl-c" notification.
//!
//! ```rust,no_run
//! use compio_signal::ctrl_c;
//!
//! compio_runtime::block_on(async {
//!     ctrl_c().await.unwrap();
//!     println!("ctrl-c received!");
//! })
//! ```

#![cfg_attr(feature = "lazy_cell", feature(lazy_cell))]
#![warn(missing_docs)]

#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(unix)]
pub mod unix;

/// Completes when a "ctrl-c" notification is sent to the process.
pub async fn ctrl_c() -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        windows::ctrl_c().await
    }
    #[cfg(unix)]
    {
        unix::signal(libc::SIGINT).await
    }
}
