use std::{future::Future, io, mem::ManuallyDrop};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};
#[cfg(unix)]
use compio_driver::op::CreateSocket;
use compio_driver::{
    impl_raw_fd,
    op::{
        Accept, BufResultExt, CloseSocket, Connect, Recv, RecvFrom, RecvFromVectored,
        RecvResultExt, RecvVectored, Send, SendTo, SendToVectored, SendVectored, ShutdownSocket,
    },
    ToSharedFd,
};
use compio_runtime::Attacher;
use socket2::{Domain, Protocol, SockAddr, Socket as Socket2, Type};

use crate::PollFd;

#[derive(Debug, Clone)]
pub struct Socket {
    socket: Attacher<Socket2>,
}

impl Socket {
    pub fn from_socket2(socket: Socket2) -> io::Result<Self> {
        #[cfg(unix)]
        {
            #[cfg(not(any(
                target_os = "android",
                target_os = "dragonfly",
                target_os = "freebsd",
                target_os = "fuchsia",
                target_os = "hurd",
                target_os = "illumos",
                target_os = "linux",
                target_os = "netbsd",
                target_os = "openbsd",
                target_os = "espidf",
                target_os = "vita",
            )))]
            socket.set_cloexec(true)?;
            #[cfg(any(
                target_os = "ios",
                target_os = "macos",
                target_os = "tvos",
                target_os = "watchos",
            ))]
            socket.set_nosigpipe(true)?;
            // On Linux we use blocking socket
            // Newer kernels have the patch that allows to arm io_uring poll mechanism for
            // non blocking socket when there is no connections in listen queue
            //
            // https://patchwork.kernel.org/project/linux-block/patch/f999615b-205c-49b7-b272-c4e42e45e09d@kernel.dk/#22949861
            if cfg!(all(
                unix,
                not(all(target_os = "linux", feature = "io-uring"))
            )) {
                socket.set_nonblocking(true)?;
            }
        }

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

        #[allow(unused_mut)]
        let mut ty: i32 = ty.into();
        #[cfg(any(
            target_os = "android",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "fuchsia",
            target_os = "hurd",
            target_os = "illumos",
            target_os = "linux",
            target_os = "netbsd",
            target_os = "openbsd",
        ))]
        {
            ty |= libc::SOCK_CLOEXEC;
        }

        let op = CreateSocket::new(
            domain.into(),
            ty,
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
        let accept_sock = unsafe { Socket2::from_raw_fd(res? as _) };
        if cfg!(all(
            unix,
            not(all(target_os = "linux", feature = "io-uring"))
        )) {
            accept_sock.set_nonblocking(true)?;
        }
        let accept_sock = Self::from_socket2(accept_sock)?;
        let addr = op.into_addr();
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

    pub async fn recv<B: IoBufMut>(&self, buffer: B) -> BufResult<usize, B> {
        let fd = self.to_shared_fd();
        let op = Recv::new(fd, buffer);
        compio_runtime::submit(op).await.into_inner().map_advanced()
    }

    pub async fn recv_vectored<V: IoVectoredBufMut>(&self, buffer: V) -> BufResult<usize, V> {
        let fd = self.to_shared_fd();
        let op = RecvVectored::new(fd, buffer);
        compio_runtime::submit(op).await.into_inner().map_advanced()
    }

    pub async fn send<T: IoBuf>(&self, buffer: T) -> BufResult<usize, T> {
        let fd = self.to_shared_fd();
        let op = Send::new(fd, buffer);
        compio_runtime::submit(op).await.into_inner()
    }

    pub async fn send_vectored<T: IoVectoredBuf>(&self, buffer: T) -> BufResult<usize, T> {
        let fd = self.to_shared_fd();
        let op = SendVectored::new(fd, buffer);
        compio_runtime::submit(op).await.into_inner()
    }

    pub async fn recv_from<T: IoBufMut>(&self, buffer: T) -> BufResult<(usize, SockAddr), T> {
        let fd = self.to_shared_fd();
        let op = RecvFrom::new(fd, buffer);
        compio_runtime::submit(op)
            .await
            .into_inner()
            .map_addr()
            .map_advanced()
    }

    pub async fn recv_from_vectored<T: IoVectoredBufMut>(
        &self,
        buffer: T,
    ) -> BufResult<(usize, SockAddr), T> {
        let fd = self.to_shared_fd();
        let op = RecvFromVectored::new(fd, buffer);
        compio_runtime::submit(op)
            .await
            .into_inner()
            .map_addr()
            .map_advanced()
    }

    pub async fn send_to<T: IoBuf>(&self, buffer: T, addr: &SockAddr) -> BufResult<usize, T> {
        let fd = self.to_shared_fd();
        let op = SendTo::new(fd, buffer, addr.clone());
        compio_runtime::submit(op).await.into_inner()
    }

    pub async fn send_to_vectored<T: IoVectoredBuf>(
        &self,
        buffer: T,
        addr: &SockAddr,
    ) -> BufResult<usize, T> {
        let fd = self.to_shared_fd();
        let op = SendToVectored::new(fd, buffer, addr.clone());
        compio_runtime::submit(op).await.into_inner()
    }
}

impl_raw_fd!(Socket, Socket2, socket, socket);
