//! [Windows named pipes](https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipes).
//!
//! The infrastructure of the code comes from tokio.

use std::{
    ffi::{c_void, OsStr},
    io,
    os::windows::prelude::{AsRawHandle, HandleOrInvalid, IntoRawHandle, OwnedHandle},
    ptr::null_mut,
};

use widestring::U16CString;
use windows_sys::Win32::{
    Foundation::{GENERIC_READ, GENERIC_WRITE},
    Storage::FileSystem::{
        CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, OPEN_EXISTING,
        PIPE_ACCESS_INBOUND, PIPE_ACCESS_OUTBOUND, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT,
        WRITE_DAC, WRITE_OWNER,
    },
    System::{
        Pipes::{
            CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeInfo, SetNamedPipeHandleState,
            PIPE_ACCEPT_REMOTE_CLIENTS, PIPE_READMODE_BYTE, PIPE_READMODE_MESSAGE,
            PIPE_REJECT_REMOTE_CLIENTS, PIPE_SERVER_END, PIPE_TYPE_BYTE, PIPE_TYPE_MESSAGE,
            PIPE_UNLIMITED_INSTANCES,
        },
        SystemServices::ACCESS_SYSTEM_SECURITY,
    },
};

#[cfg(feature = "runtime")]
use crate::{buf::*, op::ConnectNamedPipe, task::RUNTIME, *};
use crate::{
    driver::{AsRawFd, FromRawFd, IntoRawFd, RawFd},
    fs::File,
};

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
/// use compio::named_pipe::ServerOptions;
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
/// let server = compio::task::block_on(async move {
///     loop {
///         // Wait for a client to connect.
///         let connected = server.connect().await?;
///
///         // Construct the next server to be connected before sending the one
///         // we already have of onto a task. This ensures that the server
///         // isn't closed (after it's done in the task) before a new one is
///         // available. Otherwise the client might error with
///         // `io::ErrorKind::NotFound`.
///         server = ServerOptions::new().create(PIPE_NAME)?;
///
///         let client = compio::task::spawn(async move {
///             // use the connected client
/// #           Ok::<_, std::io::Error>(())
///         });
/// #       if true { break } // needed for type inference to work
///     }
///
///     Ok::<_, io::Error>(())
/// });
///
/// // do something else not server related here
/// # Ok(()) }
/// ```
///
/// [Windows named pipe]: https://docs.microsoft.com/en-us/windows/win32/ipc/named-pipes
#[derive(Debug)]
pub struct NamedPipeServer {
    handle: File,
}

impl NamedPipeServer {
    pub(crate) fn from_handle(handle: OwnedHandle) -> io::Result<Self> {
        #[cfg(feature = "runtime")]
        RUNTIME.with(|runtime| runtime.attach(handle.as_raw_handle() as _))?;
        Ok(unsafe { Self::from_raw_fd(handle.into_raw_handle()) })
    }

    /// Retrieves information about the named pipe the server is associated
    /// with.
    ///
    /// ```no_run
    /// use compio::named_pipe::{PipeEnd, PipeMode, ServerOptions};
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\tokio-named-pipe-server-info";
    ///
    /// # compio::task::block_on(async move {
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
        unsafe { named_pipe_info(self.as_raw_fd()) }
    }

    /// Enables a named pipe server process to wait for a client process to
    /// connect to an instance of a named pipe. A client process connects by
    /// creating a named pipe with the same name.
    ///
    /// This corresponds to the [`ConnectNamedPipe`] system call.
    ///
    /// # Cancel safety
    ///
    /// This method is cancellation safe in the sense that if it is used as the
    /// event in a [`select!`](futures_util::select) statement and some other
    /// branch completes first, then no connection events have been lost.
    ///
    /// [`ConnectNamedPipe`]: https://docs.microsoft.com/en-us/windows/win32/api/namedpipeapi/nf-namedpipeapi-connectnamedpipe
    ///
    /// # Example
    ///
    /// ```no_run
    /// use compio::named_pipe::ServerOptions;
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\mynamedpipe";
    ///
    /// # compio::task::block_on(async move {
    /// let pipe = ServerOptions::new().create(PIPE_NAME)?;
    ///
    /// // Wait for a client to connect.
    /// pipe.connect().await?;
    ///
    /// // Use the connected client...
    /// # std::io::Result::Ok(()) });
    /// ```
    #[cfg(feature = "runtime")]
    pub async fn connect(&self) -> io::Result<()> {
        let op = ConnectNamedPipe::new(self.as_raw_fd());
        RUNTIME.with(|runtime| runtime.submit(op)).await.0?;
        Ok(())
    }

    /// Disconnects the server end of a named pipe instance from a client
    /// process.
    ///
    /// ```
    /// use compio::named_pipe::{ClientOptions, ServerOptions};
    /// use windows_sys::Win32::Foundation::ERROR_PIPE_NOT_CONNECTED;
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\tokio-named-pipe-disconnect";
    ///
    /// # compio::task::block_on(async move {
    /// let server = ServerOptions::new().create(PIPE_NAME).unwrap();
    ///
    /// let mut client = ClientOptions::new().open(PIPE_NAME).unwrap();
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
        let res = unsafe { DisconnectNamedPipe(self.as_raw_fd() as _) };
        if res == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    /// Read some bytes from the pipe into the specified
    /// buffer, returning how many bytes were read.
    #[cfg(feature = "runtime")]
    pub async fn read<T: IoBufMut>(&self, buffer: T) -> BufResult<usize, T> {
        self.handle.read_at(buffer, 0).await
    }

    /// Write a buffer into the pipe, returning how many bytes were written.
    #[cfg(feature = "runtime")]
    pub async fn write<T: IoBuf>(&self, buffer: T) -> BufResult<usize, T> {
        self.handle.write_at(buffer, 0).await
    }
}

