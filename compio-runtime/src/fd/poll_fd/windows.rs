use std::{
    cell::RefCell,
    fmt::Debug,
    io,
    ops::Deref,
    os::windows::io::{AsRawHandle, AsRawSocket, FromRawHandle, OwnedHandle, RawSocket},
    pin::Pin,
    ptr::null,
    sync::atomic::Ordering,
    task::{Context, Poll, Waker},
};

use compio_buf::{BufResult, IntoInner};
use compio_driver::{
    AsFd, AsRawFd, BorrowedFd, OpCode, OpType, RawFd, SharedFd, ToSharedFd, syscall,
};
use compio_io::compat::WakerArrayRef;
use synchrony::unsync::atomic::{AtomicI32, AtomicUsize};
use windows_sys::Win32::{
    Networking::WinSock::{
        FD_ACCEPT, FD_CONNECT, FD_MAX_EVENTS, FD_READ, FD_WRITE, WSAEnumNetworkEvents,
        WSAEventSelect, WSANETWORKEVENTS,
    },
    System::{IO::OVERLAPPED, Threading::CreateEventW},
};

use crate::Submit;

pub struct PollFd<T: AsFd> {
    inner: SharedFd<T>,
    event: WSAEvent,
    submit: RefCell<Option<Submit<WaitWSAEvent<T>>>>,
    accept_waker: RefCell<Option<Waker>>,
    connect_waker: RefCell<Option<Waker>>,
    read_waker: RefCell<Option<Waker>>,
    write_waker: RefCell<Option<Waker>>,
}

impl<T: AsFd + Debug> Debug for PollFd<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PollFd")
            .field("inner", &self.inner)
            .finish()
    }
}

impl<T: AsFd> PollFd<T> {
    pub fn new(inner: SharedFd<T>) -> io::Result<Self> {
        Ok(Self {
            inner,
            event: WSAEvent::new()?,
            submit: RefCell::new(None),
            accept_waker: RefCell::new(None),
            connect_waker: RefCell::new(None),
            read_waker: RefCell::new(None),
            write_waker: RefCell::new(None),
        })
    }
}

impl<T: AsFd + 'static> PollFd<T> {
    fn poll_ready(&self, event: u32) -> Poll<io::Result<()>> {
        let mut submit = self.submit.borrow_mut();
        loop {
            match submit.as_mut() {
                None => {
                    let op = self.event.create_op(self.to_shared_fd(), event)?;
                    *submit = Some(crate::submit(op));
                }
                Some(f) => {
                    let accept_waker = self.accept_waker.borrow();
                    let connect_waker = self.connect_waker.borrow();
                    let read_waker = self.read_waker.borrow();
                    let write_waker = self.write_waker.borrow();
                    let waker = WakerArrayRef::new([
                        accept_waker.as_ref(),
                        connect_waker.as_ref(),
                        read_waker.as_ref(),
                        write_waker.as_ref(),
                    ]);
                    match waker.with(|waker| Pin::new(f).poll(&mut Context::from_waker(waker))) {
                        Poll::Ready(BufResult(Ok(_), op)) => {
                            submit.take();
                            let events = op.into_inner();
                            let event = event as i32;
                            self.event.clear_event(&self.inner, events.lNetworkEvents)?;
                            if (events.lNetworkEvents & event) != 0 {
                                let err = events.iErrorCode[event.ilog2() as usize];
                                if err == 0 {
                                    break Poll::Ready(Ok(()));
                                } else {
                                    break Poll::Ready(Err(io::Error::from_raw_os_error(err)));
                                }
                            }
                        }
                        Poll::Ready(BufResult(Err(e), _)) => break Poll::Ready(Err(e)),
                        Poll::Pending => break Poll::Pending,
                    }
                }
            }
        }
    }

    pub fn poll_accept_ready(&self, cx: &mut Context) -> Poll<io::Result<()>> {
        self.accept_waker.borrow_mut().replace(cx.waker().clone());
        self.poll_ready(FD_ACCEPT)
    }

    pub fn poll_connect_ready(&self, cx: &mut Context) -> Poll<io::Result<()>> {
        self.connect_waker.borrow_mut().replace(cx.waker().clone());
        self.poll_ready(FD_CONNECT)
    }

    pub fn poll_read_ready(&self, cx: &mut Context) -> Poll<io::Result<()>> {
        self.read_waker.borrow_mut().replace(cx.waker().clone());
        self.poll_ready(FD_READ)
    }

    pub fn poll_write_ready(&self, cx: &mut Context) -> Poll<io::Result<()>> {
        self.write_waker.borrow_mut().replace(cx.waker().clone());
        self.poll_ready(FD_WRITE)
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
struct WSAEvent {
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

    pub fn create_op<T: AsFd + 'static>(
        &self,
        socket: SharedFd<T>,
        event: u32,
    ) -> io::Result<WaitWSAEvent<T>> {
        let event = event as i32;
        let ev_object = self.ev_object.clone();

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
        Ok(WaitWSAEvent::new(socket, ev_object))
    }

    fn clear_event_inner(&self, event: i32) {
        let index = event.ilog2() as usize;
        if self.ev_record[index].fetch_sub(1, Ordering::Relaxed) == 1 {
            self.events.fetch_add(!event, Ordering::Relaxed);
        }
    }

    pub fn clear_event<T: AsFd>(&self, socket: &T, events: i32) -> io::Result<()> {
        for i in 0..FD_MAX_EVENTS {
            let event = 1 << i;
            if (events & event) != 0 {
                self.clear_event_inner(event);
            }
        }
        syscall!(
            SOCKET,
            WSAEventSelect(
                socket.as_fd().as_raw_fd() as _,
                self.ev_object.as_raw_handle() as _,
                events
            )
        )?;
        Ok(())
    }
}

struct WaitWSAEvent<T> {
    socket: SharedFd<T>,
    ev_object: SharedFd<OwnedHandle>,
    events: WSANETWORKEVENTS,
}

impl<T> WaitWSAEvent<T> {
    pub fn new(socket: SharedFd<T>, ev_object: SharedFd<OwnedHandle>) -> Self {
        Self {
            socket,
            ev_object,
            events: unsafe { std::mem::zeroed() },
        }
    }
}

unsafe impl<T: AsFd> OpCode for WaitWSAEvent<T> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}

    fn op_type(&self, _: &Self::Control) -> OpType {
        OpType::Event(self.ev_object.as_raw_fd())
    }

    unsafe fn operate(
        &mut self,
        _: &mut Self::Control,
        _optr: *mut OVERLAPPED,
    ) -> Poll<io::Result<usize>> {
        syscall!(
            SOCKET,
            WSAEnumNetworkEvents(
                self.socket.as_fd().as_raw_fd() as _,
                self.ev_object.as_raw_handle() as _,
                &mut self.events
            )
        )?;
        Poll::Ready(Ok(0))
    }
}

impl<T: AsFd> IntoInner for WaitWSAEvent<T> {
    type Inner = WSANETWORKEVENTS;

    fn into_inner(self) -> Self::Inner {
        self.events
    }
}
