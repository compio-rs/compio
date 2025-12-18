use std::{
    future::Future,
    io,
    mem::{ManuallyDrop, MaybeUninit},
};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};
#[cfg(unix)]
use compio_driver::op::CreateSocket;
use compio_driver::{
    AsRawFd, ToSharedFd, impl_raw_fd,
    op::{
        Accept, CloseSocket, Connect, Recv, RecvFrom, RecvFromVectored, RecvManaged, RecvMsg,
        RecvResultExt, RecvVectored, ResultTakeBuffer, Send, SendMsg, SendTo, SendToVectored,
        SendVectored, ShutdownSocket,
    },
    syscall,
};
use compio_runtime::{Attacher, BorrowedBuffer, BufferPool};
use socket2::{Domain, Protocol, SockAddr, Socket as Socket2, Type};

use crate::PollFd;

#[derive(Debug, Clone)]
pub struct Socket {
    pub(crate) socket: Attacher<Socket2>,
}

impl Socket {
    pub fn from_socket2(socket: Socket2) -> io::Result<Self> {
        Ok(Self {
            socket: Attacher::new(socket)?,
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
        use std::panic::resume_unwind;

        let socket = compio_runtime::spawn_blocking(move || Socket2::new(domain, ty, protocol))
            .await
            .unwrap_or_else(|e| resume_unwind(e))?;
        Self::from_socket2(socket)
    }

    #[cfg(unix)]
    pub async fn new(domain: Domain, ty: Type, protocol: Option<Protocol>) -> io::Result<Self> {
        use std::os::fd::FromRawFd;

        let op = CreateSocket::new(
            domain.into(),
            ty.into(),
            protocol.map(|p| p.into()).unwrap_or_default(),
        );
        let BufResult(res, _) = compio_runtime::submit(op).await;
        let socket = unsafe { Socket2::from_raw_fd(res? as _) };

        Self::from_socket2(socket)
    }

    pub async fn bind(addr: &SockAddr, ty: Type, protocol: Option<Protocol>) -> io::Result<Self> {
        let socket = Self::new(addr.domain(), ty, protocol).await?;
        socket.socket.bind(addr)?;
        Ok(socket)
    }

    pub fn listen(&self, backlog: i32) -> io::Result<()> {
        self.socket.listen(backlog)
    }

    pub fn connect(&self, addr: &SockAddr) -> io::Result<()> {
        self.socket.connect(addr)
    }

    pub async fn connect_async(&self, addr: &SockAddr) -> io::Result<()> {
        let op = Connect::new(self.to_shared_fd(), addr.clone());
        let BufResult(res, _op) = compio_runtime::submit(op).await;
        #[cfg(windows)]
        {
            res?;
            _op.update_context()?;
            Ok(())
        }
        #[cfg(unix)]
        {
            res.map(|_| ())
        }
    }

    #[cfg(unix)]
    pub async fn accept(&self) -> io::Result<(Self, SockAddr)> {
        use std::os::fd::FromRawFd;

        let op = Accept::new(self.to_shared_fd());
        let BufResult(res, op) = compio_runtime::submit(op).await;
        let addr = op.into_addr();
        let accept_sock = unsafe { Socket2::from_raw_fd(res? as _) };
        let accept_sock = Self::from_socket2(accept_sock)?;
        Ok((accept_sock, addr))
    }

    #[cfg(windows)]
    pub async fn accept(&self) -> io::Result<(Self, SockAddr)> {
        use std::panic::resume_unwind;

        let domain = self.local_addr()?.domain();
        // We should allow users sending this accepted socket to a new thread.
        let ty = self.socket.r#type()?;
        let protocol = self.socket.protocol()?;
        let accept_sock =
            compio_runtime::spawn_blocking(move || Socket2::new(domain, ty, protocol))
                .await
                .unwrap_or_else(|e| resume_unwind(e))?;
        let op = Accept::new(self.to_shared_fd(), accept_sock);
        let BufResult(res, op) = compio_runtime::submit(op).await;
        res?;
        op.update_context()?;
        let (accept_sock, addr) = op.into_addr()?;
        Ok((Self::from_socket2(accept_sock)?, addr))
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

    pub async fn shutdown(&self) -> io::Result<()> {
        let op = ShutdownSocket::new(self.to_shared_fd(), std::net::Shutdown::Write);
        compio_runtime::submit(op).await.0?;
        Ok(())
    }

    pub async fn recv<B: IoBufMut>(&self, buffer: B, flags: i32) -> BufResult<usize, B> {
        let fd = self.to_shared_fd();
        let op = Recv::new(fd, buffer, flags);
        compio_runtime::submit(op).await.into_inner()
    }

    pub async fn recv_vectored<V: IoVectoredBufMut>(
        &self,
        buffer: V,
        flags: i32,
    ) -> BufResult<usize, V> {
        let fd = self.to_shared_fd();
        let op = RecvVectored::new(fd, buffer, flags);
        compio_runtime::submit(op).await.into_inner()
    }

    pub async fn recv_managed<'a>(
        &self,
        buffer_pool: &'a BufferPool,
        len: usize,
        flags: i32,
    ) -> io::Result<BorrowedBuffer<'a>> {
        let fd = self.to_shared_fd();
        let buffer_pool = buffer_pool.try_inner()?;
        let op = RecvManaged::new(fd, buffer_pool, len, flags)?;
        compio_runtime::submit_with_extra(op)
            .await
            .take_buffer(buffer_pool)
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

    pub async fn recv_from<T: IoBufMut>(
        &self,
        buffer: T,
        flags: i32,
    ) -> BufResult<(usize, SockAddr), T> {
        let fd = self.to_shared_fd();
        let op = RecvFrom::new(fd, buffer, flags);
        compio_runtime::submit(op).await.into_inner().map_addr()
    }

    pub async fn recv_from_vectored<T: IoVectoredBufMut>(
        &self,
        buffer: T,
        flags: i32,
    ) -> BufResult<(usize, SockAddr), T> {
        let fd = self.to_shared_fd();
        let op = RecvFromVectored::new(fd, buffer, flags);
        compio_runtime::submit(op).await.into_inner().map_addr()
    }

    pub async fn recv_msg<T: IoBufMut, C: IoBufMut>(
        &self,
        buffer: T,
        control: C,
        flags: i32,
    ) -> BufResult<(usize, usize, SockAddr), (T, C)> {
        self.recv_msg_vectored([buffer], control, flags)
            .await
            .map_buffer(|([buffer], control)| (buffer, control))
    }

    pub async fn recv_msg_vectored<T: IoVectoredBufMut, C: IoBufMut>(
        &self,
        buffer: T,
        control: C,
        flags: i32,
    ) -> BufResult<(usize, usize, SockAddr), (T, C)> {
        let fd = self.to_shared_fd();
        let op = RecvMsg::new(fd, buffer, control, flags);
        compio_runtime::submit(op).await.into_inner().map_addr()
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

    pub async fn send_msg<T: IoBuf, C: IoBuf>(
        &self,
        buffer: T,
        control: C,
        addr: &SockAddr,
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
        addr: &SockAddr,
        flags: i32,
    ) -> BufResult<usize, (T, C)> {
        let fd = self.to_shared_fd();
        let op = SendMsg::new(fd, buffer, control, addr.clone(), flags);
        compio_runtime::submit(op).await.into_inner()
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

impl_raw_fd!(Socket, Socket2, socket, socket);