impl AsRawFd for NamedPipeServer {
    fn as_raw_fd(&self) -> RawFd {
        self.handle.as_raw_fd()
    }
}

impl FromRawFd for NamedPipeServer {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self {
            handle: File::from_raw_fd(fd),
        }
    }
}

impl IntoRawFd for NamedPipeServer {
    fn into_raw_fd(self) -> RawFd {
        self.handle.into_raw_fd()
    }
}

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
/// use compio::{named_pipe::ClientOptions, time};
/// use windows_sys::Win32::Foundation::ERROR_PIPE_BUSY;
///
/// const PIPE_NAME: &str = r"\\.\pipe\named-pipe-idiomatic-client";
///
/// # compio::task::block_on(async move {
/// let client = loop {
///     match ClientOptions::new().open(PIPE_NAME) {
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
    pub(crate) fn from_handle(handle: OwnedHandle) -> io::Result<Self> {
        #[cfg(feature = "runtime")]
        RUNTIME.with(|runtime| runtime.attach(handle.as_raw_handle() as _))?;
        Ok(unsafe { Self::from_raw_fd(handle.into_raw_handle()) })
    }

    /// Retrieves information about the named pipe the client is associated
    /// with.
    ///
    /// ```no_run
    /// use compio::named_pipe::{ClientOptions, PipeEnd, PipeMode};
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\tokio-named-pipe-client-info";
    ///
    /// # compio::task::block_on(async move {
    /// let client = ClientOptions::new().open(PIPE_NAME)?;
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
        unsafe { named_pipe_info(self.as_raw_fd()) }
    }

    /// Read some bytes from the pipe into the specified
    /// buffer, returning how many bytes were read.
    #[cfg(feature = "runtime")]
    pub async fn read<T: IoBufMut>(&self, buffer: T) -> BufResult<usize, T> {
        self.handle.read_at(buffer, 0).await
    }

    /// Write a buffer into the pipe, returning how many bytes were written.
    #[cfg(feature = "runtime")]
    pub async fn write<T: IoBuf>(&self, buffer: T) -> BufResult<usize, T> {
        self.handle.write_at(buffer, 0).await
    }
}

impl AsRawFd for NamedPipeClient {
    fn as_raw_fd(&self) -> RawFd {
        self.handle.as_raw_fd()
    }
}

impl FromRawFd for NamedPipeClient {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Self {
            handle: File::from_raw_fd(fd),
        }
    }
}

impl IntoRawFd for NamedPipeClient {
    fn into_raw_fd(self) -> RawFd {
        self.handle.into_raw_fd()
    }
}

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
}

