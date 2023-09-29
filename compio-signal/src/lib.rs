//! Asynchronous signal handling.
//!
//! # Examples
//!
//! Print on "ctrl-c" notification.
//!
//! ```rust,no_run
//! use compio::signal;
//!
//! compio::task::block_on(async {
//!     signal::ctrl_c().await.unwrap();
//!     println!("ctrl-c received!");
//! })
//! ```

#![cfg_attr(feature = "lazy_cell", feature(lazy_cell))]

#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(target_os = "windows")]
#[doc(no_inline)]
pub use windows::ctrl_c;

#[cfg(unix)]
pub mod unix;

/// Completes when a "ctrl-c" notification is sent to the process.
#[cfg(unix)]
pub async fn ctrl_c() -> std::io::Result<()> {
    unix::signal(libc::SIGINT).await
}
