#[derive(Debug, thiserror::Error)]
pub enum QuicError {
    #[error("quiche error: {0}")]
    Quiche(#[from] quiche::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Quic result type
pub type QuicResult<T, E = QuicError> = Result<T, E>;