impl ServerOptions {
    /// Creates a new named pipe builder with the default settings.
    ///
    /// ```
    /// use compio::named_pipe::ServerOptions;
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\tokio-named-pipe-new";
    ///
    /// # compio::task::block_on(async move {
    /// let server = ServerOptions::new().create(PIPE_NAME).unwrap();
    /// # })
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
    /// use compio::named_pipe::{ClientOptions, ServerOptions};
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\tokio-named-pipe-access-inbound-err1";
    ///
    /// # compio::task::block_on(async move {
    /// let _server = ServerOptions::new()
    ///     .access_inbound(false)
    ///     .create(PIPE_NAME)
    ///     .unwrap();
    ///
    /// let e = ClientOptions::new().open(PIPE_NAME).unwrap_err();
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
    /// use compio::named_pipe::{ClientOptions, ServerOptions};
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\tokio-named-pipe-access-inbound-err2";
    ///
    /// # compio::task::block_on(async move {
    /// let server = ServerOptions::new()
    ///     .access_inbound(false)
    ///     .create(PIPE_NAME)
    ///     .unwrap();
    ///
    /// let mut client = ClientOptions::new().write(false).open(PIPE_NAME).unwrap();
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
    /// use compio::named_pipe::{ClientOptions, ServerOptions};
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\tokio-named-pipe-access-inbound";
    ///
    /// # compio::task::block_on(async move {
    /// let mut server = ServerOptions::new()
    ///     .access_inbound(false)
    ///     .create(PIPE_NAME)
    ///     .unwrap();
    ///
    /// let mut client = ClientOptions::new().write(false).open(PIPE_NAME).unwrap();
    ///
    /// server.connect().await.unwrap();
    ///
    /// let write = server.write("ping");
    ///
    /// let mut buf = Vec::with_capacity(4);
    /// let read = client.read(buf);
    ///
    /// let ((write, _), (read, buf)) = futures_util::join!(write, read);
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
    /// use compio::named_pipe::{ClientOptions, ServerOptions};
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\tokio-named-pipe-access-outbound-err1";
    ///
    /// # compio::task::block_on(async move {
    /// let server = ServerOptions::new()
    ///     .access_outbound(false)
    ///     .create(PIPE_NAME)
    ///     .unwrap();
    ///
    /// let e = ClientOptions::new().open(PIPE_NAME).unwrap_err();
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
    /// use compio::named_pipe::{ClientOptions, ServerOptions};
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\tokio-named-pipe-access-outbound-err2";
    ///
    /// # compio::task::block_on(async move {
    /// let server = ServerOptions::new()
    ///     .access_outbound(false)
    ///     .create(PIPE_NAME)
    ///     .unwrap();
    ///
    /// let mut client = ClientOptions::new().read(false).open(PIPE_NAME).unwrap();
    ///
    /// server.connect().await.unwrap();
    ///
    /// let mut buf = Vec::with_capacity(4);
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
    /// use compio::named_pipe::{ClientOptions, ServerOptions};
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\tokio-named-pipe-access-outbound";
    ///
    /// # compio::task::block_on(async move {
    /// let mut server = ServerOptions::new()
    ///     .access_outbound(false)
    ///     .create(PIPE_NAME)
    ///     .unwrap();
    ///
    /// let mut client = ClientOptions::new().read(false).open(PIPE_NAME).unwrap();
    ///
    /// server.connect().await.unwrap();
    ///
    /// let write = client.write("ping");
    ///
    /// let mut buf = Vec::with_capacity(4);
    /// let read = server.read(buf);
    ///
    /// let ((write, _), (read, buf)) = futures_util::join!(write, read);
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
    /// use compio::named_pipe::ServerOptions;
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\tokio-named-pipe-first-instance-error";
    ///
    /// # compio::task::block_on(async move {
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
    /// use compio::named_pipe::ServerOptions;
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\tokio-named-pipe-first-instance";
    ///
    /// # compio::task::block_on(async move {
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
    //
    /// use compio::{driver::AsRawFd, named_pipe::ServerOptions};
    /// use windows_sys::{
    ///     Win32::Foundation::ERROR_SUCCESS,
    ///     Win32::Security::DACL_SECURITY_INFORMATION,
    ///     Win32::Security::Authorization::{SetSecurityInfo, SE_KERNEL_OBJECT},
    /// };
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\write_dac_pipe";
    ///
    /// # compio::task::block_on(async move {
    /// let mut pipe_template = ServerOptions::new();
    /// pipe_template.write_dac(true);
    /// let pipe = pipe_template.create(PIPE_NAME).unwrap();
    ///
    /// unsafe {
    ///     assert_eq!(
    ///         ERROR_SUCCESS,
    ///         SetSecurityInfo(
    ///             pipe.as_raw_fd() as _,
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
    //
    /// use compio::{driver::AsRawFd, named_pipe::ServerOptions};
    /// use windows_sys::{
    ///     Win32::Foundation::ERROR_ACCESS_DENIED,
    ///     Win32::Security::DACL_SECURITY_INFORMATION,
    ///     Win32::Security::Authorization::{SetSecurityInfo, SE_KERNEL_OBJECT},
    /// };
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\write_dac_pipe_fail";
    ///
    /// # compio::task::block_on(async move {
    /// let mut pipe_template = ServerOptions::new();
    /// pipe_template.write_dac(false);
    /// let pipe = pipe_template.create(PIPE_NAME).unwrap();
    ///
    /// unsafe {
    ///     assert_eq!(
    ///         ERROR_ACCESS_DENIED,
    ///         SetSecurityInfo(
    ///             pipe.as_raw_fd() as _,
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
    /// use compio::named_pipe::{ClientOptions, ServerOptions};
    /// use windows_sys::Win32::Foundation::ERROR_PIPE_BUSY;
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\tokio-named-pipe-max-instances";
    ///
    /// # compio::task::block_on(async move {
    /// let mut server = ServerOptions::new();
    /// server.max_instances(2);
    ///
    /// let s1 = server.create(PIPE_NAME).unwrap();
    /// let c1 = ClientOptions::new().open(PIPE_NAME);
    ///
    /// let s2 = server.create(PIPE_NAME).unwrap();
    /// let c2 = ClientOptions::new().open(PIPE_NAME);
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
    /// use compio::named_pipe::ServerOptions;
    ///
    /// # compio::task::block_on(async move {
    /// let builder = ServerOptions::new().max_instances(255);
    /// # })
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

