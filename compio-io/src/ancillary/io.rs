#[cfg(feature = "allocator_api")]
use std::alloc::Allocator;

use compio_buf::{BufResult, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut, t_alloc};
use futures_util::Stream;

use crate::{AsyncReadManaged, IoResult};

/// Trait for asynchronous read with ancillary (control) data.
/// Intended for connected stream sockets (TCP, Unix streams) where no source
/// address is needed.
pub trait AsyncReadAncillary {
    /// Read data with ancillary data into an owned buffer.
    async fn read_with_ancillary<T: IoBufMut, C: IoBufMut>(
        &mut self,
        buffer: T,
        control: C,
    ) -> BufResult<(usize, usize), (T, C)>;

    /// Read data with ancillary data into a vectored buffer.
    async fn read_vectored_with_ancillary<T: IoVectoredBufMut, C: IoBufMut>(
        &mut self,
        buffer: T,
        control: C,
    ) -> BufResult<(usize, usize), (T, C)>;
}

impl<A: AsyncReadAncillary + ?Sized> AsyncReadAncillary for &mut A {
    #[inline]
    async fn read_with_ancillary<T: IoBufMut, C: IoBufMut>(
        &mut self,
        buffer: T,
        control: C,
    ) -> BufResult<(usize, usize), (T, C)> {
        (**self).read_with_ancillary(buffer, control).await
    }

    #[inline]
    async fn read_vectored_with_ancillary<T: IoVectoredBufMut, C: IoBufMut>(
        &mut self,
        buffer: T,
        control: C,
    ) -> BufResult<(usize, usize), (T, C)> {
        (**self).read_vectored_with_ancillary(buffer, control).await
    }
}

impl<A: AsyncReadAncillary + ?Sized, #[cfg(feature = "allocator_api")] Alloc: Allocator>
    AsyncReadAncillary for t_alloc!(Box, A, Alloc)
{
    #[inline]
    async fn read_with_ancillary<T: IoBufMut, C: IoBufMut>(
        &mut self,
        buffer: T,
        control: C,
    ) -> BufResult<(usize, usize), (T, C)> {
        (**self).read_with_ancillary(buffer, control).await
    }

    #[inline]
    async fn read_vectored_with_ancillary<T: IoVectoredBufMut, C: IoBufMut>(
        &mut self,
        buffer: T,
        control: C,
    ) -> BufResult<(usize, usize), (T, C)> {
        (**self).read_vectored_with_ancillary(buffer, control).await
    }
}

/// Trait for asynchronous write with ancillary (control) data.
/// Intended for connected stream sockets (TCP, Unix streams) where no
/// destination address is needed.
pub trait AsyncWriteAncillary {
    /// Write data with ancillary data from an owned buffer.
    async fn write_with_ancillary<T: IoBuf, C: IoBuf>(
        &mut self,
        buffer: T,
        control: C,
    ) -> BufResult<usize, (T, C)>;

    /// Write data with ancillary data from a vectored buffer.
    async fn write_vectored_with_ancillary<T: IoVectoredBuf, C: IoBuf>(
        &mut self,
        buffer: T,
        control: C,
    ) -> BufResult<usize, (T, C)>;
}

impl<A: AsyncWriteAncillary + ?Sized> AsyncWriteAncillary for &mut A {
    #[inline]
    async fn write_with_ancillary<T: IoBuf, C: IoBuf>(
        &mut self,
        buffer: T,
        control: C,
    ) -> BufResult<usize, (T, C)> {
        (**self).write_with_ancillary(buffer, control).await
    }

    #[inline]
    async fn write_vectored_with_ancillary<T: IoVectoredBuf, C: IoBuf>(
        &mut self,
        buffer: T,
        control: C,
    ) -> BufResult<usize, (T, C)> {
        (**self)
            .write_vectored_with_ancillary(buffer, control)
            .await
    }
}

impl<A: AsyncWriteAncillary + ?Sized, #[cfg(feature = "allocator_api")] Alloc: Allocator>
    AsyncWriteAncillary for t_alloc!(Box, A, Alloc)
{
    #[inline]
    async fn write_with_ancillary<T: IoBuf, C: IoBuf>(
        &mut self,
        buffer: T,
        control: C,
    ) -> BufResult<usize, (T, C)> {
        (**self).write_with_ancillary(buffer, control).await
    }

    #[inline]
    async fn write_vectored_with_ancillary<T: IoVectoredBuf, C: IoBuf>(
        &mut self,
        buffer: T,
        control: C,
    ) -> BufResult<usize, (T, C)> {
        (**self)
            .write_vectored_with_ancillary(buffer, control)
            .await
    }
}

/// Trait for asynchronous read with ancillary (control) data that returns
/// managed buffers. Intended for connected stream sockets (TCP, Unix streams)
/// where no source address is needed.
pub trait AsyncReadAncillaryManaged: AsyncReadManaged {
    /// Read data into a managed buffer with ancillary data.
    ///
    /// # Implementation Note
    ///
    /// - If `len` == 0, implementation should use buffer's size as `len`
    /// - if `len` > 0, `min(len, buffer_size)` will be the max number of bytes
    ///   to be read.
    async fn read_managed_with_ancillary<C: IoBufMut>(
        &mut self,
        len: usize,
        control: C,
    ) -> IoResult<Option<(Self::Buffer, C)>>;
}

/// Trait for asynchronous read with ancillary (control) data that returns
/// multiple managed buffers. Intended for connected stream sockets (TCP, Unix
/// streams) where no source address is needed.
pub trait AsyncReadAncillaryMulti {
    /// A wrapped type for the payload data and the ancillary data.
    type Return;

    /// Read data and ancillary data into multiple managed buffers.
    fn read_multi_with_ancillary(
        &mut self,
        control_len: usize,
    ) -> impl Stream<Item = IoResult<Self::Return>>;
}
