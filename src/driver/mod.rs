//! The platform-specified driver.
//! Some types differ by compilation target.

use std::{io, time::Duration};
#[cfg(unix)]
mod unix;

cfg_if::cfg_if! {
    if #[cfg(target_os = "windows")] {
        mod iocp;
        pub use iocp::*;
    } else if #[cfg(target_os = "linux")] {
        mod iour;
        pub use iour::*;
    } else if #[cfg(unix)]{
        mod mio;
        pub use self::mio::*;
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
/// use arrayvec::ArrayVec;
/// use compio::{
///     buf::IntoInner,
///     driver::{AsRawFd, Driver, Entry, Poller},
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
/// let mut driver = Driver::new().unwrap();
/// driver.attach(socket.as_raw_fd()).unwrap();
/// driver.attach(other_socket.as_raw_fd()).unwrap();
///
/// // write data
/// let mut op_write = op::Send::new(socket.as_raw_fd(), "hello world");
///
/// // read data
/// let buf = Vec::with_capacity(32);
/// let mut op_read = op::Recv::new(other_socket.as_raw_fd(), buf);
///
/// let ops = [(&mut op_write, 1).into(), (&mut op_read, 2).into()];
/// let mut entries = ArrayVec::<Entry, 2>::new();
/// unsafe {
///     driver
///         .poll(None, &mut ops.into_iter(), &mut entries)
///         .unwrap()
/// };
/// while entries.len() < 2 {
///     unsafe {
///         driver
///             .poll(None, &mut [].into_iter(), &mut entries)
///             .unwrap()
///     };
/// }
///
/// let mut n_bytes = 0;
/// for entry in entries {
///     match entry.user_data() {
///         1 => {
///             entry.into_result().unwrap();
///         }
///         2 => {
///             n_bytes = entry.into_result().unwrap();
///         }
///         _ => unreachable!(),
///     }
/// }
///
/// let mut buf = op_read.into_inner().into_inner();
/// unsafe { buf.set_len(n_bytes) };
/// assert_eq!(buf, b"hello world");
/// ```
pub trait Poller {
    /// Attach an fd to the driver.
    ///
    /// ## Platform specific
    /// * IOCP: it will be attached to the completion port. An fd could only be
    ///   attached to one driver, and could only be attached once, even if you
    ///   `try_clone` it. It will cause unexpected result to attach the handle
    ///   with one driver and push an op to another driver.
    /// * io-uring/mio: it will do nothing and return `Ok(())`
    fn attach(&mut self, fd: RawFd) -> io::Result<()>;

    /// Cancel an operation with the pushed user-defined data.
    ///
    /// The cancellation is not reliable. The underlying operation may continue,
    /// but just don't return from [`Poller::poll`]. Therefore, although an
    /// operation is cancelled, you should not reuse its `user_data`.
    fn cancel(&mut self, user_data: usize);

    /// Poll the driver with an optional timeout.
    ///
    /// The operations in `ops` may not be totally consumed. This method will
    /// try its best to consume them, but if an error occurs, it will return
    /// immediately.
    ///
    /// If there are no tasks completed, this call will block and wait.
    /// If no timeout specified, it will block forever.
    /// To interrupt the blocking, see [`Event`].
    ///
    /// [`Event`]: crate::event::Event
    ///
    /// # Safety
    ///
    /// * Operations should be alive until [`Poller::poll`] returns its result.
    /// * User defined data should be unique.
    unsafe fn poll<'a>(
        &mut self,
        timeout: Option<Duration>,
        ops: &mut impl Iterator<Item = Operation<'a>>,
        entries: &mut impl Extend<Entry>,
    ) -> io::Result<()>;
}

/// An operation with a unique user defined data.
pub struct Operation<'a> {
    op: &'a mut dyn OpCode,
    user_data: usize,
}

impl<'a> Operation<'a> {
    /// Create [`Operation`].
    pub fn new(op: &'a mut impl OpCode, user_data: usize) -> Self {
        Self { op, user_data }
    }

    /// Create [`Operation`] from dyn [`OpCode`].
    pub fn new_dyn(op: &'a mut dyn OpCode, user_data: usize) -> Self {
        Self { op, user_data }
    }

    /// Get the opcode inside.
    pub fn opcode_mut(&mut self) -> &mut dyn OpCode {
        self.op
    }

    /// Get the user defined data.
    pub fn user_data(&self) -> usize {
        self.user_data
    }
}

impl<'a, O: OpCode> From<(&'a mut O, usize)> for Operation<'a> {
    fn from((op, user_data): (&'a mut O, usize)) -> Self {
        Self::new(op, user_data)
    }
}

impl<'a> From<(&'a mut dyn OpCode, usize)> for Operation<'a> {
    fn from((op, user_data): (&'a mut dyn OpCode, usize)) -> Self {
        Self::new_dyn(op, user_data)
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
