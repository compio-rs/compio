//! Completion-based terminal support for the [Compio](https://crates.io/crates/compio)
//! runtime.
//!
//! This crate keeps Crossterm's event data model and command types. It replaces
//! Crossterm's threaded event reader with a local Compio [`event::EventStream`]
//! and writes queued commands through Compio [`io::AsyncWrite`] sinks. Event
//! reading never starts a helper thread.
//!
//! Crossterm's cursor, style, terminal, and terminal detection modules are
//! re-exported unchanged. Crossterm's synchronous command execution traits and
//! macros are replaced by [`CommandQueue`], [`Commands`], and
//! [`Queueable`].
//!
//! [`io::AsyncWrite`]: compio_io::AsyncWrite

#![allow(unused_features)]
#![warn(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]
use std::io;

use crossterm::terminal::{disable_raw_mode, enable_raw_mode};

mod command;
pub mod event;

pub use command::{CommandQueue, Commands, Queueable};
pub use crossterm::{Command, cursor, style, terminal, tty};

/// A guard that keeps the terminal in raw mode until it is dropped.
#[derive(Debug)]
#[must_use = "raw mode is disabled when the guard is dropped"]
pub struct RawMode;

impl RawMode {
    /// Enables terminal raw mode.
    pub fn enable() -> io::Result<Self> {
        enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        if let Err(error) = disable_raw_mode() {
            compio_log::error!("failed to disable terminal raw mode: {error}");
        }
    }
}
