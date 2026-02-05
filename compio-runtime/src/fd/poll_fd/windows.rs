use std::{
    io,
    ops::Deref,
    os::windows::io::{AsRawHandle, AsRawSocket, FromRawHandle, OwnedHandle, RawSocket},
    pin::Pin,
    ptr::null,
    sync::atomic::{AtomicI32, AtomicUsize, Ordering},
    task::Poll,
};

use compio_buf::{BufResult, IntoInner};
use compio_driver::{
    AsFd, AsRawFd, BorrowedFd, OpCode, OpType, RawFd, SharedFd, ToSharedFd, syscall,
};
use windows_sys::Win32::{
    Foundation::ERROR_IO_PENDING,
    Networking::WinSock::{
        FD_ACCEPT, FD_CONNECT, FD_MAX_EVENTS, FD_READ, FD_WRITE, WSAEnumNetworkEvents,
        WSAEventSelect, WSANETWORKEVENTS,
    },
    System::{IO::OVERLAPPED, Threading::CreateEventW},
};

#[derive(Debug)]
pub struct PollFd<T: AsFd> {
    inner: SharedFd<T>,
    event: WSAEvent,
}

impl<T: AsFd> PollFd<T> {
    pub fn new(inner: SharedFd<T>) -> io::Result<Self> {
        Ok(Self {
            inner,
            event: WSAEvent::new()?,
        })
    }
}

impl<T: AsFd + 'static> PollFd<T> {
    pub async fn accept_ready(&self) -> io::Result<()> {
        self.event.wait(self.to_shared_fd(), FD_ACCEPT).await
    }

    pub async fn connect_ready(&self) -> io::Result<()> {
        self.event.wait(self.to_shared_fd(), FD_CONNECT).await
    }

    pub async fn read_ready(&self) -> io::Result<()> {
        self.event.wait(self.to_shared_fd(), FD_READ).await
    }

    pub async fn write_ready(&self) -> io::Result<()> {
        self.event.wait(self.to_shared_fd(), FD_WRITE).await
    }
}

impl<T: AsFd> IntoInner for PollFd<T> {
    type Inner = SharedFd<T>;

    fn into_inner(self) -> Self::Inner {
        self.inner
    }
}

impl<T: AsFd> ToSharedFd<T> for PollFd<T> {
    fn to_shared_fd(&self) -> SharedFd<T> {
        self.inner.clone()
    }
}

impl<T: AsFd> AsFd for PollFd<T> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.inner.as_fd()
    }
}

impl<T: AsFd> AsRawFd for PollFd<T> {
    fn as_raw_fd(&self) -> RawFd {
        self.inner.as_fd().as_raw_fd()
    }
}

impl<T: AsFd + AsRawSocket> AsRawSocket for PollFd<T> {
    fn as_raw_socket(&self) -> RawSocket {
        self.inner.as_raw_socket()
    }
}

impl<T: AsFd> Deref for PollFd<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[derive(Debug)]
pub struct WSAEvent {
    ev_object: SharedFd<OwnedHandle>,
    ev_record: [AtomicUsize; FD_MAX_EVENTS as usize],
    events: AtomicI32,
}

impl WSAEvent {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            ev_object: SharedFd::new(unsafe {
                OwnedHandle::from_raw_handle(
                    syscall!(HANDLE, CreateEventW(null(), 1, 0, null()))? as _
                )
            }),
            ev_record: Default::default(),
            events: AtomicI32::new(0),
        })
    }

    pub async fn wait<T: AsFd + 'static>(
        &self,
        mut socket: SharedFd<T>,
        event: u32,
    ) -> io::Result<()> {
        struct EventGuard<'a> {
            wsa_event: &'a WSAEvent,
            event: i32,
        }

        impl Drop for EventGuard<'_> {
            fn drop(&mut self) {
                let index = self.event.ilog2() as usize;
                if self.wsa_event.ev_record[index].fetch_sub(1, Ordering::Relaxed) == 1 {
                    self.wsa_event
                        .events
                        .fetch_add(!self.event, Ordering::Relaxed);
                }
            }
        }

        let event = event as i32;
        let mut ev_object = self.ev_object.clone();

        let index = event.ilog2() as usize;
        let events = if self.ev_record[index].fetch_add(1, Ordering::Relaxed) == 0 {
            self.events.fetch_or(event, Ordering::Relaxed) | event
        } else {
            self.events.load(Ordering::Relaxed)
        };
        syscall!(
            SOCKET,
            WSAEventSelect(
                socket.as_fd().as_raw_fd() as _,
                ev_object.as_raw_handle() as _,
                events
            )
        )?;
        let _guard = EventGuard {
            wsa_event: self,
            event,
        };
        loop {
            let op = WaitWSAEvent::new(socket, ev_object, event);
            let BufResult(res, op) = crate::submit(op).await;
            WaitWSAEvent {
                socket,
                ev_object,
                ..
            } = op;
            match res {
                Ok(_) => break Ok(()),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) => break Err(e),
            }
        }
    }
}

struct WaitWSAEvent<T> {
    socket: SharedFd<T>,
    ev_object: SharedFd<OwnedHandle>,
    event: i32,
}

impl<T> WaitWSAEvent<T> {
    pub fn new(socket: SharedFd<T>, ev_object: SharedFd<OwnedHandle>, event: i32) -> Self {
        Self {
            socket,
            ev_object,
            event,
        }
    }
}

impl<T> IntoInner for WaitWSAEvent<T> {
    type Inner = SharedFd<OwnedHandle>;

    fn into_inner(self) -> Self::Inner {
        self.ev_object
    }
}

unsafe impl<T: AsFd> OpCode for WaitWSAEvent<T> {
    fn op_type(&self) -> OpType {
        OpType::Event(self.ev_object.as_raw_fd())
    }

    unsafe fn operate(self: Pin<&mut Self>, _optr: *mut OVERLAPPED) -> Poll<io::Result<usize>> {
        let mut events: WSANETWORKEVENTS = unsafe { std::mem::zeroed() };
        syscall!(
            SOCKET,
            WSAEnumNetworkEvents(
                self.socket.as_fd().as_raw_fd() as _,
                self.ev_object.as_raw_handle() as _,
                &mut events
            )
        )?;
        let res = if (events.lNetworkEvents & self.event) != 0 {
            events.iErrorCode[self.event.ilog2() as usize]
        } else {
            ERROR_IO_PENDING as _
        };
        if res == 0 {
            Poll::Ready(Ok(0))
        } else {
            Poll::Ready(Err(io::Error::from_raw_os_error(res)))
        }
    }
}