    /// Creates the named pipe identified by `addr` for use as a server.
    ///
    /// This uses the [`CreateNamedPipe`] function.
    ///
    /// [`CreateNamedPipe`]: https://docs.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-createnamedpipea
    ///
    /// # Examples
    ///
    /// ```
    /// use compio::named_pipe::ServerOptions;
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\tokio-named-pipe-create";
    ///
    /// # compio::task::block_on(async move {
    /// let server = ServerOptions::new().create(PIPE_NAME).unwrap();
    /// # })
    /// ```
    pub fn create(&self, addr: impl AsRef<OsStr>) -> io::Result<NamedPipeServer> {
        // Safety: We're calling create_with_security_attributes_raw w/ a null
        // pointer which disables it.
        unsafe { self.create_with_security_attributes_raw(addr, null_mut()) }
    }

    /// Creates the named pipe identified by `addr` for use as a server.
    ///
    /// This is the same as [`create`] except that it supports providing the raw
    /// pointer to a structure of [`SECURITY_ATTRIBUTES`] which will be passed
    /// as the `lpSecurityAttributes` argument to [`CreateFile`].
    ///
    /// # Safety
    ///
    /// The `attrs` argument must either be null or point at a valid instance of
    /// the [`SECURITY_ATTRIBUTES`] structure. If the argument is null, the
    /// behavior is identical to calling the [`create`] method.
    ///
    /// [`create`]: ServerOptions::create
    /// [`CreateFile`]: https://docs.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew
    /// [`SECURITY_ATTRIBUTES`]: https://docs.rs/windows-sys/latest/windows_sys/Win32/Security/struct.SECURITY_ATTRIBUTES.html
    pub unsafe fn create_with_security_attributes_raw(
        &self,
        addr: impl AsRef<OsStr>,
        attrs: *mut c_void,
    ) -> io::Result<NamedPipeServer> {
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

        let h = CreateNamedPipeW(
            addr.as_ptr(),
            open_mode,
            pipe_mode,
            self.max_instances,
            self.out_buffer_size,
            self.in_buffer_size,
            self.default_timeout,
            attrs as *mut _,
        );

        let h = OwnedHandle::try_from(HandleOrInvalid::from_raw_handle(h as _))
            .map_err(|_| io::Error::last_os_error())?;

        NamedPipeServer::from_handle(h)
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
    generic_read: bool,
    generic_write: bool,
    security_qos_flags: u32,
    pipe_mode: PipeMode,
}

impl ClientOptions {
    /// Creates a new named pipe builder with the default settings.
    ///
    /// ```
    /// use compio::named_pipe::{ClientOptions, ServerOptions};
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\tokio-named-pipe-client-new";
    ///
    /// # compio::task::block_on(async move {
    /// // Server must be created in order for the client creation to succeed.
    /// let server = ServerOptions::new().create(PIPE_NAME).unwrap();
    /// let client = ClientOptions::new().open(PIPE_NAME).unwrap();
    /// # })
    /// ```
    pub fn new() -> Self {
        Self {
            generic_read: true,
            generic_write: true,
            security_qos_flags: SECURITY_IDENTIFICATION | SECURITY_SQOS_PRESENT,
            pipe_mode: PipeMode::Byte,
        }
    }

