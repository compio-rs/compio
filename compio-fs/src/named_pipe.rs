//! [Windows named pipes](https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipes).
//!
//! The infrastructure of the code comes from tokio.

#[cfg(doc)]
use std::ptr::null_mut;
use std::{ffi::OsStr, io, ptr::null};

use compio_buf::{BufResult, IoBuf, IoBufMut};
use compio_driver::{op::ConnectNamedPipe, syscall, FromRawFd, RawFd};
use compio_io::{AsyncRead, AsyncReadAt, AsyncWrite, AsyncWriteAt};
use compio_runtime::{impl_attachable, impl_try_as_raw_fd, Runtime, TryAsRawFd};
use widestring::U16CString;
use windows_sys::Win32::{
    Security::SECURITY_ATTRIBUTES,
    Storage::FileSystem::{
        FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_INBOUND,
        PIPE_ACCESS_OUTBOUND, WRITE_DAC, WRITE_OWNER,
    },
    System::{
        Pipes::{
            CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeInfo, PIPE_ACCEPT_REMOTE_CLIENTS,
            PIPE_READMODE_BYTE, PIPE_READMODE_MESSAGE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_SERVER_END,
            PIPE_TYPE_BYTE, PIPE_TYPE_MESSAGE, PIPE_UNLIMITED_INSTANCES,
        },
        SystemServices::ACCESS_SYSTEM_SECURITY,
    },
};

use crate::{File, OpenOptions};

/// A [Windows named pipe] server.
///
/// Accepting client connections involves creating a server with
/// [`ServerOptions::create`] and waiting for clients to connect using
/// [`NamedPipeServer::connect`].
///
/// To avoid having clients sporadically fail with
/// [`std::io::ErrorKind::NotFound`] when they connect to a server, we must
/// ensure that at least one server instance is available at all times. This
/// means that the typical listen loop for a server is a bit involved, because
/// we have to ensure that we never drop a server accidentally while a client
/// might connect.
///
/// So a correctly implemented server looks like this:
///
/// ```no_run
/// use std::io;
///
/// use compio_fs::named_pipe::ServerOptions;
///
/// const PIPE_NAME: &str = r"\\.\pipe\named-pipe-idiomatic-server";
///
/// # fn main() -> std::io::Result<()> {
/// // The first server needs to be constructed early so that clients can
/// // be correctly connected. Otherwise calling .wait will cause the client to
/// // error.
/// //
/// // Here we also make use of `first_pipe_instance`, which will ensure that
/// // there are no other servers up and running already.
/// let mut server = ServerOptions::new()
///     .first_pipe_instance(true)
///     .create(PIPE_NAME)?;
///
/// // Spawn the server loop.
/// # compio_runtime::Runtime::new().unwrap().block_on(async move {
/// loop {
///     // Wait for a client to connect.
///     let connected = server.connect().await?;
///
///     // Construct the next server to be connected before sending the one
///     // we already have of onto a task. This ensures that the server
///     // isn't closed (after it's done in the task) before a new one is
///     // available. Otherwise the client might error with
///     // `io::ErrorKind::NotFound`.
///     server = ServerOptions::new().create(PIPE_NAME)?;
///
///     let client = compio_runtime::spawn(async move {
///         // use the connected client
/// #       Ok::<_, std::io::Error>(())
///     });
/// # if true { break } // needed for type inference to work
/// }
/// # Ok::<_, io::Error>(())
/// # })
/// # }
/// ```
///
/// [Windows named pipe]: https://docs.microsoft.com/en-us/windows/win32/ipc/named-pipes
#[derive(Debug)]
pub struct NamedPipeServer {
    handle: File,
}

impl NamedPipeServer {
    /// Creates a new independently owned handle to the underlying file handle.
    ///
    /// It does not clear the attach state.
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            handle: self.handle.try_clone()?,
        })
    }

    /// Retrieves information about the named pipe the server is associated
    /// with.
    ///
    /// ```no_run
    /// use compio_fs::named_pipe::{PipeEnd, PipeMode, ServerOptions};
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\compio-named-pipe-server-info";
    ///
    /// # compio_runtime::Runtime::new().unwrap().block_on(async move {
    /// let server = ServerOptions::new()
    ///     .pipe_mode(PipeMode::Message)
    ///     .max_instances(5)
    ///     .create(PIPE_NAME)?;
    ///
    /// let server_info = server.info()?;
    ///
    /// assert_eq!(server_info.end, PipeEnd::Server);
    /// assert_eq!(server_info.mode, PipeMode::Message);
    /// assert_eq!(server_info.max_instances, 5);
    /// # std::io::Result::Ok(()) });
    /// ```
    pub fn info(&self) -> io::Result<PipeInfo> {
        // Safety: we're ensuring the lifetime of the named pipe.
        // Safety: getting info doesn't need to be attached.
        unsafe { named_pipe_info(self.as_raw_fd_unchecked()) }
    }

    /// Enables a named pipe server process to wait for a client process to
    /// connect to an instance of a named pipe. A client process connects by
    /// creating a named pipe with the same name.
    ///
    /// This corresponds to the [`ConnectNamedPipe`] system call.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use compio_fs::named_pipe::ServerOptions;
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\mynamedpipe";
    ///
    /// # compio_runtime::Runtime::new().unwrap().block_on(async move {
    /// let pipe = ServerOptions::new().create(PIPE_NAME)?;
    ///
    /// // Wait for a client to connect.
    /// pipe.connect().await?;
    ///
    /// // Use the connected client...
    /// # std::io::Result::Ok(()) });
    /// ```
    pub async fn connect(&self) -> io::Result<()> {
        let op = ConnectNamedPipe::new(self.handle.try_as_raw_fd()?);
        Runtime::current().submit(op).await.0?;
        Ok(())
    }

    /// Disconnects the server end of a named pipe instance from a client
    /// process.
    ///
    /// ```
    /// use compio_fs::named_pipe::{ClientOptions, ServerOptions};
    /// use compio_io::AsyncWrite;
    /// use windows_sys::Win32::Foundation::ERROR_PIPE_NOT_CONNECTED;
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\compio-named-pipe-disconnect";
    ///
    /// # compio_runtime::Runtime::new().unwrap().block_on(async move {
    /// let server = ServerOptions::new().create(PIPE_NAME).unwrap();
    ///
    /// let mut client = ClientOptions::new().open(PIPE_NAME).await.unwrap();
    ///
    /// // Wait for a client to become connected.
    /// server.connect().await.unwrap();
    ///
    /// // Forcibly disconnect the client.
    /// server.disconnect().unwrap();
    ///
    /// // Write fails with an OS-specific error after client has been
    /// // disconnected.
    /// let e = client.write("ping").await.0.unwrap_err();
    /// assert_eq!(e.raw_os_error(), Some(ERROR_PIPE_NOT_CONNECTED as i32));
    /// # })
    /// ```
    pub fn disconnect(&self) -> io::Result<()> {
        syscall!(BOOL, DisconnectNamedPipe(self.try_as_raw_fd()? as _))?;
        Ok(())
    }
}

