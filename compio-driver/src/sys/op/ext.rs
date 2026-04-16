//! Extension traits

use crate::sys::prelude::*;

/// Take buffer out of an operation.
pub trait TakeBuffer {
    /// Type of the buffer.
    type Buffer;

    /// Take buffer.
    fn take_buffer(self) -> Option<Self::Buffer>;
}

impl<I> TakeBuffer for I
where
    I: IntoInner<Inner = BufferRef>,
{
    type Buffer = I::Inner;

    fn take_buffer(self) -> Option<Self::Buffer> {
        Some(self.into_inner())
    }
}

/// Helper trait for taking buffer from a [`BufResult`].
pub trait ResultTakeBuffer {
    /// Type of the buffer.
    type Buffer;

    /// Call [`SetLen::advance_to`] if the result is [`Ok`] and return the
    /// buffer as result.
    ///
    /// # Safety
    ///
    /// The result value must be a valid length to advance to.
    unsafe fn take_buffer(self) -> io::Result<Option<Self::Buffer>>;
}

impl ResultTakeBuffer for BufResult<usize, BufferRef> {
    type Buffer = BufferRef;

    unsafe fn take_buffer(self) -> io::Result<Option<BufferRef>> {
        let (len, mut buf) = buf_try!(@try self);
        if len == 0 {
            return Ok(None);
        }
        unsafe { buf.advance_to(len) };

        Ok(Some(buf))
    }
}

impl<I: TakeBuffer<Buffer: IoBuf + SetLen>> ResultTakeBuffer for BufResult<usize, I> {
    type Buffer = I::Buffer;

    unsafe fn take_buffer(self) -> io::Result<Option<I::Buffer>> {
        let (len, buf) = buf_try!(@try self);
        // Kernel returns 0 for the operation, return Ok(None)
        if len == 0 {
            return Ok(None);
        }
        let Some(mut buf) = buf.take_buffer() else {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("Read {len} bytes, but no buffer was selected by kernel"),
            ));
        };
        unsafe { buf.advance_to(len) };
        Ok(Some(buf))
    }
}

/// Trait to update the buffer length inside the [`BufResult`].
pub trait BufResultExt {
    /// Call [`SetLen::advance_to`] if the result is [`Ok`].
    ///
    /// # Safety
    ///
    /// The result value must be a valid length to advance to.
    unsafe fn map_advanced(self) -> Self;
}

/// Trait to update the buffer length inside the [`BufResult`].
pub trait VecBufResultExt {
    /// Call [`SetLen::advance_vec_to`] if the result is [`Ok`].
    ///
    /// # Safety
    ///
    /// The result value must be a valid length to advance to.
    unsafe fn map_vec_advanced(self) -> Self;
}

impl<T: SetLen + IoBuf> BufResultExt for BufResult<usize, T> {
    unsafe fn map_advanced(self) -> Self {
        unsafe {
            self.map_res(|res| (res, ()))
                .map_advanced()
                .map_res(|(res, _)| res)
        }
    }
}

impl<T: SetLen + IoVectoredBuf> VecBufResultExt for BufResult<usize, T> {
    unsafe fn map_vec_advanced(self) -> Self {
        unsafe {
            self.map_res(|res| (res, ()))
                .map_vec_advanced()
                .map_res(|(res, _)| res)
        }
    }
}

impl<T: SetLen + IoBuf, O> BufResultExt for BufResult<(usize, O), T> {
    unsafe fn map_advanced(self) -> Self {
        self.map(|(init, obj), mut buffer| {
            unsafe {
                buffer.advance_to(init);
            }
            ((init, obj), buffer)
        })
    }
}

impl<T: SetLen + IoVectoredBuf, O> VecBufResultExt for BufResult<(usize, O), T> {
    unsafe fn map_vec_advanced(self) -> Self {
        self.map(|(init, obj), mut buffer| {
            unsafe {
                buffer.advance_vec_to(init);
            }
            ((init, obj), buffer)
        })
    }
}

impl<T: SetLen + IoBuf, C: SetLen + IoBuf, O> BufResultExt
    for BufResult<(usize, usize, O), (T, C)>
{
    unsafe fn map_advanced(self) -> Self {
        self.map(
            |(init_buffer, init_control, obj), (mut buffer, mut control)| {
                unsafe {
                    buffer.advance_to(init_buffer);
                    control.advance_to(init_control);
                }
                ((init_buffer, init_control, obj), (buffer, control))
            },
        )
    }
}

impl<T: SetLen + IoVectoredBuf, C: SetLen + IoBuf, O> VecBufResultExt
    for BufResult<(usize, usize, O), (T, C)>
{
    unsafe fn map_vec_advanced(self) -> Self {
        self.map(
            |(init_buffer, init_control, obj), (mut buffer, mut control)| {
                unsafe {
                    buffer.advance_vec_to(init_buffer);
                    control.advance_to(init_control);
                }
                ((init_buffer, init_control, obj), (buffer, control))
            },
        )
    }
}

/// Helper trait for [`RecvFrom`], [`RecvFromVectored`] and [`RecvMsg`].
///
/// [`RecvFrom`]: crate::op::RecvFrom
/// [`RecvMsg`]: crate::op::RecvMsg
/// [`RecvFromVectored`]: crate::op::RecvFromVectored
pub trait RecvResultExt {
    /// The mapped result.
    type RecvResult;

    /// Create [`SockAddr`] if the result is [`Ok`].
    fn map_addr(self) -> Self::RecvResult;
}

impl<T> RecvResultExt for BufResult<usize, (T, Option<SockAddr>)> {
    type RecvResult = BufResult<(usize, Option<SockAddr>), T>;

    fn map_addr(self) -> Self::RecvResult {
        self.map_buffer(|(buffer, addr)| (buffer, addr, 0))
            .map_addr()
            .map_res(|(res, _, addr)| (res, addr))
    }
}

impl<T> RecvResultExt for BufResult<usize, (T, Option<SockAddr>, usize)> {
    type RecvResult = BufResult<(usize, usize, Option<SockAddr>), T>;

    fn map_addr(self) -> Self::RecvResult {
        self.map2(
            |res, (buffer, addr, len)| ((res, len, addr), buffer),
            |(buffer, ..)| buffer,
        )
    }
}
