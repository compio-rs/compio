//! Unix pipe types.

use std::{io, os::unix::fs::FileTypeExt, path::Path};

use compio_driver::{impl_raw_fd, syscall, AsRawFd, FromRawFd, IntoRawFd};
#[cfg(feature = "runtime")]
use {
    compio_buf::{buf_try, BufResult, IntoInner, IoBuf, IoBufMut},
    compio_driver::op::{BufResultExt, Recv, RecvVectored, Send, SendVectored},
    compio_io::{AsyncRead, AsyncWrite},
    compio_runtime::{impl_attachable, submit, Attachable},
};

use crate::File;

/// Creates a pair of anonymous pipe.
///
/// ```
/// use compio_fs::pipe::anonymous;
/// use compio_io::{AsyncReadExt, AsyncWriteExt};
///
/// # compio_runtime::block_on(async {
/// let (mut rx, mut tx) = anonymous().unwrap();
///
/// tx.write_all("Hello world!").await.unwrap();
/// let (_, buf) = rx.read_exact(Vec::with_capacity(12)).await.unwrap();
/// assert_eq!(&buf, b"Hello world!");
/// # });
/// ```
pub fn anonymous() -> io::Result<(Receiver, Sender)> {
    let (receiver, sender) = os_pipe::pipe()?;
    let receiver = Receiver::from_file(unsafe { File::from_raw_fd(receiver.into_raw_fd()) })?;
    let sender = Sender::from_file(unsafe { File::from_raw_fd(sender.into_raw_fd()) })?;
    Ok((receiver, sender))
}

/// Options and flags which can be used to configure how a FIFO file is opened.
///
/// This builder allows configuring how to create a pipe end from a FIFO file.
/// Generally speaking, when using `OpenOptions`, you'll first call [`new`],
/// then chain calls to methods to set each option, then call either
/// [`open_receiver`] or [`open_sender`], passing the path of the FIFO file you
/// are trying to open. This will give you a [`io::Result`] with a pipe end
/// inside that you can further operate on.
///
/// [`new`]: OpenOptions::new
/// [`open_receiver`]: OpenOptions::open_receiver
/// [`open_sender`]: OpenOptions::open_sender
///
/// # Examples
///
/// Opening a pair of pipe ends from a FIFO file:
///
/// ```no_run
/// use compio_fs::pipe;
///
/// const FIFO_NAME: &str = "path/to/a/fifo";
///
/// # async fn dox() -> std::io::Result<()> {
/// let rx = pipe::OpenOptions::new().open_receiver(FIFO_NAME)?;
/// let tx = pipe::OpenOptions::new().open_sender(FIFO_NAME)?;
/// # Ok(())
/// # }
/// ```
///
/// Opening a [`Sender`] on Linux when you are sure the file is a FIFO:
///
/// ```ignore
/// use compio_fs::pipe;
/// use nix::{sys::stat::Mode, unistd::mkfifo};
///
/// // Our program has exclusive access to this path.
/// const FIFO_NAME: &str = "path/to/a/new/fifo";
///
/// # async fn dox() -> std::io::Result<()> {
/// mkfifo(FIFO_NAME, Mode::S_IRWXU)?;
/// let tx = pipe::OpenOptions::new()
///     .read_write(true)
///     .unchecked(true)
///     .open_sender(FIFO_NAME)?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct OpenOptions {
    #[cfg(target_os = "linux")]
    read_write: bool,
    unchecked: bool,
}

impl OpenOptions {
    /// Creates a blank new set of options ready for configuration.
    ///
    /// All options are initially set to `false`.
    pub fn new() -> OpenOptions {
        OpenOptions {
            #[cfg(target_os = "linux")]
            read_write: false,
            unchecked: false,
        }
    }