impl AsyncRead for NamedPipeServer {
    #[inline]
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        (&*self).read(buf).await
    }
}

impl AsyncRead for &NamedPipeServer {
    #[inline]
    async fn read<B: IoBufMut>(&mut self, buffer: B) -> BufResult<usize, B> {
        // The position is ignored.
        self.handle.read_at(buffer, 0).await
    }
}

impl AsyncWrite for NamedPipeServer {
    #[inline]
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        (&*self).write(buf).await
    }

    #[inline]
    async fn flush(&mut self) -> io::Result<()> {
        (&*self).flush().await
    }

    #[inline]
    async fn shutdown(&mut self) -> io::Result<()> {
        (&*self).shutdown().await
    }
}

impl AsyncWrite for &NamedPipeServer {
    #[inline]
    async fn write<T: IoBuf>(&mut self, buffer: T) -> BufResult<usize, T> {
        // The position is ignored.
        (&self.handle).write_at(buffer, 0).await
    }

    #[inline]
    async fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }

    #[inline]
    async fn shutdown(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl_try_as_raw_fd!(NamedPipeServer, handle);

impl_attachable!(NamedPipeServer, handle);

/// A [Windows named pipe] client.
///
/// Constructed using [`ClientOptions::open`].
///
/// Connecting a client correctly involves a few steps. When connecting through
/// [`ClientOptions::open`], it might error indicating one of two things:
///
/// * [`std::io::ErrorKind::NotFound`] - There is no server available.
/// * [`ERROR_PIPE_BUSY`] - There is a server available, but it is busy. Sleep
///   for a while and try again.
///
/// So a correctly implemented client looks like this:
///
/// ```no_run
/// use std::time::Duration;
///
/// use compio_fs::named_pipe::ClientOptions;
/// use compio_runtime::time;
/// use windows_sys::Win32::Foundation::ERROR_PIPE_BUSY;
///
/// const PIPE_NAME: &str = r"\\.\pipe\named-pipe-idiomatic-client";
///
/// # compio_runtime::Runtime::new().unwrap().block_on(async move {
/// let client = loop {
///     match ClientOptions::new().open(PIPE_NAME).await {
///         Ok(client) => break client,
///         Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => (),
///         Err(e) => return Err(e),
///     }
///
///     time::sleep(Duration::from_millis(50)).await;
/// };
///
/// // use the connected client
/// # Ok(()) });
/// ```
///
/// [`ERROR_PIPE_BUSY`]: https://docs.rs/windows-sys/latest/windows_sys/Win32/Foundation/constant.ERROR_PIPE_BUSY.html
/// [Windows named pipe]: https://docs.microsoft.com/en-us/windows/win32/ipc/named-pipes
#[derive(Debug)]
pub struct NamedPipeClient {
    handle: File,
}

impl NamedPipeClient {
    /// Creates a new independently owned handle to the underlying file handle.
    ///
    /// It does not clear the attach state.
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            handle: self.handle.try_clone()?,
        })
    }

    /// Retrieves information about the named pipe the client is associated
    /// with.
    ///
    /// ```no_run
    /// use compio_fs::named_pipe::{ClientOptions, PipeEnd, PipeMode};
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\compio-named-pipe-client-info";
    ///
    /// # compio_runtime::Runtime::new().unwrap().block_on(async move {
    /// let client = ClientOptions::new().open(PIPE_NAME).await?;
    ///
    /// let client_info = client.info()?;
    ///
    /// assert_eq!(client_info.end, PipeEnd::Client);
    /// assert_eq!(client_info.mode, PipeMode::Message);
    /// assert_eq!(client_info.max_instances, 5);
    /// # std::io::Result::Ok(()) });
    /// ```
    pub fn info(&self) -> io::Result<PipeInfo> {
        // Safety: we're ensuring the lifetime of the named pipe.
        // Safety: getting info doesn't need to be attached.
        unsafe { named_pipe_info(self.as_raw_fd_unchecked()) }
    }
}

