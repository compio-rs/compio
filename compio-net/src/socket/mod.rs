use std::{
    future::Future,
    io,
    mem::{ManuallyDrop, MaybeUninit},
};

use compio_buf::{
    BufResult, IntoInner, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut, SetLen, buf_try,
};
#[cfg(unix)]
use compio_driver::op::{Bind, CreateSocket, Listen, ShutdownSocket};
use compio_driver::{
    AsRawFd, BufferRef, OpCode, RawFd, ResultTakeBuffer, TakeBuffer, ToSharedFd,
    op::{
        Accept, BufResultExt, CloseSocket, Connect, Recv, RecvFrom, RecvFromManaged, RecvFromMulti,
        RecvFromMultiResult, RecvFromVectored, RecvManaged, RecvMsg, RecvMsgManaged, RecvMsgMulti,
        RecvMsgMultiResult, RecvMulti, RecvResultExt, RecvVectored, Send, SendMsg, SendMsgZc,
        SendTo, SendToVectored, SendToVectoredZc, SendToZc, SendVectored, SendVectoredZc, SendZc,
        VecBufResultExt,
    },
    syscall,
};
use compio_runtime::{Attacher, Runtime, fd::PollFd};
use futures_util::{Stream, StreamExt, future::Either};
use socket2::{Domain, Protocol, SockAddr, Socket as Socket2, Type};
use sys::SocketState;

use crate::Incoming;

cfg_if::cfg_if! {
    if #[cfg(any(
        target_os = "linux", target_os = "android",
        target_os = "hurd",
        target_os = "dragonfly", target_os = "freebsd",
        target_os = "openbsd", target_os = "netbsd",
        target_os = "solaris", target_os = "illumos",
        target_os = "haiku", target_os = "nto",
        target_os = "cygwin"))] {
        pub(crate) use libc::MSG_NOSIGNAL;
    } else {
        pub(crate) const MSG_NOSIGNAL: std::ffi::c_int = 0x0;
    }
}

