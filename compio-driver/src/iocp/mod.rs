use std::{
    collections::HashMap,
    io,
    os::windows::{
        io::{OwnedHandle, OwnedSocket},
        prelude::{AsRawHandle, AsRawSocket},
    },
    pin::Pin,
    sync::Arc,
    task::Poll,
    time::Duration,
};

use compio_log::{instrument, trace};
use windows_sys::Win32::{Foundation::ERROR_CANCELLED, System::IO::OVERLAPPED};

use crate::{AsyncifyPool, Entry, Key, ProactorBuilder};

pub(crate) mod op;

mod cp;
mod wait;

pub(crate) use windows_sys::Win32::Networking::WinSock::{
    SOCKADDR_STORAGE as sockaddr_storage, socklen_t,
};

/// On windows, handle and socket are in the same size.
/// Both of them could be attached to an IOCP.
/// Therefore, both could be seen as fd.
pub type RawFd = isize;

/// Extracts raw fds.
pub trait AsRawFd {
    /// Extracts the raw fd.
    fn as_raw_fd(&self) -> RawFd;
}

/// Owned handle or socket on Windows.
#[derive(Debug)]
pub enum OwnedFd {
    /// Win32 handle.
    File(OwnedHandle),
    /// Windows socket handle.
    Socket(OwnedSocket),
}

impl AsRawFd for OwnedFd {
    fn as_raw_fd(&self) -> RawFd {
        match self {
            Self::File(fd) => fd.as_raw_handle() as _,
            Self::Socket(s) => s.as_raw_socket() as _,
        }
    }
}

impl AsRawFd for RawFd {
    fn as_raw_fd(&self) -> RawFd {
        *self
    }
}

impl AsRawFd for std::fs::File {
    fn as_raw_fd(&self) -> RawFd {
        self.as_raw_handle() as _
    }
}

impl AsRawFd for OwnedHandle {
    fn as_raw_fd(&self) -> RawFd {
        self.as_raw_handle() as _
    }
}

impl AsRawFd for socket2::Socket {
    fn as_raw_fd(&self) -> RawFd {
        self.as_raw_socket() as _
    }
}

impl AsRawFd for OwnedSocket {
    fn as_raw_fd(&self) -> RawFd {
        self.as_raw_socket() as _
    }
}

impl AsRawFd for std::process::ChildStdin {
    fn as_raw_fd(&self) -> RawFd {
        self.as_raw_handle() as _
    }
}

impl AsRawFd for std::process::ChildStdout {
    fn as_raw_fd(&self) -> RawFd {
        self.as_raw_handle() as _
    }
}

impl AsRawFd for std::process::ChildStderr {
    fn as_raw_fd(&self) -> RawFd {
        self.as_raw_handle() as _
    }
}

impl From<OwnedHandle> for OwnedFd {
    fn from(value: OwnedHandle) -> Self {
        Self::File(value)
    }
}

impl From<std::fs::File> for OwnedFd {
    fn from(value: std::fs::File) -> Self {
        Self::File(OwnedHandle::from(value))
    }
}

impl From<std::process::ChildStdin> for OwnedFd {
    fn from(value: std::process::ChildStdin) -> Self {
        Self::File(OwnedHandle::from(value))
    }
}

impl From<std::process::ChildStdout> for OwnedFd {
    fn from(value: std::process::ChildStdout) -> Self {
        Self::File(OwnedHandle::from(value))
    }
}

impl From<std::process::ChildStderr> for OwnedFd {
    fn from(value: std::process::ChildStderr) -> Self {
        Self::File(OwnedHandle::from(value))
    }
}

impl From<OwnedSocket> for OwnedFd {
    fn from(value: OwnedSocket) -> Self {
        Self::Socket(value)
    }
}

impl From<socket2::Socket> for OwnedFd {
    fn from(value: socket2::Socket) -> Self {
        Self::Socket(OwnedSocket::from(value))
    }
}

/// Operation type.
pub enum OpType {
    /// An overlapped operation.
    Overlapped,
    /// A blocking operation, needs a thread to spawn. The `operate` method
    /// should be thread safe.
    Blocking,
    /// A Win32 event object to be waited. The user should ensure that the
    /// handle is valid till operation completes. The `operate` method should be
    /// thread safe.
    Event(RawFd),
}

/// Abstraction of IOCP operations.
pub trait OpCode {
    /// Determines that the operation is really overlapped defined by Windows
    /// API. If not, the driver will try to operate it in another thread.
    fn op_type(&self) -> OpType {
        OpType::Overlapped
    }

    /// Perform Windows API call with given pointer to overlapped struct.
    ///
    /// It is always safe to cast `optr` to a pointer to
    /// [`Overlapped<Self>`].
    ///
    /// Don't do heavy work here if [`OpCode::op_type`] returns
    /// [`OpType::Event`].
    ///
    /// # Safety
    ///
    /// * `self` must be alive until the operation completes.
    /// * When [`OpCode::op_type`] returns [`OpType::Blocking`], this method is
    ///   called in another thread.
    unsafe fn operate(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> Poll<io::Result<usize>>;

    /// Cancel the async IO operation.
    ///
    /// Usually it calls `CancelIoEx`.
    ///
    /// # Safety
    ///
    /// * Should not use [`Overlapped::op`].
    unsafe fn cancel(self: Pin<&mut Self>, optr: *mut OVERLAPPED) -> io::Result<()> {
        let _optr = optr; // ignore it
        Ok(())
    }
}

/// Low-level driver of IOCP.
pub(crate) struct Driver {
    port: cp::Port,
    waits: HashMap<usize, wait::Wait>,
    pool: AsyncifyPool,
    notify_overlapped: Arc<Overlapped>,
}

impl Driver {
    pub fn new(builder: &ProactorBuilder) -> io::Result<Self> {
        instrument!(compio_log::Level::TRACE, "new", ?builder);

        let port = cp::Port::new()?;
        let driver = port.as_raw_handle() as _;
        Ok(Self {
            port,
            waits: HashMap::default(),
            pool: builder.create_or_get_thread_pool(),
            notify_overlapped: Arc::new(Overlapped::new(driver)),
        })
    }