impl AsyncRead for NamedPipeClient {
    #[inline]
    async fn read<B: IoBufMut>(&mut self, buf: B) -> BufResult<usize, B> {
        (&*self).read(buf).await
    }
}

impl AsyncRead for &NamedPipeClient {
    #[inline]
    async fn read<B: IoBufMut>(&mut self, buffer: B) -> BufResult<usize, B> {
        // The position is ignored.
        self.handle.read_at(buffer, 0).await
    }
}

impl AsyncWrite for NamedPipeClient {
    #[inline]
    async fn write<T: IoBuf>(&mut self, buf: T) -> BufResult<usize, T> {
        (&*self).write(buf).await
    }

    #[inline]
    async fn flush(&mut self) -> io::Result<()> {
        (&*self).flush().await
    }

    #[inline]
    async fn shutdown(&mut self) -> io::Result<()> {
        (&*self).shutdown().await
    }
}

impl AsyncWrite for &NamedPipeClient {
    #[inline]
    async fn write<T: IoBuf>(&mut self, buffer: T) -> BufResult<usize, T> {
        // The position is ignored.
        (&self.handle).write_at(buffer, 0).await
    }

    #[inline]
    async fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }

    #[inline]
    async fn shutdown(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl_try_as_raw_fd!(NamedPipeClient, handle);

impl_attachable!(NamedPipeClient, handle);

/// A builder structure for construct a named pipe with named pipe-specific
/// options. This is required to use for named pipe servers who wants to modify
/// pipe-related options.
///
/// See [`ServerOptions::create`].
#[derive(Debug, Clone)]
pub struct ServerOptions {
    // dwOpenMode
    access_inbound: bool,
    access_outbound: bool,
    first_pipe_instance: bool,
    write_dac: bool,
    write_owner: bool,
    access_system_security: bool,
    // dwPipeMode
    pipe_mode: PipeMode,
    reject_remote_clients: bool,
    // other options
    max_instances: u32,
    out_buffer_size: u32,
    in_buffer_size: u32,
    default_timeout: u32,
    security_attributes: *const SECURITY_ATTRIBUTES,
}

impl ServerOptions {
    /// Creates a new named pipe builder with the default settings.
    ///
    /// ```
    /// use compio_fs::named_pipe::ServerOptions;
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\compio-named-pipe-new";
    ///
    /// let server = ServerOptions::new().create(PIPE_NAME).unwrap();
    /// ```
    pub fn new() -> ServerOptions {
        ServerOptions {
            access_inbound: true,
            access_outbound: true,
            first_pipe_instance: false,
            write_dac: false,
            write_owner: false,
            access_system_security: false,
            pipe_mode: PipeMode::Byte,
            reject_remote_clients: true,
            max_instances: PIPE_UNLIMITED_INSTANCES,
            out_buffer_size: 65536,
            in_buffer_size: 65536,
            default_timeout: 0,
            security_attributes: null(),
        }
    }

    /// The pipe mode.
    ///
    /// The default pipe mode is [`PipeMode::Byte`]. See [`PipeMode`] for
    /// documentation of what each mode means.
    ///
    /// This corresponds to specifying `PIPE_TYPE_` and `PIPE_READMODE_` in
    /// [`dwPipeMode`].
    ///
    /// [`dwPipeMode`]: https://docs.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-createnamedpipea
    pub fn pipe_mode(&mut self, pipe_mode: PipeMode) -> &mut Self {
        self.pipe_mode = pipe_mode;
        self
    }

    /// The flow of data in the pipe goes from client to server only.
    ///
    /// This corresponds to setting [`PIPE_ACCESS_INBOUND`].
    ///
    /// [`PIPE_ACCESS_INBOUND`]: https://docs.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-createnamedpipea#pipe_access_inbound
    ///
    /// # Errors
    ///
    /// Server side prevents connecting by denying inbound access, client errors
    /// with [`std::io::ErrorKind::PermissionDenied`] when attempting to create
    /// the connection.
    ///
    /// ```
    /// use std::io;
    ///
    /// use compio_fs::named_pipe::{ClientOptions, ServerOptions};
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\compio-named-pipe-access-inbound-err1";
    ///
    /// # compio_runtime::Runtime::new().unwrap().block_on(async move {
    /// let _server = ServerOptions::new()
    ///     .access_inbound(false)
    ///     .create(PIPE_NAME)
    ///     .unwrap();
    ///
    /// let e = ClientOptions::new().open(PIPE_NAME).await.unwrap_err();
    ///
    /// assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
    /// # })
    /// ```
    ///
    /// Disabling writing allows a client to connect, but errors with
    /// [`std::io::ErrorKind::PermissionDenied`] if a write is attempted.
    ///
    /// ```
    /// use std::io;
    ///
    /// use compio_fs::named_pipe::{ClientOptions, ServerOptions};
    /// use compio_io::AsyncWrite;
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\compio-named-pipe-access-inbound-err2";
    ///
    /// # compio_runtime::Runtime::new().unwrap().block_on(async move {
    /// let server = ServerOptions::new()
    ///     .access_inbound(false)
    ///     .create(PIPE_NAME)
    ///     .unwrap();
    ///
    /// let mut client = ClientOptions::new()
    ///     .write(false)
    ///     .open(PIPE_NAME)
    ///     .await
    ///     .unwrap();
    ///
    /// server.connect().await.unwrap();
    ///
    /// let e = client.write("ping").await.0.unwrap_err();
    /// assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
    /// # })
    /// ```
    ///
    /// # Examples
    ///
    /// A unidirectional named pipe that only supports server-to-client
    /// communication.
    ///
    /// ```
    /// use std::io;
    ///
    /// use compio_buf::BufResult;
    /// use compio_fs::named_pipe::{ClientOptions, ServerOptions};
    /// use compio_io::{AsyncReadExt, AsyncWriteExt};
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\compio-named-pipe-access-inbound";
    ///
    /// # compio_runtime::Runtime::new().unwrap().block_on(async move {
    /// let mut server = ServerOptions::new()
    ///     .access_inbound(false)
    ///     .create(PIPE_NAME)
    ///     .unwrap();
    ///
    /// let mut client = ClientOptions::new()
    ///     .write(false)
    ///     .open(PIPE_NAME)
    ///     .await
    ///     .unwrap();
    ///
    /// server.connect().await.unwrap();
    ///
    /// let write = server.write_all("ping");
    ///
    /// let buf = Vec::with_capacity(4);
    /// let read = client.read_exact(buf);
    ///
    /// let (BufResult(write, _), BufResult(read, buf)) = futures_util::join!(write, read);
    /// write.unwrap();
    /// let read = read.unwrap();
    ///
    /// assert_eq!(read, 4);
    /// assert_eq!(&buf[..], b"ping");
    /// # })
    /// ```
    pub fn access_inbound(&mut self, allowed: bool) -> &mut Self {
        self.access_inbound = allowed;
        self
    }

    /// The flow of data in the pipe goes from server to client only.
    ///
    /// This corresponds to setting [`PIPE_ACCESS_OUTBOUND`].
    ///
    /// [`PIPE_ACCESS_OUTBOUND`]: https://docs.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-createnamedpipea#pipe_access_outbound
    ///
    /// # Errors
    ///
    /// Server side prevents connecting by denying outbound access, client
    /// errors with [`std::io::ErrorKind::PermissionDenied`] when attempting to
    /// create the connection.
    ///
    /// ```
    /// use std::io;
    ///
    /// use compio_fs::named_pipe::{ClientOptions, ServerOptions};
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\compio-named-pipe-access-outbound-err1";
    ///
    /// # compio_runtime::Runtime::new().unwrap().block_on(async move {
    /// let server = ServerOptions::new()
    ///     .access_outbound(false)
    ///     .create(PIPE_NAME)
    ///     .unwrap();
    ///
    /// let e = ClientOptions::new().open(PIPE_NAME).await.unwrap_err();
    ///
    /// assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
    /// # })
    /// ```
    ///
    /// Disabling reading allows a client to connect, but attempting to read
    /// will error with [`std::io::ErrorKind::PermissionDenied`].
    ///
    /// ```
    /// use std::io;
    ///
    /// use compio_fs::named_pipe::{ClientOptions, ServerOptions};
    /// use compio_io::AsyncRead;
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\compio-named-pipe-access-outbound-err2";
    ///
    /// # compio_runtime::Runtime::new().unwrap().block_on(async move {
    /// let server = ServerOptions::new()
    ///     .access_outbound(false)
    ///     .create(PIPE_NAME)
    ///     .unwrap();
    ///
    /// let mut client = ClientOptions::new()
    ///     .read(false)
    ///     .open(PIPE_NAME)
    ///     .await
    ///     .unwrap();
    ///
    /// server.connect().await.unwrap();
    ///
    /// let buf = Vec::with_capacity(4);
    /// let e = client.read(buf).await.0.unwrap_err();
    /// assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
    /// # })
    /// ```
    ///
    /// # Examples
    ///
    /// A unidirectional named pipe that only supports client-to-server
    /// communication.
    ///
    /// ```
    /// use compio_buf::BufResult;
    /// use compio_fs::named_pipe::{ClientOptions, ServerOptions};
    /// use compio_io::{AsyncReadExt, AsyncWriteExt};
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\compio-named-pipe-access-outbound";
    ///
    /// # compio_runtime::Runtime::new().unwrap().block_on(async move {
    /// let mut server = ServerOptions::new()
    ///     .access_outbound(false)
    ///     .create(PIPE_NAME)
    ///     .unwrap();
    ///
    /// let mut client = ClientOptions::new()
    ///     .read(false)
    ///     .open(PIPE_NAME)
    ///     .await
    ///     .unwrap();
    ///
    /// server.connect().await.unwrap();
    ///
    /// let write = client.write_all("ping");
    ///
    /// let buf = Vec::with_capacity(4);
    /// let read = server.read_exact(buf);
    ///
    /// let (BufResult(write, _), BufResult(read, buf)) = futures_util::join!(write, read);
    /// write.unwrap();
    /// let read = read.unwrap();
    ///
    /// println!("done reading and writing");
    ///
    /// assert_eq!(read, 4);
    /// assert_eq!(&buf[..], b"ping");
    /// # })
    /// ```
    pub fn access_outbound(&mut self, allowed: bool) -> &mut Self {
        self.access_outbound = allowed;
        self
    }

    /// If you attempt to create multiple instances of a pipe with this flag
    /// set, creation of the first server instance succeeds, but creation of any
    /// subsequent instances will fail with
    /// [`std::io::ErrorKind::PermissionDenied`].
    ///
    /// This option is intended to be used with servers that want to ensure that
    /// they are the only process listening for clients on a given named pipe.
    /// This is accomplished by enabling it for the first server instance
    /// created in a process.
    ///
    /// This corresponds to setting [`FILE_FLAG_FIRST_PIPE_INSTANCE`].
    ///
    /// # Errors
    ///
    /// If this option is set and more than one instance of the server for a
    /// given named pipe exists, calling [`create`] will fail with
    /// [`std::io::ErrorKind::PermissionDenied`].
    ///
    /// ```
    /// use std::io;
    ///
    /// use compio_fs::named_pipe::ServerOptions;
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\compio-named-pipe-first-instance-error";
    ///
    /// # compio_runtime::Runtime::new().unwrap().block_on(async move {
    /// let server1 = ServerOptions::new()
    ///     .first_pipe_instance(true)
    ///     .create(PIPE_NAME)
    ///     .unwrap();
    ///
    /// // Second server errs, since it's not the first instance.
    /// let e = ServerOptions::new()
    ///     .first_pipe_instance(true)
    ///     .create(PIPE_NAME)
    ///     .unwrap_err();
    ///
    /// assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
    /// # })
    /// ```
    ///
    /// # Examples
    ///
    /// ```
    /// use std::io;
    ///
    /// use compio_fs::named_pipe::ServerOptions;
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\compio-named-pipe-first-instance";
    ///
    /// # compio_runtime::Runtime::new().unwrap().block_on(async move {
    /// let mut builder = ServerOptions::new();
    /// builder.first_pipe_instance(true);
    ///
    /// let server = builder.create(PIPE_NAME).unwrap();
    /// let e = builder.create(PIPE_NAME).unwrap_err();
    /// assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
    /// drop(server);
    ///
    /// // OK: since, we've closed the other instance.
    /// let _server2 = builder.create(PIPE_NAME).unwrap();
    /// # })
    /// ```
    ///
    /// [`create`]: ServerOptions::create
    /// [`FILE_FLAG_FIRST_PIPE_INSTANCE`]: https://docs.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-createnamedpipea#pipe_first_pipe_instance
    pub fn first_pipe_instance(&mut self, first: bool) -> &mut Self {
        self.first_pipe_instance = first;
        self
    }

    /// Requests permission to modify the pipe's discretionary access control
    /// list.
    ///
    /// This corresponds to setting [`WRITE_DAC`] in dwOpenMode.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::{io, ptr};
    ///
    /// use compio_fs::named_pipe::ServerOptions;
    /// use compio_runtime::TryAsRawFd;
    /// use windows_sys::Win32::{
    ///     Foundation::ERROR_SUCCESS,
    ///     Security::{
    ///         Authorization::{SetSecurityInfo, SE_KERNEL_OBJECT},
    ///         DACL_SECURITY_INFORMATION,
    ///     },
    /// };
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\write_dac_pipe";
    ///
    /// # compio_runtime::Runtime::new().unwrap().block_on(async move {
    /// let mut pipe_template = ServerOptions::new();
    /// pipe_template.write_dac(true);
    /// let pipe = pipe_template.create(PIPE_NAME).unwrap();
    ///
    /// unsafe {
    ///     assert_eq!(
    ///         ERROR_SUCCESS,
    ///         SetSecurityInfo(
    ///             pipe.as_raw_fd_unchecked() as _,
    ///             SE_KERNEL_OBJECT,
    ///             DACL_SECURITY_INFORMATION,
    ///             ptr::null_mut(),
    ///             ptr::null_mut(),
    ///             ptr::null_mut(),
    ///             ptr::null_mut(),
    ///         )
    ///     );
    /// }
    ///
    /// # })
    /// ```
    /// ```
    /// use std::{io, ptr};
    ///
    /// use compio_fs::named_pipe::ServerOptions;
    /// use compio_runtime::TryAsRawFd;
    /// use windows_sys::Win32::{
    ///     Foundation::ERROR_ACCESS_DENIED,
    ///     Security::{
    ///         Authorization::{SetSecurityInfo, SE_KERNEL_OBJECT},
    ///         DACL_SECURITY_INFORMATION,
    ///     },
    /// };
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\write_dac_pipe_fail";
    ///
    /// # compio_runtime::Runtime::new().unwrap().block_on(async move {
    /// let mut pipe_template = ServerOptions::new();
    /// pipe_template.write_dac(false);
    /// let pipe = pipe_template.create(PIPE_NAME).unwrap();
    ///
    /// unsafe {
    ///     assert_eq!(
    ///         ERROR_ACCESS_DENIED,
    ///         SetSecurityInfo(
    ///             pipe.as_raw_fd_unchecked() as _,
    ///             SE_KERNEL_OBJECT,
    ///             DACL_SECURITY_INFORMATION,
    ///             ptr::null_mut(),
    ///             ptr::null_mut(),
    ///             ptr::null_mut(),
    ///             ptr::null_mut(),
    ///         )
    ///     );
    /// }
    ///
    /// # })
    /// ```
    ///
    /// [`WRITE_DAC`]: https://docs.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-createnamedpipea
    pub fn write_dac(&mut self, requested: bool) -> &mut Self {
        self.write_dac = requested;
        self
    }

    /// Requests permission to modify the pipe's owner.
    ///
    /// This corresponds to setting [`WRITE_OWNER`] in dwOpenMode.
    ///
    /// [`WRITE_OWNER`]: https://docs.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-createnamedpipea
    pub fn write_owner(&mut self, requested: bool) -> &mut Self {
        self.write_owner = requested;
        self
    }

    /// Requests permission to modify the pipe's system access control list.
    ///
    /// This corresponds to setting [`ACCESS_SYSTEM_SECURITY`] in dwOpenMode.
    ///
    /// [`ACCESS_SYSTEM_SECURITY`]: https://docs.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-createnamedpipea
    pub fn access_system_security(&mut self, requested: bool) -> &mut Self {
        self.access_system_security = requested;
        self
    }

    /// Indicates whether this server can accept remote clients or not. Remote
    /// clients are disabled by default.
    ///
    /// This corresponds to setting [`PIPE_REJECT_REMOTE_CLIENTS`].
    ///
    /// [`PIPE_REJECT_REMOTE_CLIENTS`]: https://docs.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-createnamedpipea#pipe_reject_remote_clients
    pub fn reject_remote_clients(&mut self, reject: bool) -> &mut Self {
        self.reject_remote_clients = reject;
        self
    }

    /// The maximum number of instances that can be created for this pipe. The
    /// first instance of the pipe can specify this value; the same number must
    /// be specified for other instances of the pipe. Acceptable values are in
    /// the range 1 through 254. The default value is unlimited.
    ///
    /// This corresponds to specifying [`nMaxInstances`].
    ///
    /// [`nMaxInstances`]: https://docs.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-createnamedpipea
    ///
    /// # Errors
    ///
    /// The same numbers of `max_instances` have to be used by all servers. Any
    /// additional servers trying to be built which uses a mismatching value
    /// might error.
    ///
    /// ```
    /// use std::io;
    ///
    /// use compio_fs::named_pipe::{ClientOptions, ServerOptions};
    /// use windows_sys::Win32::Foundation::ERROR_PIPE_BUSY;
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\compio-named-pipe-max-instances";
    ///
    /// # compio_runtime::Runtime::new().unwrap().block_on(async move {
    /// let mut server = ServerOptions::new();
    /// server.max_instances(2);
    ///
    /// let s1 = server.create(PIPE_NAME).unwrap();
    /// let c1 = ClientOptions::new().open(PIPE_NAME).await.unwrap();
    ///
    /// let s2 = server.create(PIPE_NAME).unwrap();
    /// let c2 = ClientOptions::new().open(PIPE_NAME).await.unwrap();
    ///
    /// // Too many servers!
    /// let e = server.create(PIPE_NAME).unwrap_err();
    /// assert_eq!(e.raw_os_error(), Some(ERROR_PIPE_BUSY as i32));
    ///
    /// // Still too many servers even if we specify a higher value!
    /// let e = server.max_instances(100).create(PIPE_NAME).unwrap_err();
    /// assert_eq!(e.raw_os_error(), Some(ERROR_PIPE_BUSY as i32));
    /// # })
    /// ```
    ///
    /// # Panics
    ///
    /// This function will panic if more than 254 instances are specified. If
    /// you do not wish to set an instance limit, leave it unspecified.
    ///
    /// ```should_panic
    /// use compio_fs::named_pipe::ServerOptions;
    ///
    /// let builder = ServerOptions::new().max_instances(255);
    /// ```
    #[track_caller]
    pub fn max_instances(&mut self, instances: usize) -> &mut Self {
        assert!(instances < 255, "cannot specify more than 254 instances");
        self.max_instances = instances as u32;
        self
    }

    /// The number of bytes to reserve for the output buffer.
    ///
    /// This corresponds to specifying [`nOutBufferSize`].
    ///
    /// [`nOutBufferSize`]: https://docs.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-createnamedpipea
    pub fn out_buffer_size(&mut self, buffer: u32) -> &mut Self {
        self.out_buffer_size = buffer;
        self
    }

    /// The number of bytes to reserve for the input buffer.
    ///
    /// This corresponds to specifying [`nInBufferSize`].
    ///
    /// [`nInBufferSize`]: https://docs.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-createnamedpipea
    pub fn in_buffer_size(&mut self, buffer: u32) -> &mut Self {
        self.in_buffer_size = buffer;
        self
    }

    /// Set the security attributes for the server handle.
    ///
    /// # Safety
    ///
    /// The `attrs` argument must either be null or point at a valid instance of
    /// the [`SECURITY_ATTRIBUTES`] structure.
    pub unsafe fn security_attributes(&mut self, attrs: *mut SECURITY_ATTRIBUTES) -> &mut Self {
        self.security_attributes = attrs;
        self
    }

    /// Creates the named pipe identified by `addr` for use as a server.
    ///
    /// This uses the [`CreateNamedPipe`] function.
    ///
    /// [`CreateNamedPipe`]: https://docs.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-createnamedpipea
    ///
    /// # Examples
    ///
    /// ```
    /// use compio_fs::named_pipe::ServerOptions;
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\compio-named-pipe-create";
    ///
    /// # compio_runtime::Runtime::new().unwrap().block_on(async move {
    /// let server = ServerOptions::new().create(PIPE_NAME).unwrap();
    /// # })
    /// ```
    pub fn create(&self, addr: impl AsRef<OsStr>) -> io::Result<NamedPipeServer> {
        let addr = U16CString::from_os_str(addr)
            .map_err(|e| io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        let pipe_mode = {
            let mut mode = if matches!(self.pipe_mode, PipeMode::Message) {
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE
            } else {
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE
            };
            if self.reject_remote_clients {
                mode |= PIPE_REJECT_REMOTE_CLIENTS;
            } else {
                mode |= PIPE_ACCEPT_REMOTE_CLIENTS;
            }
            mode
        };
        let open_mode = {
            let mut mode = FILE_FLAG_OVERLAPPED;
            if self.access_inbound {
                mode |= PIPE_ACCESS_INBOUND;
            }
            if self.access_outbound {
                mode |= PIPE_ACCESS_OUTBOUND;
            }
            if self.first_pipe_instance {
                mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
            }
            if self.write_dac {
                mode |= WRITE_DAC;
            }
            if self.write_owner {
                mode |= WRITE_OWNER;
            }
            if self.access_system_security {
                mode |= ACCESS_SYSTEM_SECURITY;
            }
            mode
        };

        let h = syscall!(
            HANDLE,
            CreateNamedPipeW(
                addr.as_ptr(),
                open_mode,
                pipe_mode,
                self.max_instances,
                self.out_buffer_size,
                self.in_buffer_size,
                self.default_timeout,
                self.security_attributes,
            )
        )?;

        Ok(unsafe { NamedPipeServer::from_raw_fd(h as _) })
    }
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// A builder suitable for building and interacting with named pipes from the
/// client side.
///
/// See [`ClientOptions::open`].
#[derive(Debug, Clone)]
pub struct ClientOptions {
    options: OpenOptions,
    pipe_mode: PipeMode,
}

impl ClientOptions {
    /// Creates a new named pipe builder with the default settings.
    ///
    /// ```
    /// use compio_fs::named_pipe::{ClientOptions, ServerOptions};
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\compio-named-pipe-client-new";
    ///
    /// # compio_runtime::Runtime::new().unwrap().block_on(async move {
    /// // Server must be created in order for the client creation to succeed.
    /// let server = ServerOptions::new().create(PIPE_NAME).unwrap();
    /// let client = ClientOptions::new().open(PIPE_NAME).await.unwrap();
    /// # })
    /// ```
    pub fn new() -> Self {
        use windows_sys::Win32::Storage::FileSystem::SECURITY_IDENTIFICATION;

        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(true)
            .security_qos_flags(SECURITY_IDENTIFICATION);
        Self {
            options,
            pipe_mode: PipeMode::Byte,
        }
    }

    /// If the client supports reading data. This is enabled by default.
    pub fn read(&mut self, allowed: bool) -> &mut Self {
        self.options.read(allowed);
        self
    }

    /// If the created pipe supports writing data. This is enabled by default.
    pub fn write(&mut self, allowed: bool) -> &mut Self {
        self.options.write(allowed);
        self
    }

    /// Sets qos flags which are combined with other flags and attributes in the
    /// call to [`CreateFile`].
    ///
    /// When `security_qos_flags` is not set, a malicious program can gain the
    /// elevated privileges of a privileged Rust process when it allows opening
    /// user-specified paths, by tricking it into opening a named pipe. So
    /// arguably `security_qos_flags` should also be set when opening arbitrary
    /// paths. However the bits can then conflict with other flags, specifically
    /// `FILE_FLAG_OPEN_NO_RECALL`.
    ///
    /// For information about possible values, see [Impersonation Levels] on the
    /// Windows Dev Center site. The `SECURITY_SQOS_PRESENT` flag is set
    /// automatically when using this method.
    ///
    /// [`CreateFile`]: https://docs.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilea
    /// [`SECURITY_IDENTIFICATION`]: https://docs.rs/windows-sys/latest/windows_sys/Win32/Storage/FileSystem/constant.SECURITY_IDENTIFICATION.html
    /// [Impersonation Levels]: https://docs.microsoft.com/en-us/windows/win32/api/winnt/ne-winnt-security_impersonation_level
    pub fn security_qos_flags(&mut self, flags: u32) -> &mut Self {
        self.options.security_qos_flags(flags);
        self
    }

    /// The pipe mode.
    ///
    /// The default pipe mode is [`PipeMode::Byte`]. See [`PipeMode`] for
    /// documentation of what each mode means.
    pub fn pipe_mode(&mut self, pipe_mode: PipeMode) -> &mut Self {
        self.pipe_mode = pipe_mode;
        self
    }

    /// Set the security attributes for the file handle.
    ///
    /// # Safety
    ///
    /// See [`OpenOptions::security_attributes`]
    pub unsafe fn security_attributes(&mut self, attrs: *mut SECURITY_ATTRIBUTES) -> &mut Self {
        self.options.security_attributes(attrs);
        self
    }

    /// Opens the named pipe identified by `addr`.
    ///
    /// This opens the client using [`CreateFile`] with the
    /// `dwCreationDisposition` option set to `OPEN_EXISTING`.
    ///
    /// [`CreateFile`]: https://docs.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilea
    ///
    /// # Errors
    ///
    /// There are a few errors you need to take into account when creating a
    /// named pipe on the client side:
    ///
    /// * [`std::io::ErrorKind::NotFound`] - This indicates that the named pipe
    ///   does not exist. Presumably the server is not up.
    /// * [`ERROR_PIPE_BUSY`] - This error is raised when the named pipe exists,
    ///   but the server is not currently waiting for a connection. Please see
    ///   the examples for how to check for this error.
    ///
    /// [`ERROR_PIPE_BUSY`]: https://docs.rs/windows-sys/latest/windows_sys/Win32/Foundation/constant.ERROR_PIPE_BUSY.html
    ///
    /// A connect loop that waits until a pipe becomes available looks like
    /// this:
    ///
    /// ```no_run
    /// use std::time::Duration;
    ///
    /// use compio_fs::named_pipe::ClientOptions;
    /// use compio_runtime::time;
    /// use windows_sys::Win32::Foundation::ERROR_PIPE_BUSY;
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\mynamedpipe";
    ///
    /// # compio_runtime::Runtime::new().unwrap().block_on(async move {
    /// let client = loop {
    ///     match ClientOptions::new().open(PIPE_NAME).await {
    ///         Ok(client) => break client,
    ///         Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => (),
    ///         Err(e) => return Err(e),
    ///     }
    ///
    ///     time::sleep(Duration::from_millis(50)).await;
    /// };
    ///
    /// // use the connected client.
    /// # Ok(()) });
    /// ```
    pub async fn open(&self, addr: impl AsRef<OsStr>) -> io::Result<NamedPipeClient> {
        use windows_sys::Win32::System::Pipes::SetNamedPipeHandleState;

        let file = self.options.open(addr.as_ref()).await?;

        if matches!(self.pipe_mode, PipeMode::Message) {
            let mode = PIPE_READMODE_MESSAGE;
            syscall!(
                BOOL,
                SetNamedPipeHandleState(file.as_raw_fd_unchecked() as _, &mode, null(), null())
            )?;
        }

        Ok(NamedPipeClient { handle: file })
    }
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// The pipe mode of a named pipe.
///
/// Set through [`ServerOptions::pipe_mode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PipeMode {
    /// Data is written to the pipe as a stream of bytes. The pipe does not
    /// distinguish bytes written during different write operations.
    ///
    /// Corresponds to [`PIPE_TYPE_BYTE`].
    ///
    /// [`PIPE_TYPE_BYTE`]: https://docs.rs/windows-sys/latest/windows_sys/Win32/System/Pipes/constant.PIPE_TYPE_BYTE.html
    Byte,
    /// Data is written to the pipe as a stream of messages. The pipe treats the
    /// bytes written during each write operation as a message unit. Any reading
    /// on a named pipe returns [`ERROR_MORE_DATA`] when a message is not read
    /// completely.
    ///
    /// Corresponds to [`PIPE_TYPE_MESSAGE`].
    ///
    /// [`ERROR_MORE_DATA`]: https://docs.rs/windows-sys/latest/windows_sys/Win32/Foundation/constant.ERROR_MORE_DATA.html
    /// [`PIPE_TYPE_MESSAGE`]: https://docs.rs/windows-sys/latest/windows_sys/Win32/System/Pipes/constant.PIPE_TYPE_MESSAGE.html
    Message,
}

/// Indicates the end of a named pipe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PipeEnd {
    /// The named pipe refers to the client end of a named pipe instance.
    ///
    /// Corresponds to [`PIPE_CLIENT_END`].
    ///
    /// [`PIPE_CLIENT_END`]: https://docs.rs/windows-sys/latest/windows_sys/Win32/System/Pipes/constant.PIPE_CLIENT_END.html
    Client,
    /// The named pipe refers to the server end of a named pipe instance.
    ///
    /// Corresponds to [`PIPE_SERVER_END`].
    ///
    /// [`PIPE_SERVER_END`]: https://docs.rs/windows-sys/latest/windows_sys/Win32/System/Pipes/constant.PIPE_SERVER_END.html
    Server,
}

/// Information about a named pipe.
///
/// Constructed through [`NamedPipeServer::info`] or [`NamedPipeClient::info`].
#[derive(Debug)]
pub struct PipeInfo {
    /// Indicates the mode of a named pipe.
    pub mode: PipeMode,
    /// Indicates the end of a named pipe.
    pub end: PipeEnd,
    /// The maximum number of instances that can be created for this pipe.
    pub max_instances: u32,
    /// The number of bytes to reserve for the output buffer.
    pub out_buffer_size: u32,
    /// The number of bytes to reserve for the input buffer.
    pub in_buffer_size: u32,
}

/// Internal function to get the info out of a raw named pipe.
unsafe fn named_pipe_info(handle: RawFd) -> io::Result<PipeInfo> {
    let mut flags = 0;
    let mut out_buffer_size = 0;
    let mut in_buffer_size = 0;
    let mut max_instances = 0;

    syscall!(
        BOOL,
        GetNamedPipeInfo(
            handle as _,
            &mut flags,
            &mut out_buffer_size,
            &mut in_buffer_size,
            &mut max_instances,
        )
    )?;

    let mut end = PipeEnd::Client;
    let mut mode = PipeMode::Byte;

    if flags & PIPE_SERVER_END != 0 {
        end = PipeEnd::Server;
    }

    if flags & PIPE_TYPE_MESSAGE != 0 {
        mode = PipeMode::Message;
    }

    Ok(PipeInfo {
        end,
        mode,
        out_buffer_size,
        in_buffer_size,
        max_instances,
    })
}
