use compio_buf::bytes::Bytes;
use futures_util::Stream;

use crate::{
    framed::{Framed, SymmetricFramed, codec::bytes::BytesCodec, frame::CapacityDelimited},
    read::AsyncBufRead,
};

/// A type alias for a framed connection using the bytes codec.
pub type BytesFramed<R> = SymmetricFramed<R, (), BytesCodec, CapacityDelimited, Bytes>;

/// Extension trait for creating bytes-framed connections.
pub trait BytesFramedExt<R> {
    /// Create a bytes-framed stream from a file.
    fn from_reader(reader: R) -> Self;

    /// Convert this readable stream into a bytes-framed stream.
    fn bytes(self) -> Self;
}

impl<R> BytesFramedExt<R> for BytesFramed<R>
where
    R: AsyncBufRead + 'static + Unpin,
{
    fn from_reader(reader: R) -> Self {
        let framer = CapacityDelimited::new();
        Framed::symmetric(BytesCodec::new(), framer).with_reader(reader)
    }

    fn bytes(self) -> Self
    where
        Self: Stream<Item = Result<Bytes, std::io::Error>>,
    {
        self
    }
}
