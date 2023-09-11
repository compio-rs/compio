//! Compio custom IO errors
use std::{
    fmt::Display,
    io::{self, ErrorKind},
};

use strum::{AsRefStr, EnumMessage, EnumString};

/// Custom IO errors
///
/// Clients could parse them from IOError error prefix
#[derive(Debug, AsRefStr, EnumString, EnumMessage)]
pub enum Error {
    /// Returned during file registration update by file registry embedded into
    /// driver
    #[strum(message = "Requested to register files outside of registration range")]
    FilesOutOfRange,
    /// Returned by driver's `get_free_registered_fd` method
    #[strum(message = "No free registered file descriptors are available")]
    NoFreeRegisteredFiles,
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: {}",
            self.as_ref(),
            self.get_message().unwrap_or_default()
        )
    }
}

impl From<Error> for io::Error {
    fn from(other: Error) -> io::Error {
        io::Error::new(ErrorKind::Other, other.to_string())
    }
}
