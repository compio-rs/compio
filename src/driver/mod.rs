//! The platform-specified driver.
//! Some types differ by compilation target.

use std::{io, mem::MaybeUninit, time::Duration};

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
/// use std::net::SocketAddr;
///
/// use compio::{
///     buf::IntoInner,
///     driver::{AsRawFd, Driver, Poller},
///     net::UdpSocket,
///     op,
/// };
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
/// let entry = driver.poll_one(None).unwrap();
/// assert_eq!(entry.user_data(), 1);
/// entry.into_result().unwrap();
///
/// // read data
/// let buf = Vec::with_capacity(32);
/// let mut op = op::Recv::new(other_socket.as_raw_fd(), buf);
/// unsafe { driver.push(&mut op, 2) }.unwrap();
/// let entry = driver.poll_one(None).unwrap();
/// assert_eq!(entry.user_data(), 2);
/// let n_bytes = entry.into_result().unwrap();
/// let mut buf = op.into_inner().into_inner();
/// unsafe { buf.set_len(n_bytes) };
///
/// assert_eq!(buf, b"hello world");
/// ```
pub trait Poller {
    /// Attach an fd to the driver.
    ///
    /// ## Platform specific
    /// * IOCP: it will be attached to the IOCP completion port.
    /// * io-uring: it will do nothing and return `Ok(())`
    fn attach(&self, fd: RawFd) -> io::Result<()>;

    /// Push an operation with user-defined data.
    /// The data could be retrived from [`Entry`] when polling.
    ///
    /// # Safety
    ///
    /// - `op` should be alive until [`Poller::poll`] returns its result.
    unsafe fn push(&self, op: &mut (impl OpCode + 'static), user_data: usize) -> io::Result<()>;

    /// Cancel an operation with the pushed user-defined data.
    fn cancel(&self, user_data: usize);

    /// Poll the driver with an optional timeout.
    ///
    /// If there are already tasks completed, this method will return
    /// immediately.
    ///
    /// If there are no tasks completed, this call will block and wait.
    /// If no timeout specified, it will block forever.
    /// To interrupt the blocking, see [`Event`].
    ///
    /// [`Event`]: crate::event::Event
    fn poll(
        &self,
        timeout: Option<Duration>,
        entries: &mut [MaybeUninit<Entry>],
    ) -> io::Result<usize>;

    /// Poll the driver and get only one entry back.
    ///
    /// See [`Poller::poll`].
    fn poll_one(&self, timeout: Option<Duration>) -> io::Result<Entry> {
        let mut entry = MaybeUninit::uninit();
        let polled = self.poll(timeout, std::slice::from_mut(&mut entry))?;
        debug_assert_eq!(polled, 1);
        let entry = unsafe { entry.assume_init() };
        Ok(entry)
    }
}

/// An completed entry returned from kernel.
#[derive(Debug)]
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