cfg_if::cfg_if! {
    if #[cfg(target_os = "linux")] {
        #[path = "linux.rs"]
        mod sys;
    } else {
        mod sys {
            #[derive(Default, Clone, Debug)]
            pub(super) struct SocketState;

            impl SocketState {
                pub(super) fn new() -> Self {
                    SocketState
                }

                pub(super) fn get(&self) -> Option<bool> {
                    None
                }

                pub(super) fn set(&self, _: &compio_driver::Extra) {}
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Socket {
    pub(crate) socket: Attacher<Socket2>,
    state: SocketState,
}

impl Socket {
    pub fn from_socket2(socket: Socket2) -> io::Result<Self> {
        Ok(Self {
            socket: Attacher::new(socket)?,
            state: SocketState::new(),
        })
    }

    pub fn peer_addr(&self) -> io::Result<SockAddr> {
        self.socket.peer_addr()
    }

    pub fn local_addr(&self) -> io::Result<SockAddr> {
        self.socket.local_addr()
    }

    pub fn to_poll_fd(&self) -> io::Result<PollFd<Socket2>> {
        PollFd::from_shared_fd(self.to_shared_fd())
    }

    pub fn into_poll_fd(self) -> io::Result<PollFd<Socket2>> {
        PollFd::from_shared_fd(self.socket.into_inner())
    }

    #[cfg(windows)]
    pub async fn new(domain: Domain, ty: Type, protocol: Option<Protocol>) -> io::Result<Self> {
        Self::from_socket2(Socket2::new(domain, ty, protocol)?)
    }

    #[cfg(unix)]
    pub async fn new(domain: Domain, ty: Type, protocol: Option<Protocol>) -> io::Result<Self> {
        let op = CreateSocket::new(
            domain.into(),
            ty.into(),
            protocol.map(|p| p.into()).unwrap_or_default(),
        );
        let (_, op) = buf_try!(@try compio_runtime::submit(op).await);

        Self::from_socket2(op.into_inner())
    }

    pub async fn bind(&self, addr: &SockAddr) -> io::Result<()> {
        #[cfg(not(unix))]
        self.socket.bind(addr)?;
        #[cfg(unix)]
        {
            let op = Bind::new(self.to_shared_fd(), addr.clone());
            compio_runtime::submit(op).await.0?;
        }
        Ok(())
    }

    pub async fn listen(&self, backlog: i32) -> io::Result<()> {
        #[cfg(not(unix))]
        {
            self.socket.listen(backlog)
        }
        #[cfg(unix)]
        {
            let op = Listen::new(self.to_shared_fd(), backlog);
            compio_runtime::submit(op).await.0?;
            Ok(())
        }
    }

    pub fn connect(&self, addr: &SockAddr) -> io::Result<()> {
        self.socket.connect(addr)
    }

    pub async fn connect_async(&self, addr: &SockAddr) -> io::Result<()> {
        let op = Connect::new(self.to_shared_fd(), addr.clone());
        let (_, _op) = buf_try!(@try compio_runtime::submit(op).await);
        #[cfg(windows)]
        _op.update_context()?;
        Ok(())
    }

    #[cfg(unix)]
    pub async fn accept(&self) -> io::Result<(Self, SockAddr)> {
        let op = Accept::new(self.to_shared_fd());
        let (_, op) = buf_try!(@try compio_runtime::submit(op).await);
        let (accept_sock, addr) = op.into_inner();
        let accept_sock = Self::from_socket2(accept_sock)?;
        Ok((accept_sock, addr))
    }

    #[cfg(windows)]
    pub async fn accept(&self) -> io::Result<(Self, SockAddr)> {
        let domain = self.local_addr()?.domain();
        let ty = self.socket.r#type()?;
        let protocol = self.socket.protocol()?;
        let accept_sock = Socket2::new(domain, ty, protocol)?;
        let op = Accept::new(self.to_shared_fd(), accept_sock);
        let (_, op) = buf_try!(@try compio_runtime::submit(op).await);
        op.update_context()?;
        let (accept_sock, addr) = op.into_addr()?;
        Ok((Self::from_socket2(accept_sock)?, addr))
    }

    pub fn incoming(&self) -> Incoming<'_> {
        Incoming::new(self)
    }

    pub fn close(self) -> impl Future<Output = io::Result<()>> {
        // Make sure that self won't be dropped after `close` called.
        // Users may call this method and drop the future immediately. In that way the
        // `close` should be cancelled.
        let this = ManuallyDrop::new(self);
        async move {
            let fd = ManuallyDrop::into_inner(this)
                .socket
                .into_inner()
                .take()
                .await;
            if let Some(fd) = fd {
                let op = CloseSocket::new(fd.into());
                compio_runtime::submit(op).await.0?;
            }
            Ok(())
        }
    }

    #[cfg(unix)]
    async fn shutdown_impl(&self) -> io::Result<()> {
        let op = ShutdownSocket::new(self.to_shared_fd(), std::net::Shutdown::Write);
        compio_runtime::submit(op).await.0.map(|_| ())
    }

    #[cfg(windows)]
    async fn shutdown_impl(&self) -> io::Result<()> {
        self.socket.shutdown(std::net::Shutdown::Write)?;
        Ok(())
    }

    pub async fn shutdown(&self) -> io::Result<()> {
        match self.shutdown_impl().await {
            Ok(_) => Ok(()),
            // The socket is not connected, so we can ignore this error.
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::NotConnected
                        | io::ErrorKind::ConnectionAborted
                        | io::ErrorKind::ConnectionReset
                        | io::ErrorKind::ConnectionRefused
                ) =>
            {
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// This method signifies whether the socket was non-empty after the last
    /// receive operation.
    ///
    /// # Behavior
    ///
    /// It returns `Some(..)` only on the IO_URING driver and `None` on others.
    pub fn sock_nonempty(&self) -> Option<bool> {
        self.state.get()
    }

    pub async fn recv<B: IoBufMut>(&self, buffer: B, flags: i32) -> BufResult<usize, B> {
        let fd = self.to_shared_fd();
        let op = Recv::new(fd, buffer, flags);
        let (res, extra) = compio_runtime::submit(op).with_extra().await;
        self.state.set(&extra);
        let res = res.into_inner();
        unsafe { res.map_advanced() }
    }

    pub async fn recv_vectored<V: IoVectoredBufMut>(
        &self,
        buffer: V,
        flags: i32,
    ) -> BufResult<usize, V> {
        let fd = self.to_shared_fd();
        let op = RecvVectored::new(fd, buffer, flags);
        let (res, extra) = compio_runtime::submit(op).with_extra().await;
        self.state.set(&extra);
        let res = res.into_inner();
        unsafe { res.map_vec_advanced() }
    }

    pub async fn recv_managed(&self, len: usize, flags: i32) -> io::Result<Option<BufferRef>> {
        let fd = self.to_shared_fd();
        let (res, extra) = Runtime::with_current(|rt| {
            let buffer_pool = rt.buffer_pool()?;
            let op = RecvManaged::new(fd, &buffer_pool, len, flags)?;
            io::Result::Ok(rt.submit(op).with_extra())
        })?
        .await;

        self.state.set(&extra);

        unsafe { res.take_buffer() }
    }

    pub fn recv_multi(&self, len: usize, flags: i32) -> impl Stream<Item = io::Result<BufferRef>> {
        let fd = self.to_shared_fd();
        Runtime::with_current(|rt| {
            let buffer_pool = rt.buffer_pool()?;
            let op = RecvMulti::new(fd, &buffer_pool, len, flags)?;
            io::Result::Ok(rt.submit_multi(op).into_managed(buffer_pool))
        })
        .map(Either::Left)
        .unwrap_or_else(|e| Either::Right(futures_util::stream::once(std::future::ready(Err(e)))))
    }

    pub async fn send<T: IoBuf>(&self, buffer: T, flags: i32) -> BufResult<usize, T> {
        let fd = self.to_shared_fd();
        let op = Send::new(fd, buffer, flags);
        compio_runtime::submit(op).await.into_inner()
    }

    pub async fn send_vectored<T: IoVectoredBuf>(
        &self,
        buffer: T,
        flags: i32,
    ) -> BufResult<usize, T> {
        let fd = self.to_shared_fd();
        let op = SendVectored::new(fd, buffer, flags);
        compio_runtime::submit(op).await.into_inner()
    }

    pub async fn send_zerocopy<T: IoBuf>(
        &self,
        buf: T,
        flags: i32,
    ) -> BufResult<usize, impl Future<Output = T> + use<T>> {
        submit_zerocopy(SendZc::new(self.to_shared_fd(), buf, flags)).await
    }

    pub async fn send_zerocopy_vectored<T: IoVectoredBuf>(
        &self,
        buf: T,
        flags: i32,
    ) -> BufResult<usize, impl Future<Output = T> + use<T>> {
        submit_zerocopy(SendVectoredZc::new(self.to_shared_fd(), buf, flags)).await
    }

    pub async fn recv_from<T: IoBufMut>(
        &self,
        buffer: T,
        flags: i32,
    ) -> BufResult<(usize, Option<SockAddr>), T> {
        let fd = self.to_shared_fd();
        let op = RecvFrom::new(fd, buffer, flags);
        let (res, extra) = compio_runtime::submit(op).with_extra().await;
        self.state.set(&extra);
        let res = res.into_inner().map_addr();
        unsafe { res.map_advanced() }
    }

    pub async fn recv_from_vectored<T: IoVectoredBufMut>(
        &self,
        buffer: T,
        flags: i32,
    ) -> BufResult<(usize, Option<SockAddr>), T> {
        let fd = self.to_shared_fd();
        let op = RecvFromVectored::new(fd, buffer, flags);
        let (res, extra) = compio_runtime::submit(op).with_extra().await;
        self.state.set(&extra);
        let res = res.into_inner().map_addr();
        unsafe { res.map_vec_advanced() }
    }

    pub async fn recv_from_managed(
        &self,
        len: usize,
        flags: i32,
    ) -> io::Result<Option<(BufferRef, Option<SockAddr>)>> {
        let fd = self.to_shared_fd();
        let (inner, extra) = Runtime::with_current(|rt| {
            let buffer_pool = rt.buffer_pool()?;
            let op = RecvFromManaged::new(fd, &buffer_pool, len, flags)?;
            io::Result::Ok(rt.submit(op).with_extra())
        })?
        .await;
        self.state.set(&extra);
        let (len, op) = buf_try!(@try inner);
        // Kernel returns 0 for the operation, drop the buffer and return Ok(None)
        if len == 0 {
            return Ok(None);
        }
        let Some((mut buf, addr)) = op.take_buffer() else {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("Read {len} bytes, but no buffer was selected by kernel"),
            ));
        };
        unsafe { buf.advance_to(len) };
        Ok(Some((buf, addr)))
    }

    pub fn recv_from_multi(
        &self,
        flags: i32,
    ) -> impl Stream<Item = io::Result<RecvFromMultiResult>> {
        let fd = self.to_shared_fd();
        Runtime::with_current(|rt| {
            let buffer_pool = rt.buffer_pool()?;
            let op = RecvFromMulti::new(fd, &buffer_pool, flags)?;
            io::Result::Ok(rt.submit_multi(op).into_managed(buffer_pool))
        })
        .map(Either::Left)
        .unwrap_or_else(|e| Either::Right(futures_util::stream::once(std::future::ready(Err(e)))))
    }

    pub async fn recv_msg<T: IoBufMut, C: IoBufMut>(
        &self,
        buffer: T,
        control: C,
        flags: i32,
    ) -> BufResult<(usize, usize, Option<SockAddr>), (T, C)> {
        self.recv_msg_vectored([buffer], control, flags)
            .await
            .map_buffer(|([buffer], control)| (buffer, control))
    }

    pub async fn recv_msg_vectored<T: IoVectoredBufMut, C: IoBufMut>(
        &self,
        buffer: T,
        control: C,
        flags: i32,
    ) -> BufResult<(usize, usize, Option<SockAddr>), (T, C)> {
        let fd = self.to_shared_fd();
        let op = RecvMsg::new(fd, buffer, control, flags);
        let (res, extra) = compio_runtime::submit(op).with_extra().await;
        self.state.set(&extra);
        let res = res.into_inner().map_addr();
        unsafe { res.map_vec_advanced() }
    }

    pub async fn recv_msg_managed<C: IoBufMut>(
        &self,
        len: usize,
        control: C,
        flags: i32,
    ) -> io::Result<Option<(BufferRef, C, Option<SockAddr>)>> {
        let fd = self.to_shared_fd();
        let (inner, extra) = Runtime::with_current(|rt| {
            let buffer_pool = rt.buffer_pool()?;
            let op = RecvMsgManaged::new(fd, &buffer_pool, len, control, flags)?;
            io::Result::Ok(rt.submit(op).with_extra())
        })?
        .await;
        self.state.set(&extra);
        let (len, op) = buf_try!(@try inner);
        // Kernel returns 0 for the operation, drop the buffer and return Ok(None)
        if len == 0 {
            return Ok(None);
        }
        let Some(((mut buf, mut control), addr, control_len)) = op.take_buffer() else {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("Read {len} bytes, but no buffer was selected by kernel"),
            ));
        };
        unsafe { buf.advance_to(len) };
        unsafe { control.advance_to(control_len) };
        Ok(Some((buf, control, addr)))
    }

    pub fn recv_msg_multi(
        &self,
        control_len: usize,
        flags: i32,
    ) -> impl Stream<Item = io::Result<RecvMsgMultiResult>> {
        let fd = self.to_shared_fd();
        Runtime::with_current(|rt| {
            let buffer_pool = rt.buffer_pool()?;
            let op = RecvMsgMulti::new(fd, &buffer_pool, control_len, flags)?;
            io::Result::Ok(
                rt.submit_multi(op)
                    .into_managed_with(buffer_pool, control_len),
            )
        })
        .map(Either::Left)
        .unwrap_or_else(|e| Either::Right(futures_util::stream::once(std::future::ready(Err(e)))))
    }

    pub async fn send_to<T: IoBuf>(
        &self,
        buffer: T,
        addr: &SockAddr,
        flags: i32,
    ) -> BufResult<usize, T> {
        let fd = self.to_shared_fd();
        let op = SendTo::new(fd, buffer, addr.clone(), flags);
        compio_runtime::submit(op).await.into_inner()
    }

    pub async fn send_to_vectored<T: IoVectoredBuf>(
        &self,
        buffer: T,
        addr: &SockAddr,
        flags: i32,
    ) -> BufResult<usize, T> {
        let fd = self.to_shared_fd();
        let op = SendToVectored::new(fd, buffer, addr.clone(), flags);
        compio_runtime::submit(op).await.into_inner()
    }

    pub async fn send_to_zerocopy<T: IoBuf>(
        &self,
        buffer: T,
        addr: &SockAddr,
        flags: i32,
    ) -> BufResult<usize, impl Future<Output = T> + use<T>> {
        let op = SendToZc::new(self.to_shared_fd(), buffer, addr.clone(), flags);
        submit_zerocopy(op).await
    }

    pub async fn send_to_zerocopy_vectored<T: IoVectoredBuf>(
        &self,
        buffer: T,
        addr: &SockAddr,
        flags: i32,
    ) -> BufResult<usize, impl Future<Output = T> + use<T>> {
        let op = SendToVectoredZc::new(self.to_shared_fd(), buffer, addr.clone(), flags);
        submit_zerocopy(op).await
    }

    pub async fn send_msg<T: IoBuf, C: IoBuf>(
        &self,
        buffer: T,
        control: C,
        addr: Option<&SockAddr>,
        flags: i32,
    ) -> BufResult<usize, (T, C)> {
        self.send_msg_vectored([buffer], control, addr, flags)
            .await
            .map_buffer(|([buffer], control)| (buffer, control))
    }

    pub async fn send_msg_vectored<T: IoVectoredBuf, C: IoBuf>(
        &self,
        buffer: T,
        control: C,
        addr: Option<&SockAddr>,
        flags: i32,
    ) -> BufResult<usize, (T, C)> {
        let fd = self.to_shared_fd();
        let op = SendMsg::new(fd, buffer, control, addr.cloned(), flags);
        compio_runtime::submit(op).await.into_inner()
    }

    pub async fn send_msg_zerocopy<T: IoBuf, C: IoBuf>(
        &self,
        buffer: T,
        control: C,
        addr: Option<&SockAddr>,
        flags: i32,
    ) -> BufResult<usize, impl Future<Output = (T, C)> + use<T, C>> {
        self.send_msg_zerocopy_vectored([buffer], control, addr, flags)
            .await
            .map_buffer(|fut| async move {
                let ([buffer], control) = fut.await;
                (buffer, control)
            })
    }

    pub async fn send_msg_zerocopy_vectored<T: IoVectoredBuf, C: IoBuf>(
        &self,
        buffer: T,
        control: C,
        addr: Option<&SockAddr>,
        flags: i32,
    ) -> BufResult<usize, impl Future<Output = (T, C)> + use<T, C>> {
        let fd = self.to_shared_fd();
        let op = SendMsgZc::new(fd, buffer, control, addr.cloned(), flags);
        submit_zerocopy(op).await
    }

    #[cfg(unix)]
    pub unsafe fn get_socket_option<T: Copy>(&self, level: i32, name: i32) -> io::Result<T> {
        let mut value: MaybeUninit<T> = MaybeUninit::uninit();
        let mut len = size_of::<T>() as libc::socklen_t;
        syscall!(libc::getsockopt(
            self.socket.as_raw_fd(),
            level,
            name,
            value.as_mut_ptr() as _,
            &mut len
        ))
        .map(|_| {
            debug_assert_eq!(len as usize, size_of::<T>());
            // SAFETY: The value is initialized by `getsockopt`.
            unsafe { value.assume_init() }
        })
    }

    #[cfg(windows)]
    pub unsafe fn get_socket_option<T: Copy>(&self, level: i32, name: i32) -> io::Result<T> {
        let mut value: MaybeUninit<T> = MaybeUninit::uninit();
        let mut len = size_of::<T>() as i32;
        syscall!(
            SOCKET,
            windows_sys::Win32::Networking::WinSock::getsockopt(
                self.socket.as_raw_fd() as _,
                level,
                name,
                value.as_mut_ptr() as _,
                &mut len
            )
        )
        .map(|_| {
            debug_assert_eq!(len as usize, size_of::<T>());
            // SAFETY: The value is initialized by `getsockopt`.
            unsafe { value.assume_init() }
        })
    }

    #[cfg(unix)]
    pub unsafe fn set_socket_option<T: Copy>(
        &self,
        level: i32,
        name: i32,
        value: &T,
    ) -> io::Result<()> {
        syscall!(libc::setsockopt(
            self.socket.as_raw_fd(),
            level,
            name,
            value as *const _ as _,
            std::mem::size_of::<T>() as _
        ))
        .map(|_| ())
    }

    #[cfg(windows)]
    pub unsafe fn set_socket_option<T: Copy>(
        &self,
        level: i32,
        name: i32,
        value: &T,
    ) -> io::Result<()> {
        syscall!(
            SOCKET,
            windows_sys::Win32::Networking::WinSock::setsockopt(
                self.socket.as_raw_fd() as _,
                level,
                name,
                value as *const _ as _,
                std::mem::size_of::<T>() as _
            )
        )
        .map(|_| ())
    }
}

