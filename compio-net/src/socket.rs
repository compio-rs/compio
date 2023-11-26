use std::{future::Future, io, mem::ManuallyDrop};

use compio_buf::{buf_try, BufResult, IntoInner, IoBuf, IoBufMut, IoVectoredBuf, IoVectoredBufMut};
use compio_driver::op::{
    Accept, BufResultExt, CloseSocket, Connect, Recv, RecvFrom, RecvFromVectored, RecvResultExt,
    RecvVectored, Send, SendTo, SendToVectored, SendVectored, ShutdownSocket,
};
use compio_runtime::{
    impl_attachable, impl_try_as_raw_fd, Attacher, Runtime, TryAsRawFd, TryClone,
};
use socket2::{Domain, Protocol, SockAddr, Socket as Socket2, Type};

#[derive(Debug)]
pub struct Socket {
    socket: Attacher<Socket2>,
}

impl Socket {
    pub fn from_socket2(socket: Socket2) -> Self {
        Self {
            socket: Attacher::new(socket),
        }
    }

    pub fn try_clone(&self) -> io::Result<Self> {
        let socket = self.socket.try_clone()?;
        Ok(Self { socket })
    }

    pub fn peer_addr(&self) -> io::Result<SockAddr> {
        unsafe { self.socket.get_unchecked() }.peer_addr()
    }

    pub fn local_addr(&self) -> io::Result<SockAddr> {
        unsafe { self.socket.get_unchecked() }.local_addr()
    }

    pub fn new(domain: Domain, ty: Type, protocol: Option<Protocol>) -> io::Result<Self> {
        let socket = Socket2::new(domain, ty, protocol)?;
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
        Ok(Self::from_socket2(socket))
    }

    pub fn bind(addr: &SockAddr, ty: Type, protocol: Option<Protocol>) -> io::Result<Self> {
        let socket = Self::new(addr.domain(), ty, protocol)?;
        unsafe { socket.socket.get_unchecked() }.bind(addr)?;
        Ok(socket)
    }

    pub fn listen(&self, backlog: i32) -> io::Result<()> {
        unsafe { self.socket.get_unchecked() }.listen(backlog)
    }

    pub fn connect(&self, addr: &SockAddr) -> io::Result<()> {
        self.socket.try_get()?.connect(addr)
    }

    pub async fn connect_async(&self, addr: &SockAddr) -> io::Result<()> {
        let op = Connect::new(self.try_as_raw_fd()?, addr.clone());
        let BufResult(res, _op) = Runtime::current().submit(op).await;
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
        use compio_driver::FromRawFd;

        let op = Accept::new(self.try_as_raw_fd()?);
        let BufResult(res, op) = Runtime::current().submit(op).await;
        let accept_sock = unsafe { Socket2::from_raw_fd(res? as _) };
        if cfg!(all(
            unix,
            not(all(target_os = "linux", feature = "io-uring"))
        )) {
            accept_sock.set_nonblocking(true)?;
        }
        let accept_sock = Self::from_socket2(accept_sock);
        let addr = op.into_addr();
        Ok((accept_sock, addr))
    }

    #[cfg(windows)]
    pub async fn accept(&self) -> io::Result<(Self, SockAddr)> {
        use compio_driver::AsRawFd;

        let local_addr = self.local_addr()?;
        // We should allow users sending this accepted socket to a new thread.
        let accept_sock = Socket2::new(
            local_addr.domain(),
            unsafe { self.socket.get_unchecked() }.r#type()?,
            unsafe { self.socket.get_unchecked() }.protocol()?,
        )?;
        let op = Accept::new(self.try_as_raw_fd()?, accept_sock.as_raw_fd() as _);
        let BufResult(res, op) = Runtime::current().submit(op).await;
        res?;
        op.update_context()?;
        let addr = op.into_addr()?;
        Ok((Self::from_socket2(accept_sock), addr))
    }

    pub fn close(self) -> impl Future<Output = io::Result<()>> {
        // Make sure that self won't be dropped after `close` called.
        // Users may call this method and drop the future immediately. In that way the
        // `close` should be cancelled.
        let this = ManuallyDrop::new(self);
        async move {
            let op = CloseSocket::new(this.try_as_raw_fd()?);
            Runtime::current().submit(op).await.0?;
            Ok(())
        }
    }

    pub async fn shutdown(&self) -> io::Result<()> {
        let op = ShutdownSocket::new(self.try_as_raw_fd()?, std::net::Shutdown::Write);
        Runtime::current().submit(op).await.0?;
        Ok(())
    }

    pub async fn recv<B: IoBufMut>(&self, buffer: B) -> BufResult<usize, B> {
        let (fd, buffer) = buf_try!(self.try_as_raw_fd(), buffer);
        let op = Recv::new(fd, buffer);
        Runtime::current()
            .submit(op)
            .await
            .into_inner()
            .map_advanced()
    }

    pub async fn recv_vectored<V: IoVectoredBufMut>(&self, buffer: V) -> BufResult<usize, V> {
        let (fd, buffer) = buf_try!(self.try_as_raw_fd(), buffer);
        let op = RecvVectored::new(fd, buffer);
        Runtime::current()
            .submit(op)
            .await
            .into_inner()
            .map_advanced()
    }

    pub async fn send<T: IoBuf>(&self, buffer: T) -> BufResult<usize, T> {
        let (fd, buffer) = buf_try!(self.try_as_raw_fd(), buffer);
        let op = Send::new(fd, buffer);
        Runtime::current().submit(op).await.into_inner()
    }

    pub async fn send_vectored<T: IoVectoredBuf>(&self, buffer: T) -> BufResult<usize, T> {
        let (fd, buffer) = buf_try!(self.try_as_raw_fd(), buffer);
        let op = SendVectored::new(fd, buffer);
        Runtime::current().submit(op).await.into_inner()
    }

    pub async fn recv_from<T: IoBufMut>(&self, buffer: T) -> BufResult<(usize, SockAddr), T> {
        let (fd, buffer) = buf_try!(self.try_as_raw_fd(), buffer);
        let op = RecvFrom::new(fd, buffer);
        Runtime::current()
            .submit(op)
            .await
            .into_inner()
            .map_addr()
            .map_advanced()
    }

    pub async fn recv_from_vectored<T: IoVectoredBufMut>(
        &self,
        buffer: T,
    ) -> BufResult<(usize, SockAddr), T> {
        let (fd, buffer) = buf_try!(self.try_as_raw_fd(), buffer);
        let op = RecvFromVectored::new(fd, buffer);
        Runtime::current()
            .submit(op)
            .await
            .into_inner()
            .map_addr()
            .map_advanced()
    }

    pub async fn send_to<T: IoBuf>(&self, buffer: T, addr: &SockAddr) -> BufResult<usize, T> {
        let (fd, buffer) = buf_try!(self.try_as_raw_fd(), buffer);
        let op = SendTo::new(fd, buffer, addr.clone());
        Runtime::current().submit(op).await.into_inner()
    }

    pub async fn send_to_vectored<T: IoVectoredBuf>(
        &self,
        buffer: T,
        addr: &SockAddr,
    ) -> BufResult<usize, T> {
        let (fd, buffer) = buf_try!(self.try_as_raw_fd(), buffer);
        let op = SendToVectored::new(fd, buffer, addr.clone());
        Runtime::current().submit(op).await.into_inner()
    }
}

impl_try_as_raw_fd!(Socket, socket);

impl_attachable!(Socket, socket);
