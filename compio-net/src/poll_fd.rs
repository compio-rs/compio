#[cfg(windows)]
use std::os::windows::io::{AsRawSocket, RawSocket};
use std::{io, ops::Deref};

use compio_buf::IntoInner;
use compio_driver::{AsRawFd, RawFd, SharedFd, ToSharedFd};
#[cfg(windows)]
use windows_sys::Win32::Networking::WinSock::{FD_ACCEPT, FD_CONNECT, FD_READ, FD_WRITE};
#[cfg(unix)]
use {
    compio_buf::BufResult,
    compio_driver::op::{Interest, PollOnce},
};

/// A wrapper for socket, providing functionalities to wait for readiness.
#[derive(Debug)]
pub struct PollFd<T: AsRawFd> {
    inner: SharedFd<T>,
    #[cfg(windows)]
    event: sys::WSAEvent,
}

impl<T: AsRawFd> PollFd<T> {
    /// Create [`PollFd`] without attaching the source. Ready-based sources need
    /// not to be attached.
    pub fn new(source: T) -> io::Result<Self> {
        Self::from_shared_fd(SharedFd::new(source))
    }

    pub(crate) fn from_shared_fd(inner: SharedFd<T>) -> io::Result<Self> {
        Ok(Self {
            inner,
            #[cfg(windows)]
            event: sys::WSAEvent::new()?,
        })
    }
}

#[cfg(unix)]
impl<T: AsRawFd + 'static> PollFd<T> {
    /// Wait for accept readiness, before calling `accept`, or after `accept`
    /// returns `WouldBlock`.
    pub async fn accept_ready(&self) -> io::Result<()> {
        self.read_ready().await
    }

    /// Wait for connect readiness.
    pub async fn connect_ready(&self) -> io::Result<()> {
        self.write_ready().await
    }

    /// Wait for read readiness.
    pub async fn read_ready(&self) -> io::Result<()> {
        let op = PollOnce::new(self.to_shared_fd(), Interest::Readable);
        let BufResult(res, _) = compio_runtime::submit(op).await;
        res?;
        Ok(())
    }

    /// Wait for write readiness.
    pub async fn write_ready(&self) -> io::Result<()> {
        let op = PollOnce::new(self.to_shared_fd(), Interest::Writable);
        let BufResult(res, _) = compio_runtime::submit(op).await;
        res?;
        Ok(())
    }
}

#[cfg(windows)]
impl<T: AsRawFd + 'static> PollFd<T> {
    /// Wait for accept readiness, before calling `accept`, or after `accept`
    /// returns `WouldBlock`.
    pub async fn accept_ready(&self) -> io::Result<()> {
        self.event.wait(self.to_shared_fd(), FD_ACCEPT).await
    }

    /// Wait for connect readiness.
    pub async fn connect_ready(&self) -> io::Result<()> {
        self.event.wait(self.to_shared_fd(), FD_CONNECT).await
    }

    /// Wait for read readiness.
    pub async fn read_ready(&self) -> io::Result<()> {
        self.event.wait(self.to_shared_fd(), FD_READ).await
    }

    /// Wait for write readiness.
    pub async fn write_ready(&self) -> io::Result<()> {
        self.event.wait(self.to_shared_fd(), FD_WRITE).await
    }
}

impl<T: AsRawFd> IntoInner for PollFd<T> {
    type Inner = SharedFd<T>;

    fn into_inner(self) -> Self::Inner {
        self.inner
    }
}

impl<T: AsRawFd> ToSharedFd<T> for PollFd<T> {
    fn to_shared_fd(&self) -> SharedFd<T> {
        self.inner.clone()
    }
}

impl<T: AsRawFd> AsRawFd for PollFd<T> {
    fn as_raw_fd(&self) -> RawFd {
        self.inner.as_raw_fd()
    }
}

#[cfg(windows)]
impl<T: AsRawFd + AsRawSocket> AsRawSocket for PollFd<T> {
    fn as_raw_socket(&self) -> RawSocket {
        self.inner.as_raw_socket()
    }
}