    /// Sets the option for read-write access.
    ///
    /// This option, when true, will indicate that a FIFO file will be opened
    /// in read-write access mode. This operation is not defined by the POSIX
    /// standard and is only guaranteed to work on Linux.
    ///
    /// # Examples
    ///
    /// Opening a [`Sender`] even if there are no open reading ends:
    ///
    /// ```
    /// use compio_fs::pipe;
    ///
    /// let tx = pipe::OpenOptions::new()
    ///     .read_write(true)
    ///     .open_sender("path/to/a/fifo");
    /// ```
    ///
    /// Opening a resilient [`Receiver`] i.e. a reading pipe end which will not
    /// fail with [`UnexpectedEof`] during reading if all writing ends of the
    /// pipe close the FIFO file.
    ///
    /// [`UnexpectedEof`]: std::io::ErrorKind::UnexpectedEof
    ///
    /// ```
    /// use compio_fs::pipe;
    ///
    /// let tx = pipe::OpenOptions::new()
    ///     .read_write(true)
    ///     .open_receiver("path/to/a/fifo");
    /// ```
    #[cfg(target_os = "linux")]
    #[cfg_attr(docsrs, doc(cfg(target_os = "linux")))]
    pub fn read_write(&mut self, value: bool) -> &mut Self {
        self.read_write = value;
        self
    }

    /// Sets the option to skip the check for FIFO file type.
    ///
    /// By default, [`open_receiver`] and [`open_sender`] functions will check
    /// if the opened file is a FIFO file. Set this option to `true` if you are
    /// sure the file is a FIFO file.
    ///
    /// [`open_receiver`]: OpenOptions::open_receiver
    /// [`open_sender`]: OpenOptions::open_sender
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use compio_fs::pipe;
    /// use nix::{sys::stat::Mode, unistd::mkfifo};
    ///
    /// // Our program has exclusive access to this path.
    /// const FIFO_NAME: &str = "path/to/a/new/fifo";
    ///
    /// # async fn dox() -> std::io::Result<()> {
    /// mkfifo(FIFO_NAME, Mode::S_IRWXU)?;
    /// let rx = pipe::OpenOptions::new()
    ///     .unchecked(true)
    ///     .open_receiver(FIFO_NAME)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn unchecked(&mut self, value: bool) -> &mut Self {
        self.unchecked = value;
        self
    }

    /// Creates a [`Receiver`] from a FIFO file with the options specified by
    /// `self`.
    ///
    /// This function will open the FIFO file at the specified path, possibly
    /// check if it is a pipe, and associate the pipe with the default event
    /// loop for reading.
    ///
    /// # Errors
    ///
    /// If the file type check fails, this function will fail with
    /// `io::ErrorKind::InvalidInput`. This function may also fail with
    /// other standard OS errors.
    pub fn open_receiver<P: AsRef<Path>>(&self, path: P) -> io::Result<Receiver> {
        let file = self.open(path.as_ref(), PipeEnd::Receiver)?;
        Receiver::from_file(file)
    }

    /// Creates a [`Sender`] from a FIFO file with the options specified by
    /// `self`.
    ///
    /// This function will open the FIFO file at the specified path, possibly
    /// check if it is a pipe, and associate the pipe with the default event
    /// loop for writing.
    ///
    /// # Errors
    ///
    /// If the file type check fails, this function will fail with
    /// `io::ErrorKind::InvalidInput`. If the file is not opened in
    /// read-write access mode and the file is not currently open for
    /// reading, this function will fail with `ENXIO`. This function may
    /// also fail with other standard OS errors.
    pub fn open_sender<P: AsRef<Path>>(&self, path: P) -> io::Result<Sender> {
        let file = self.open(path.as_ref(), PipeEnd::Sender)?;
        Sender::from_file(file)
    }

    fn open(&self, path: &Path, pipe_end: PipeEnd) -> io::Result<File> {
        let options = crate::OpenOptions::new()
            .read(pipe_end == PipeEnd::Receiver)
            .write(pipe_end == PipeEnd::Sender);

        #[cfg(target_os = "linux")]
        let options = if self.read_write {
            options.read(true).write(true)
        } else {
            options
        };

        let file = options.open(path)?;

        if !self.unchecked && !is_fifo(&file)? {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a pipe"));
        }

        Ok(file)
    }
}

