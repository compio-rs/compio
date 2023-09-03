//! The platform-specified driver.
//! Some types differ by compilation target.

use std::{io, time::Duration};

cfg_if::cfg_if! {
    if #[cfg(target_os = "windows")] {
        mod iocp;
        pub use iocp::*;
    } else if #[cfg(target_os = "linux")] {
        mod iour;
        pub use iour::*;
    }
}

/// An abstract of [`Driver`].
/// It contains some low-level actions of completion-based IO.
///
/// You don't need them unless you are controlling a [`Driver`] yourself.
///
/// # Examples
///
/// ```
/// use compio::{
///     buf::IntoInner,
///     driver::{AsRawFd, Driver, Poller},
///     net::UdpSocket,
///     op,
/// };
/// use std::net::SocketAddr;
///
/// let first_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
/// let second_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
///
/// // bind sockets
/// let socket = UdpSocket::bind(first_addr).unwrap();
/// let first_addr = socket.local_addr().unwrap();
/// let other_socket = UdpSocket::bind(second_addr).unwrap();
/// let second_addr = other_socket.local_addr().unwrap();
///
/// // connect sockets
/// socket.connect(second_addr).unwrap();
/// other_socket.connect(first_addr).unwrap();
///
/// let driver = Driver::new().unwrap();
/// driver.attach(socket.as_raw_fd()).unwrap();
/// driver.attach(other_socket.as_raw_fd()).unwrap();
///
/// // write data
/// let mut op = op::Send::new(socket.as_raw_fd(), "hello world");
/// unsafe { driver.push(&mut op, 1) }.unwrap();
/// let entry = driver.poll(None).unwrap();
/// assert_eq!(entry.user_data(), 1);
/// entry.into_result().unwrap();
///
/// // read data
/// let buf = Vec::with_capacity(32);
/// let mut op = op::Recv::new(other_socket.as_raw_fd(), buf);
/// unsafe { driver.push(&mut op, 2) }.unwrap();
/// let entry = driver.poll(None).unwrap();
/// assert_eq!(entry.user_data(), 2);
/// let n_bytes = entry.into_result().unwrap();
/// let mut buf = op.into_inner().into_inner();
/// unsafe { buf.set_len(n_bytes) };
///
/// assert_eq!(buf, b"hello world");
/// ```
pub trait Poller {
    /// Attach an fd to the driver.
    fn attach(&self, fd: RawFd) -> io::Result<()>;

    /// Push an operation with user-defined data.
    /// The data could be retrived from [`Entry`] when polling.
    ///
    /// # Safety
    ///
    /// `op` should be alive until [`Poller::poll`] returns its result.
    unsafe fn push(&self, op: &mut (impl OpCode + 'static), user_data: usize) -> io::Result<()>;

    /// Post an operation result to the driver.
    fn post(&self, user_data: usize, result: usize) -> io::Result<()>;

    /// Poll the driver with an optional timeout.
    /// If no timeout specified, the call will block.
    fn poll(&self, timeout: Option<Duration>) -> io::Result<Entry>;
}

/// An completed entry returned from kernel.
pub struct Entry {
    user_data: usize,
    result: io::Result<usize>,
}

impl Entry {
    pub(crate) fn new(user_data: usize, result: io::Result<usize>) -> Self {
        Self { user_data, result }
    }

    /// The user-defined data passed to [`Poller::push`].
    pub fn user_data(&self) -> usize {
        self.user_data
    }

    /// The result of the operation.
    pub fn into_result(self) -> io::Result<usize> {
        self.result
    }
}
