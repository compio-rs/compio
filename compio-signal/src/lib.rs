//! Signal utilities.
//!
//! # Examples
//!
//! Print on "ctrl-c" notification.
//!
//! ```rust,no_run
//! use compio_signal::ctrl_c;
//!
//! # compio_runtime::Runtime::new().unwrap().block_on(async {
//! ctrl_c().await.unwrap();
//! println!("ctrl-c received!");
//! # })
//! ```

#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(feature = "once_cell_try", feature(once_cell_try))]
#![cfg_attr(feature = "lazy_cell", feature(lazy_cell))]
#![warn(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]
#![allow(stable_features)]

#[cfg(windows)]
pub mod windows;

#[cfg(unix)]
#[cfg_attr(target_os = "linux", path = "linux.rs")]
pub mod unix;

/// Completes when a "ctrl-c" notification is sent to the process.
pub async fn ctrl_c() -> std::io::Result<()> {
    #[cfg(windows)]
    {
        windows::ctrl_c().await
    }
    #[cfg(unix)]
    {
        unix::signal(libc::SIGINT).await
    }
}