impl AsRawFd for Socket {
    fn as_raw_fd(&self) -> RawFd {
        self.socket.as_raw_fd()
    }
}

#[cfg(unix)]
impl std::os::fd::AsFd for Socket {
    fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        self.socket.as_fd()
    }
}

#[cfg(unix)]
impl std::os::fd::FromRawFd for Socket {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self {
            socket: unsafe { std::os::fd::FromRawFd::from_raw_fd(fd) },
            state: SocketState::new(),
        }
    }
}

impl compio_driver::ToSharedFd<Socket2> for Socket {
    fn to_shared_fd(&self) -> compio_driver::SharedFd<Socket2> {
        self.socket.to_shared_fd()
    }
}

#[cfg(windows)]
impl std::os::windows::io::FromRawSocket for Socket {
    unsafe fn from_raw_socket(sock: std::os::windows::io::RawSocket) -> Self {
        Self {
            socket: unsafe { std::os::windows::io::FromRawSocket::from_raw_socket(sock) },
            state: SocketState::new(),
        }
    }
}

#[cfg(windows)]
impl std::os::windows::io::AsSocket for Socket {
    fn as_socket(&self) -> std::os::windows::io::BorrowedSocket<'_> {
        self.socket.as_socket()
    }
}

#[cfg(windows)]
impl std::os::windows::io::AsRawSocket for Socket {
    fn as_raw_socket(&self) -> std::os::windows::io::RawSocket {
        self.socket.as_raw_socket()
    }
}

async fn submit_zerocopy<T: OpCode + IntoInner + 'static>(
    op: T,
) -> BufResult<usize, impl Future<Output = T::Inner> + use<T>> {
    let mut stream = compio_runtime::submit_multi(op);
    let res = stream
        .next()
        .await
        .expect("SubmitMulti should yield at least one item")
        .0;

    let fut = async move {
        // we don't need 2nd CQE's result
        _ = stream.next().await;

        stream
            .try_take()
            .map_err(|_| ())
            .expect("Cannot retrieve buffer")
            .into_inner()
    };

    BufResult(res, fut)
}