impl Default for OpenOptions {
    fn default() -> OpenOptions {
        OpenOptions::new()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PipeEnd {
    Sender,
    Receiver,
}

/// Writing end of a Unix pipe.
///
/// It can be constructed from a FIFO file with [`OpenOptions::open_sender`].
///
/// Opening a named pipe for writing involves a few steps.
/// Call to [`OpenOptions::open_sender`] might fail with an error indicating
/// different things:
///
/// * [`io::ErrorKind::NotFound`] - There is no file at the specified path.
/// * [`io::ErrorKind::InvalidInput`] - The file exists, but it is not a FIFO.
/// * [`ENXIO`] - The file is a FIFO, but no process has it open for reading.
///   Sleep for a while and try again.
/// * Other OS errors not specific to opening FIFO files.
///
/// Opening a `Sender` from a FIFO file should look like this:
///
/// ```no_run
/// use std::time::Duration;
///
/// use compio_fs::pipe;
/// use compio_runtime::time;
///
/// const FIFO_NAME: &str = "path/to/a/fifo";
///
/// # async fn dox() -> std::io::Result<()> {
/// // Wait for a reader to open the file.
/// let tx = loop {
///     match pipe::OpenOptions::new().open_sender(FIFO_NAME) {
///         Ok(tx) => break tx,
///         Err(e) if e.raw_os_error() == Some(libc::ENXIO) => {}
///         Err(e) => return Err(e.into()),
///     }
///
///     time::sleep(Duration::from_millis(50)).await;
/// };
/// # Ok(())
/// # }
/// ```
///
/// On Linux, it is possible to create a `Sender` without waiting in a sleeping
/// loop. This is done by opening a named pipe in read-write access mode with
/// `OpenOptions::read_write`. This way, a `Sender` can at the same time hold
/// both a writing end and a reading end, and the latter allows to open a FIFO
/// without [`ENXIO`] error since the pipe is open for reading as well.
///
/// `Sender` cannot be used to read from a pipe, so in practice the read access
/// is only used when a FIFO is opened. However, using a `Sender` in read-write
/// mode **may lead to lost data**, because written data will be dropped by the
/// system as soon as all pipe ends are closed. To avoid lost data you have to
/// make sure that a reading end has been opened before dropping a `Sender`.
///
/// Note that using read-write access mode with FIFO files is not defined by
/// the POSIX standard and it is only guaranteed to work on Linux.
///
/// ```ignore
/// use compio_fs::pipe;
/// use compio_io::AsyncWriteExt;
///
/// const FIFO_NAME: &str = "path/to/a/fifo";
///
/// # async fn dox() {
/// let mut tx = pipe::OpenOptions::new()
///     .read_write(true)
///     .open_sender(FIFO_NAME)
///     .unwrap();
///
/// // Asynchronously write to the pipe before a reader.
/// tx.write_all("hello world").await.unwrap();
/// # }
/// ```
///
/// [`ENXIO`]: https://docs.rs/libc/latest/libc/constant.ENXIO.html
#[derive(Debug)]
pub struct Sender {
    file: File,
}

impl Sender {
    pub(crate) fn from_file(file: File) -> io::Result<Sender> {
        set_nonblocking(&file)?;
        Ok(Sender { file })
    }
}

#[cfg(feature = "runtime")]
impl AsyncWrite for Sender {
    async fn write<T: IoBuf>(&mut self, buffer: T) -> BufResult<usize, T> {
        let ((), buffer) = buf_try!(self.attach(), buffer);
        let op = Send::new(self.as_raw_fd(), buffer);
        submit(op).await.into_inner()
    }

    async fn write_vectored<T: compio_buf::IoVectoredBuf>(
        &mut self,
        buffer: T,
    ) -> BufResult<usize, T> {
        let ((), buffer) = buf_try!(self.attach(), buffer);
        let op = SendVectored::new(self.as_raw_fd(), buffer);
        submit(op).await.into_inner()
    }

    async fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }

    async fn shutdown(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl_raw_fd!(Sender, file);

#[cfg(feature = "runtime")]
impl_attachable!(Sender, file);

/// Reading end of a Unix pipe.
///
/// It can be constructed from a FIFO file with [`OpenOptions::open_receiver`].
///
/// # Examples
///
/// Receiving messages from a named pipe in a loop:
///
/// ```no_run
/// use std::io;
///
/// use compio_buf::BufResult;
/// use compio_fs::pipe;
/// use compio_io::AsyncReadExt;
///
/// const FIFO_NAME: &str = "path/to/a/fifo";
///
/// # async fn dox() -> io::Result<()> {
/// let mut rx = pipe::OpenOptions::new().open_receiver(FIFO_NAME)?;
/// loop {
///     let mut msg = Vec::with_capacity(256);
///     let BufResult(res, msg) = rx.read_exact(msg).await;
///     match res {
///         Ok(_) => { /* handle the message */ }
///         Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
///             // Writing end has been closed, we should reopen the pipe.
///             rx = pipe::OpenOptions::new().open_receiver(FIFO_NAME)?;
///         }
///         Err(e) => return Err(e.into()),
///     }
/// }
/// # }
/// ```
///
/// On Linux, you can use a `Receiver` in read-write access mode to implement
/// resilient reading from a named pipe. Unlike `Receiver` opened in read-only
/// mode, read from a pipe in read-write mode will not fail with `UnexpectedEof`
/// when the writing end is closed. This way, a `Receiver` can asynchronously
/// wait for the next writer to open the pipe.
///
/// You should not use functions waiting for EOF such as [`read_to_end`] with
/// a `Receiver` in read-write access mode, since it **may wait forever**.
/// `Receiver` in this mode also holds an open writing end, which prevents
/// receiving EOF.
///
/// To set the read-write access mode you can use `OpenOptions::read_write`.
/// Note that using read-write access mode with FIFO files is not defined by
/// the POSIX standard and it is only guaranteed to work on Linux.
///
/// ```ignore
/// use compio_fs::pipe;
/// use compio_io::AsyncReadExt;
///
/// const FIFO_NAME: &str = "path/to/a/fifo";
///
/// # async fn dox() {
/// let mut rx = pipe::OpenOptions::new()
///     .read_write(true)
///     .open_receiver(FIFO_NAME)
///     .unwrap();
/// loop {
///     let mut msg = Vec::with_capacity(256);
///     rx.read_exact(msg).await.unwrap();
///     // handle the message
/// }
/// # }
/// ```
///
/// [`read_to_end`]: crate::io::AsyncReadExt::read_to_end
#[derive(Debug)]
pub struct Receiver {
    file: File,
}

impl Receiver {
    pub(crate) fn from_file(file: File) -> io::Result<Receiver> {
        set_nonblocking(&file)?;
        Ok(Receiver { file })
    }
}

#[cfg(feature = "runtime")]
impl AsyncRead for Receiver {
    async fn read<B: IoBufMut>(&mut self, buffer: B) -> BufResult<usize, B> {
        let ((), buffer) = buf_try!(self.attach(), buffer);
        let op = Recv::new(self.as_raw_fd(), buffer);
        submit(op).await.into_inner().map_advanced()
    }

    async fn read_vectored<V: compio_buf::IoVectoredBufMut>(
        &mut self,
        buffer: V,
    ) -> BufResult<usize, V>
    where
        V: Unpin + 'static,
    {
        let ((), buffer) = buf_try!(self.attach(), buffer);
        let op = RecvVectored::new(self.as_raw_fd(), buffer);
        submit(op).await.into_inner().map_advanced()
    }
}

impl_raw_fd!(Receiver, file);

#[cfg(feature = "runtime")]
impl_attachable!(Receiver, file);

/// Checks if file is a FIFO
fn is_fifo(file: &File) -> io::Result<bool> {
    Ok(file.metadata()?.file_type().is_fifo())
}

/// Sets file's flags with O_NONBLOCK by fcntl.
fn set_nonblocking(file: &impl AsRawFd) -> io::Result<()> {
    if cfg!(not(all(target_os = "linux", feature = "io-uring"))) {
        let fd = file.as_raw_fd();
        let current_flags = syscall!(fcntl(fd, libc::F_GETFL))?;
        let flags = current_flags | libc::O_NONBLOCK;
        if flags != current_flags {
            syscall!(fcntl(fd, libc::F_SETFL, flags))?;
        }
    }
    Ok(())
}
