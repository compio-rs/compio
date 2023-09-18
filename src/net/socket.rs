use std::{io, net::Shutdown};

use socket2::{Domain, Protocol, SockAddr, Socket as Socket2, Type};

use crate::impl_raw_fd;
#[cfg(feature = "runtime")]
use crate::{
    buf::{IntoInner, IoBuf, IoBufMut, VectoredBufWrapper},
    buf_try,
    driver::AsRawFd,
    op::{
        Accept, BufResultExt, Connect, Recv, RecvFrom, RecvFromVectored, RecvResultExt,
        RecvVectored, Send, SendTo, SendToVectored, SendVectored,
    },
    task::RUNTIME,
    Attacher, BufResult,
};

pub struct Socket {
    socket: Socket2,
    #[cfg(feature = "runtime")]
    attacher: Attacher,
}

impl Socket {
    pub fn from_socket2(socket: Socket2) -> Self {
        Self {
            socket,
            #[cfg(feature = "runtime")]
            attacher: Attacher::new(),
        }
    }

    #[cfg(feature = "runtime")]
    pub(crate) fn attach(&self) -> io::Result<()> {
        self.attacher.attach(self)
    }

    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            socket: self.socket.try_clone()?,
            #[cfg(feature = "runtime")]
            attacher: self.attacher.clone(),
        })
    }

    pub fn peer_addr(&self) -> io::Result<SockAddr> {
        self.socket.peer_addr()
    }

    pub fn local_addr(&self) -> io::Result<SockAddr> {
        self.socket.local_addr()
    }

    pub fn new(domain: Domain, ty: Type, protocol: Option<Protocol>) -> io::Result<Self> {
        let socket = Socket2::new(domain, ty, protocol)?;
        // On Linux we use blocking socket
        // Newer kernels have the patch that allows to arm io_uring poll mechanism for
        // non blocking socket when there is no connections in listen queue
        //
        // https://patchwork.kernel.org/project/linux-block/patch/f999615b-205c-49b7-b272-c4e42e45e09d@kernel.dk/#22949861
        #[cfg(all(unix, not(target_os = "linux")))]
        socket.set_nonblocking(true)?;
        Ok(Self::from_socket2(socket))
    }

    pub fn bind(addr: &SockAddr, ty: Type, protocol: Option<Protocol>) -> io::Result<Self> {
        let socket = Self::new(addr.domain(), ty, protocol)?;
        socket.socket.bind(addr)?;
        Ok(socket)
    }

    pub fn listen(&self, backlog: i32) -> io::Result<()> {
        self.socket.listen(backlog)
    }

    pub fn shutdown(&self, how: Shutdown) -> io::Result<()> {
        self.socket.shutdown(how)
    }

    pub fn connect(&self, addr: &SockAddr) -> io::Result<()> {
        self.socket.connect(addr)
    }

    #[cfg(feature = "runtime")]
    pub async fn connect_async(&self, addr: &SockAddr) -> io::Result<()> {
        self.attach()?;
        let op = Connect::new(self.as_raw_fd(), addr.clone());
        let (res, _op) = RUNTIME.with(|runtime| runtime.submit(op)).await;
        #[cfg(target_os = "windows")]
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

    #[cfg(all(feature = "runtime", unix))]
    pub async fn accept(&self) -> io::Result<(Self, SockAddr)> {
        use std::os::fd::FromRawFd;

        self.attach()?;
        let op = Accept::new(self.as_raw_fd());
        let (res, op) = RUNTIME.with(|runtime| runtime.submit(op)).await;
        let accept_sock = unsafe { Socket2::from_raw_fd(res? as _) };
        accept_sock.set_nonblocking(true)?;
        let accept_sock = Self::from_socket2(accept_sock);
        let addr = op.into_addr();
        Ok((accept_sock, addr))
    }

    #[cfg(all(feature = "runtime", target_os = "windows"))]
    pub async fn accept(&self) -> io::Result<(Self, SockAddr)> {
        self.attach()?;
        let local_addr = self.local_addr()?;
        let accept_sock = Self::new(
            local_addr.domain(),
            self.socket.r#type()?,
            self.socket.protocol()?,
        )?;
        let op = Accept::new(self.as_raw_fd(), accept_sock.as_raw_fd() as _);
        let (res, op) = RUNTIME.with(|runtime| runtime.submit(op)).await;
        res?;
        op.update_context()?;
        let addr = op.into_addr()?;
        Ok((accept_sock, addr))
    }

    #[cfg(feature = "runtime")]
    pub async fn recv<T: IoBufMut<'static>>(&self, buffer: T) -> BufResult<usize, T> {
        let ((), buffer) = buf_try!(self.attach(), buffer);
        let op = Recv::new(self.as_raw_fd(), buffer);
        RUNTIME
            .with(|runtime| runtime.submit(op))
            .await
            .into_inner()
            .map_advanced()
            .into_inner()
    }

    #[cfg(feature = "runtime")]
    pub async fn recv_exact<T: IoBufMut<'static>>(&self, mut buffer: T) -> BufResult<usize, T> {
        let need = buffer.as_uninit_slice().len();
        let mut total_read = 0;
        let mut read;
        while total_read < need {
            (read, buffer) = buf_try!(self.recv(buffer).await);
            total_read += read;
        }
        let res = if total_read < need {
            Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "failed to fill whole buffer",
            ))
        } else {
            Ok(total_read)
        };
        (res, buffer)
    }

    #[cfg(feature = "runtime")]
    pub async fn recv_vectored<T: IoBufMut<'static>>(
        &self,
        buffer: VectoredBufWrapper<'static, T>,
    ) -> BufResult<usize, VectoredBufWrapper<'static, T>> {
        let ((), buffer) = buf_try!(self.attach(), buffer);
        let op = RecvVectored::new(self.as_raw_fd(), buffer);
        RUNTIME
            .with(|runtime| runtime.submit(op))
            .await
            .into_inner()
            .map_advanced()
    }

    #[cfg(feature = "runtime")]
    pub async fn send<T: IoBuf<'static>>(&self, buffer: T) -> BufResult<usize, T> {
        let ((), buffer) = buf_try!(self.attach(), buffer);
        let op = Send::new(self.as_raw_fd(), buffer);
        RUNTIME
            .with(|runtime| runtime.submit(op))
            .await
            .into_inner()
            .into_inner()
    }

    #[cfg(feature = "runtime")]
    pub async fn send_all<T: IoBuf<'static>>(&self, mut buffer: T) -> BufResult<usize, T> {
        let buf_len = buffer.buf_len();
        let mut total_written = 0;
        let mut written;
        while total_written < buf_len {
            (written, buffer) =
                buf_try!(self.send(buffer.slice(total_written..)).await.into_inner());
            total_written += written;
        }
        (Ok(total_written), buffer)
    }

    #[cfg(feature = "runtime")]
    pub async fn send_vectored<T: IoBuf<'static>>(
        &self,
        buffer: VectoredBufWrapper<'static, T>,
    ) -> BufResult<usize, VectoredBufWrapper<'static, T>> {
        let ((), buffer) = buf_try!(self.attach(), buffer);
        let op = SendVectored::new(self.as_raw_fd(), buffer);
        RUNTIME
            .with(|runtime| runtime.submit(op))
            .await
            .into_inner()
    }

    #[cfg(feature = "runtime")]
    pub async fn recv_from<T: IoBufMut<'static>>(
        &self,
        buffer: T,
    ) -> BufResult<(usize, SockAddr), T> {
        let ((), buffer) = buf_try!(self.attach(), buffer);
        let op = RecvFrom::new(self.as_raw_fd(), buffer);
        RUNTIME
            .with(|runtime| runtime.submit(op))
            .await
            .into_inner()
            .map_addr()
            .map_advanced()
            .into_inner()
    }

    #[cfg(feature = "runtime")]
    pub async fn recv_from_vectored<T: IoBufMut<'static>>(
        &self,
        buffer: VectoredBufWrapper<'static, T>,
    ) -> BufResult<(usize, SockAddr), VectoredBufWrapper<'static, T>> {
        let ((), buffer) = buf_try!(self.attach(), buffer);
        let op = RecvFromVectored::new(self.as_raw_fd(), buffer);
        RUNTIME
            .with(|runtime| runtime.submit(op))
            .await
            .into_inner()
            .map_addr()
            .map_advanced()
    }

    #[cfg(feature = "runtime")]
    pub async fn send_to<T: IoBuf<'static>>(
        &self,
        buffer: T,
        addr: &SockAddr,
    ) -> BufResult<usize, T> {
        let ((), buffer) = buf_try!(self.attach(), buffer);
        let op = SendTo::new(self.as_raw_fd(), buffer, addr.clone());
        RUNTIME
            .with(|runtime| runtime.submit(op))
            .await
            .into_inner()
            .into_inner()
    }

    #[cfg(feature = "runtime")]
    pub async fn send_to_vectored<T: IoBuf<'static>>(
        &self,
        buffer: VectoredBufWrapper<'static, T>,
        addr: &SockAddr,
    ) -> BufResult<usize, VectoredBufWrapper<'static, T>> {
        let ((), buffer) = buf_try!(self.attach(), buffer);
        let op = SendToVectored::new(self.as_raw_fd(), buffer, addr.clone());
        RUNTIME
            .with(|runtime| runtime.submit(op))
            .await
            .into_inner()
    }
}

impl_raw_fd!(Socket, socket, attacher);
