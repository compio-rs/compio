macro_rules! try_io {
    ($exp:expr) => {
        match $exp {
            (Ok(res), io) => (res, io),
            (Err(e), io) => return (Err(e.into()), io),
        }
    };
}

pub(super) use try_io;

use crate::quic::Io;

#[derive(Debug, thiserror::Error)]
pub enum QuicError {
    #[error("quiche error: {0}")]
    Quiche(#[from] quiche::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Quic result type
pub type QuicResult<T, E = QuicError> = Result<T, E>;

pub(super) type IoResult<T, I = Io> = (QuicResult<T>, I);