    pub fn create_op<T: OpCode + 'static>(&self, op: T) -> Key<T> {
        Key::new(self.port.as_raw_handle() as _, op)
    }

    pub fn attach(&mut self, fd: RawFd) -> io::Result<()> {
        self.port.attach(fd)
    }

    pub fn cancel(&mut self, op: &mut Key<dyn OpCode>) {
        instrument!(compio_log::Level::TRACE, "cancel", ?op);
        trace!("cancel RawOp");
        let overlapped_ptr = op.as_mut_ptr();
        if let Some(w) = self.waits.get_mut(&op.user_data()) {
            if w.cancel().is_ok() {
                // The pack has been cancelled successfully, which means no packet will be post
                // to IOCP. Need not set the result because `create_entry` handles it.
                self.port.post_raw(overlapped_ptr).ok();
            }
        }
        let op = op.as_op_pin();
        // It's OK to fail to cancel.
        trace!("call OpCode::cancel");
        unsafe { op.cancel(overlapped_ptr.cast()) }.ok();
    }

    pub fn push(&mut self, op: &mut Key<dyn OpCode>) -> Poll<io::Result<usize>> {
        instrument!(compio_log::Level::TRACE, "push", ?op);
        let user_data = op.user_data();
        trace!("push RawOp");
        let optr = op.as_mut_ptr();
        let op_pin = op.as_op_pin();
        match op_pin.op_type() {
            OpType::Overlapped => unsafe { op_pin.operate(optr.cast()) },
            OpType::Blocking => loop {
                if self.push_blocking(user_data)? {
                    break Poll::Pending;
                } else {
                    // It's OK to wait forever, because any blocking task will notify the IOCP after
                    // it completes.
                    unsafe {
                        self.poll(None)?;
                    }
                }
            },
            OpType::Event(e) => {
                self.waits
                    .insert(user_data, wait::Wait::new(&self.port, e, op)?);
                Poll::Pending
            }
        }
    }

    fn push_blocking(&mut self, user_data: usize) -> io::Result<bool> {
        let port = self.port.handle();
        Ok(self
            .pool
            .dispatch(move || {
                let mut op = unsafe { Key::<dyn OpCode>::new_unchecked(user_data) };
                let optr = op.as_mut_ptr();
                let res = op.operate_blocking();
                port.post(res, optr).ok();
            })
            .is_ok())
    }

    fn create_entry(
        notify_user_data: usize,
        waits: &mut HashMap<usize, wait::Wait>,
        entry: Entry,
    ) -> Option<Entry> {
        let user_data = entry.user_data();
        if user_data != notify_user_data {
            if let Some(w) = waits.remove(&user_data) {
                if w.is_cancelled() {
                    Some(Entry::new(
                        user_data,
                        Err(io::Error::from_raw_os_error(ERROR_CANCELLED as _)),
                    ))
                } else if entry.result.is_err() {
                    Some(entry)
                } else {
                    let mut op = unsafe { Key::<dyn OpCode>::new_unchecked(user_data) };
                    let result = op.operate_blocking();
                    Some(Entry::new(user_data, result))
                }
            } else {
                Some(entry)
            }
        } else {
            None
        }
    }

    pub unsafe fn poll(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        instrument!(compio_log::Level::TRACE, "poll", ?timeout);

        let notify_user_data = self.notify_overlapped.as_ref() as *const Overlapped as usize;

        for e in self.port.poll(timeout)? {
            if let Some(e) = Self::create_entry(notify_user_data, &mut self.waits, e) {
                e.notify();
            }
        }

        Ok(())
    }

    pub fn handle(&self) -> io::Result<NotifyHandle> {
        Ok(NotifyHandle::new(
            self.port.handle(),
            self.notify_overlapped.clone(),
        ))
    }
}

impl AsRawFd for Driver {
    fn as_raw_fd(&self) -> RawFd {
        self.port.as_raw_handle() as _
    }
}

/// A notify handle to the inner driver.
pub struct NotifyHandle {
    port: cp::PortHandle,
    overlapped: Arc<Overlapped>,
}

impl NotifyHandle {
    fn new(port: cp::PortHandle, overlapped: Arc<Overlapped>) -> Self {
        Self { port, overlapped }
    }

    /// Notify the inner driver.
    pub fn notify(&self) -> io::Result<()> {
        self.port.post_raw(self.overlapped.as_ref())
    }
}

/// The overlapped struct we actually used for IOCP.
#[repr(C)]
pub struct Overlapped {
    /// The base [`OVERLAPPED`].
    pub base: OVERLAPPED,
    /// The unique ID of created driver.
    pub driver: RawFd,
}

impl Overlapped {
    pub(crate) fn new(driver: RawFd) -> Self {
        Self {
            base: unsafe { std::mem::zeroed() },
            driver,
        }
    }
}

// SAFETY: neither field of `OVERLAPPED` is used
unsafe impl Send for Overlapped {}
unsafe impl Sync for Overlapped {}
