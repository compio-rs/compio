//! The platform-specified driver.
//! Some types differ by compilation target.

use std::{collections::VecDeque, io, mem::ManuallyDrop, pin::Pin, ptr::NonNull, time::Duration};

use slab::Slab;

use crate::BufResult;
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

trait Poller {
    fn attach(&mut self, fd: RawFd) -> io::Result<()>;

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

/// Low-level actions of completion-based IO.
/// It owns the operations to keep the driver safe.
///
/// # Examples
///
/// ```
/// use std::{mem::MaybeUninit, net::SocketAddr};
///
/// use arrayvec::ArrayVec;
/// use compio::{
///     buf::IntoInner,
///     driver::{AsRawFd, Entry, PollDriver},
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
/// let mut driver = PollDriver::new().unwrap();
/// driver.attach(socket.as_raw_fd()).unwrap();
/// driver.attach(other_socket.as_raw_fd()).unwrap();
///
/// // write data
/// let op_write = op::Send::new(socket.as_raw_fd(), "hello world");
/// let key_write = driver.push(op_write);
///
/// // read data
/// let buf = Vec::with_capacity(32);
/// let op_read = op::Recv::new(other_socket.as_raw_fd(), buf);
/// let key_read = driver.push(op_read);
///
/// let mut entries = ArrayVec::<Entry, 2>::new();
///
/// while entries.len() < 2 {
///     driver.poll(None, &mut entries).unwrap();
/// }
///
/// let mut n_bytes = 0;
/// let mut buf = MaybeUninit::uninit();
/// for (res, op) in driver.pop(&mut entries.into_iter()) {
///     let key = op.user_data();
///     if key == key_write {
///         res.unwrap();
///     } else if key == key_read {
///         n_bytes = res.unwrap();
///         buf.write(
///             unsafe { op.into_op::<op::Recv<Vec<u8>>>() }
///                 .into_inner()
///                 .into_inner(),
///         );
///     }
/// }
///
/// let mut buf = unsafe { buf.assume_init() };
/// unsafe { buf.set_len(n_bytes) };
/// assert_eq!(buf, b"hello world");
/// ```
pub struct PollDriver {
    driver: Driver,
    ops: Slab<RawOp>,
    squeue: VecDeque<usize>,
}

impl PollDriver {
    /// Create [`PollDriver`] with 1024 entries.
    pub fn new() -> io::Result<Self> {
        Self::with_entries(1024)
    }

    /// Create [`PollDriver`] with specified entries.
    pub fn with_entries(entries: u32) -> io::Result<Self> {
        Ok(Self {
            driver: Driver::new(entries)?,
            ops: Slab::with_capacity(entries as _),
            squeue: VecDeque::with_capacity(entries as _),
        })
    }

    /// Attach an fd to the driver.
    ///
    /// ## Platform specific
    /// * IOCP: it will be attached to the completion port. An fd could only be
    ///   attached to one driver, and could only be attached once, even if you
    ///   `try_clone` it. It will cause unexpected result to attach the handle
    ///   with one driver and push an op to another driver.
    /// * io-uring/mio: it will do nothing and return `Ok(())`
    pub fn attach(&mut self, fd: RawFd) -> io::Result<()> {
        self.driver.attach(fd)
    }

    /// Cancel an operation with the pushed user-defined data.
    ///
    /// The cancellation is not reliable. The underlying operation may continue,
    /// but just don't return from [`Poller::poll`]. Therefore, although an
    /// operation is cancelled, you should not reuse its `user_data`.
    ///
    /// It is well-defined to cancel before polling. If the submitted operation
    /// contains a cancelled user-defined data, the operation will be ignored.
    pub fn cancel(&mut self, user_data: usize) {
        self.driver.cancel(user_data);
    }