impl<T: AsRawFd> Deref for PollFd<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[cfg(windows)]
mod sys {
    use std::{
        io,
        os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
        pin::Pin,
        ptr::null,
        sync::atomic::{AtomicI32, AtomicUsize, Ordering},
        task::Poll,
    };

    use compio_buf::{BufResult, IntoInner};
    use compio_driver::{syscall, AsRawFd, OpCode, OpType, SharedFd};
    use windows_sys::Win32::{
        Networking::WinSock::{WSAEnumNetworkEvents, WSAEventSelect, WSANETWORKEVENTS},
        System::{Threading::CreateEventW, IO::OVERLAPPED},
    };

    const EVENT_COUNT: usize = 5;

    #[derive(Debug)]
    pub struct WSAEvent {
        ev_object: SharedFd<OwnedHandle>,
        ev_record: [AtomicUsize; EVENT_COUNT],
        events: AtomicI32,
    }

    impl WSAEvent {
        pub fn new() -> io::Result<Self> {
            Ok(Self {
                ev_object: SharedFd::new(unsafe {
                    OwnedHandle::from_raw_handle(syscall!(
                        HANDLE,
                        CreateEventW(null(), 1, 0, null())
                    )? as _)
                }),
                ev_record: Default::default(),
                events: AtomicI32::new(0),
            })
        }

        pub async fn wait<T: AsRawFd + 'static>(
            &self,
            socket: SharedFd<T>,
            event: u32,
        ) -> io::Result<()> {
            struct EventGuard<'a> {
                wsa_event: &'a WSAEvent,
                event: i32,
            }

            impl Drop for EventGuard<'_> {
                fn drop(&mut self) {
                    let index = (self.event.ilog2() - 1) as usize;
                    if self.wsa_event.ev_record[index].fetch_sub(1, Ordering::Relaxed) == 1 {
                        self.wsa_event
                            .events
                            .fetch_add(!self.event, Ordering::Relaxed);
                    }
                }
            }

            let event = event as i32;
            let ev_object = self.ev_object.clone();

            let index = (event.ilog2() - 1) as usize;
            let events = if self.ev_record[index].fetch_add(1, Ordering::Relaxed) == 0 {
                self.events.fetch_or(event, Ordering::Relaxed) | event
            } else {
                self.events.load(Ordering::Relaxed)
            };
            syscall!(
                SOCKET,
                WSAEventSelect(
                    socket.as_raw_fd() as _,
                    ev_object.as_raw_handle() as _,
                    events
                )
            )?;
            let _guard = EventGuard {
                wsa_event: self,
                event,
            };
            let op = WaitWSAEvent::new(socket, ev_object, index + 1);
            let BufResult(res, _) = compio_runtime::submit(op).await;
            res?;
            Ok(())
        }
    }

    struct WaitWSAEvent<T> {
        socket: SharedFd<T>,
        ev_object: SharedFd<OwnedHandle>,
        index: usize,
    }

    impl<T> WaitWSAEvent<T> {
        pub fn new(socket: SharedFd<T>, ev_object: SharedFd<OwnedHandle>, index: usize) -> Self {
            Self {
                socket,
                ev_object,
                index,
            }
        }
    }

    impl<T> IntoInner for WaitWSAEvent<T> {
        type Inner = SharedFd<OwnedHandle>;

        fn into_inner(self) -> Self::Inner {
            self.ev_object
        }
    }

    impl<T: AsRawFd> OpCode for WaitWSAEvent<T> {
        fn op_type(&self) -> OpType {
            OpType::Event(self.ev_object.as_raw_fd())
        }

        unsafe fn operate(self: Pin<&mut Self>, _optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
            let mut events: WSANETWORKEVENTS = unsafe { std::mem::zeroed() };
            events.lNetworkEvents = 10;
            syscall!(
                SOCKET,
                WSAEnumNetworkEvents(
                    self.socket.as_raw_fd() as _,
                    self.ev_object.as_raw_handle() as _,
                    &mut events
                )
            )?;
            let res = events.iErrorCode[self.index + 1];
            if res == 0 {
                Poll::Ready(Ok(0))
            } else {
                Poll::Ready(Err(io::Error::from_raw_os_error(res)))
            }
        }
    }
}