    /// If the client supports reading data. This is enabled by default.
    ///
    /// This corresponds to setting [`GENERIC_READ`] in the call to
    /// [`CreateFile`].
    ///
    /// [`GENERIC_READ`]: https://docs.microsoft.com/en-us/windows/win32/secauthz/generic-access-rights
    /// [`CreateFile`]: https://docs.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew
    pub fn read(&mut self, allowed: bool) -> &mut Self {
        self.generic_read = allowed;
        self
    }

    /// If the created pipe supports writing data. This is enabled by default.
    ///
    /// This corresponds to setting [`GENERIC_WRITE`] in the call to
    /// [`CreateFile`].
    ///
    /// [`GENERIC_WRITE`]: https://docs.microsoft.com/en-us/windows/win32/secauthz/generic-access-rights
    /// [`CreateFile`]: https://docs.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew
    pub fn write(&mut self, allowed: bool) -> &mut Self {
        self.generic_write = allowed;
        self
    }

    /// Sets qos flags which are combined with other flags and attributes in the
    /// call to [`CreateFile`].
    ///
    /// By default `security_qos_flags` is set to [`SECURITY_IDENTIFICATION`],
    /// calling this function would override that value completely with the
    /// argument specified.
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
        // See: https://github.com/rust-lang/rust/pull/58216
        self.security_qos_flags = flags | SECURITY_SQOS_PRESENT;
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
    /// use compio::{named_pipe::ClientOptions, time};
    /// use windows_sys::Win32::Foundation::ERROR_PIPE_BUSY;
    ///
    /// const PIPE_NAME: &str = r"\\.\pipe\mynamedpipe";
    ///
    /// # compio::task::block_on(async move {
    /// let client = loop {
    ///     match ClientOptions::new().open(PIPE_NAME) {
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
    pub fn open(&self, addr: impl AsRef<OsStr>) -> io::Result<NamedPipeClient> {
        // Safety: We're calling open_with_security_attributes_raw w/ a null
        // pointer which disables it.
        unsafe { self.open_with_security_attributes_raw(addr, null_mut()) }
    }

    /// Opens the named pipe identified by `addr`.
    ///
    /// This is the same as [`open`] except that it supports providing the raw
    /// pointer to a structure of [`SECURITY_ATTRIBUTES`] which will be passed
    /// as the `lpSecurityAttributes` argument to [`CreateFile`].
    ///
    /// # Safety
    ///
    /// The `attrs` argument must either be null or point at a valid instance of
    /// the [`SECURITY_ATTRIBUTES`] structure. If the argument is null, the
    /// behavior is identical to calling the [`open`] method.
    ///
    /// [`open`]: ClientOptions::open
    /// [`CreateFile`]: https://docs.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew
    /// [`SECURITY_ATTRIBUTES`]: https://docs.rs/windows-sys/latest/windows_sys/Win32/Security/struct.SECURITY_ATTRIBUTES.html
    pub unsafe fn open_with_security_attributes_raw(
        &self,
        addr: impl AsRef<OsStr>,
        attrs: *mut c_void,
    ) -> io::Result<NamedPipeClient> {
        let addr = U16CString::from_os_str(addr)
            .map_err(|e| io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        let desired_access = {
            let mut access = 0;
            if self.generic_read {
                access |= GENERIC_READ;
            }
            if self.generic_write {
                access |= GENERIC_WRITE;
            }
            access
        };

        // NB: We could use a platform specialized `OpenOptions` here, but since
        // we have access to windows_sys it ultimately doesn't hurt to use
        // `CreateFile` explicitly since it allows the use of our already
        // well-structured wide `addr` to pass into CreateFileW.
        let h = CreateFileW(
            addr.as_ptr(),
            desired_access,
            0,
            attrs as *mut _,
            OPEN_EXISTING,
            self.get_flags(),
            0,
        );

        let h = OwnedHandle::try_from(HandleOrInvalid::from_raw_handle(h as _))
            .map_err(|_| io::Error::last_os_error())?;

        if matches!(self.pipe_mode, PipeMode::Message) {
            let mode = PIPE_READMODE_MESSAGE;
            let result =
                SetNamedPipeHandleState(h.as_raw_handle() as _, &mode, null_mut(), null_mut());

            if result == 0 {
                return Err(io::Error::last_os_error());
            }
        }

        NamedPipeClient::from_handle(h)
    }

    fn get_flags(&self) -> u32 {
        self.security_qos_flags | FILE_FLAG_OVERLAPPED
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

    let result = GetNamedPipeInfo(
        handle as _,
        &mut flags,
        &mut out_buffer_size,
        &mut in_buffer_size,
        &mut max_instances,
    );

    if result == 0 {
        return Err(io::Error::last_os_error());
    }

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