    /// Push an operation into the driver, and return the unique key, called
    /// user-defined data, associated with it.
    pub fn push(&mut self, op: impl OpCode + 'static) -> usize {
        let user_data = self.ops.insert(RawOp::new(op));
        self.squeue.push_back(user_data);
        user_data
    }

    /// Poll the driver and get completed entries.
    /// You need to call [`PollDriver::pop`] to get the pushed operations.
    pub fn poll(
        &mut self,
        timeout: Option<Duration>,
        entries: &mut impl Extend<Entry>,
    ) -> io::Result<()> {
        let mut iter = std::iter::from_fn(|| {
            self.squeue.pop_front().map(|user_data| {
                let op = self
                    .ops
                    .get_mut(user_data)
                    .expect("the squeue should be valid");
                let op = Operation::new(op.as_dyn_mut(), user_data);
                unsafe { std::mem::transmute::<_, Operation<'static>>(op) }
            })
        });
        unsafe {
            self.driver.poll(timeout, &mut iter, entries)?;
        }

        Ok(())
    }

    /// Get the pushed operations from the completion entries.
    pub fn pop<'a>(
        &'a mut self,
        entries: &'a mut impl Iterator<Item = Entry>,
    ) -> impl Iterator<Item = BufResult<usize, OwnedOperation>> + 'a {
        std::iter::from_fn(|| {
            entries.next().map(|entry| {
                let op = self
                    .ops
                    .try_remove(entry.user_data())
                    .expect("the entry should be valid");
                let op = OwnedOperation::new(op, entry.user_data());
                (entry.into_result(), op)
            })
        })
    }
}

impl AsRawFd for PollDriver {
    fn as_raw_fd(&self) -> RawFd {
        self.driver.as_raw_fd()
    }
}

/// Contains the operation and the user_data.
pub struct OwnedOperation {
    op: RawOp,
    user_data: usize,
}

impl OwnedOperation {
    pub(crate) fn new(op: RawOp, user_data: usize) -> Self {
        Self { op, user_data }
    }

    pub(crate) fn into_inner(self) -> RawOp {
        self.op
    }

    /// Restore the original operation.
    ///
    /// # Safety
    ///
    /// The caller should guarantee that the type is right.
    pub unsafe fn into_op<T: OpCode>(self) -> T {
        self.op.into_inner()
    }

    /// The same user_data when the operation is pushed into the driver.
    pub fn user_data(&self) -> usize {
        self.user_data
    }
}

struct Operation<'a> {
    op: &'a mut dyn OpCode,
    user_data: usize,
}

impl<'a> Operation<'a> {
    pub fn new(op: &'a mut dyn OpCode, user_data: usize) -> Self {
        Self { op, user_data }
    }

    /// # Safety
    ///
    /// The caller should guarantee that the opcode is pinned.
    pub unsafe fn opcode_pin(&mut self) -> Pin<&mut dyn OpCode> {
        Pin::new_unchecked(self.op)
    }

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
        Self::new(op, user_data)
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

    /// The user-defined data passed to [`Operation`].
    pub fn user_data(&self) -> usize {
        self.user_data
    }

    /// The result of the operation.
    pub fn into_result(self) -> io::Result<usize> {
        self.result
    }
}

pub(crate) struct RawOp(NonNull<dyn OpCode>);

impl RawOp {
    pub(crate) fn new(op: impl OpCode + 'static) -> Self {
        let op = Box::new(op);
        Self(unsafe { NonNull::new_unchecked(Box::into_raw(op as Box<dyn OpCode>)) })
    }

    pub(crate) fn as_dyn_mut(&mut self) -> &mut dyn OpCode {
        unsafe { self.0.as_mut() }
    }

    pub unsafe fn into_inner<T: OpCode>(self) -> T {
        let this = ManuallyDrop::new(self);
        *Box::from_raw(this.0.cast().as_ptr())
    }
}

impl Drop for RawOp {
    fn drop(&mut self) {
        drop(unsafe { Box::from_raw(self.0.as_ptr()) })
    }
}
