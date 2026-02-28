//! Socket I/O operations.
//!
//! This module defines traits for asynchronous socket I/O operations, such as
//! receiving messages with ancillary data. These traits can be implemented for
//! various socket types to provide flexible and efficient I/O capabilities.

use compio_buf::{BufResult, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};

/// # AsyncRecvMsg
///
/// Trait for asynchronous message reception on sockets, allowing for receiving
/// data along with ancillary data and source address information.
pub trait AsyncRecvMsg<AddrType> {
    /// Receive data and source address with ancillary data into owned buffer.
    async fn recv_msg<T: IoBufMut, C: IoBufMut>(
        &mut self,
        buffer: T,
        control: C,
        flags: i32,
    ) -> BufResult<(usize, usize, AddrType), (T, C)>;

    /// Receive data and source address with ancillary data into vectored
    /// buffer.
    async fn recv_msg_vectored<T: IoVectoredBufMut, C: IoBufMut>(
        &mut self,
        buffer: T,
        control: C,
        flags: i32,
    ) -> BufResult<(usize, usize, AddrType), (T, C)>;
}

/// # AsyncSendMsg
///
/// Trait for asynchronous message sending on sockets, allowing for sending data
/// along with ancillary data and destination address information.
pub trait AsyncSendMsg<AddrType> {
    /// Send data and destination address with ancillary data from owned buffer.
    async fn send_msg<T: IoBuf, C: IoBuf>(
        &mut self,
        buffer: T,
        control: C,
        addr: &AddrType,
        flags: i32,
    ) -> BufResult<usize, (T, C)>;

    /// Send data and destination address with ancillary data from vectored
    /// buffer.
    async fn send_msg_vectored<T: IoVectoredBuf, C: IoBuf>(
        &mut self,
        buffer: T,
        control: C,
        addr: &AddrType,
        flags: i32,
    ) -> BufResult<usize, (T, C)>;
}
