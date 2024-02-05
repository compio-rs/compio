cfg_if::cfg_if! {
    if #[cfg(windows)] {
        mod windows;
        pub use windows::*;
    } else if #[cfg(unix)] {
        mod unix;
        pub use unix::*;
    }
}

/// Constructs a new handle to the standard input of the current process.
///
/// ## Platform specific
/// * Windows: This handle is best used for non-interactive uses, such as when a
///   file is piped into the application. For technical reasons, if `stdin` is a
///   console handle, the read method is implemented by using an ordinary
///   blocking read on a separate thread, and it is impossible to cancel that
///   read. This can make shutdown of the runtime hang until the user presses
///   enter.
///
/// [`AsyncRead`]: compio_io::AsyncRead
pub fn stdin() -> Stdin {
    Stdin::new()
}

/// Constructs a new handle to the standard output of the current process.
///
/// Concurrent writes to stdout must be executed with care: Only individual
/// writes to this [`AsyncWrite`] are guaranteed to be intact. In particular
/// you should be aware that writes using [`write_all`] are not guaranteed
/// to occur as a single write, so multiple threads writing data with
/// [`write_all`] may result in interleaved output.
///
/// [`AsyncWrite`]: compio_io::AsyncWrite
/// [`write_all`]: compio_io::AsyncWriteExt::write_all
pub fn stdout() -> Stdout {
    Stdout::new()
}

/// Constructs a new handle to the standard output of the current process.
///
/// Concurrent writes to stderr must be executed with care: Only individual
/// writes to this [`AsyncWrite`] are guaranteed to be intact. In particular
/// you should be aware that writes using [`write_all`] are not guaranteed
/// to occur as a single write, so multiple threads writing data with
/// [`write_all`] may result in interleaved output.
///
/// [`AsyncWrite`]: compio_io::AsyncWrite
/// [`write_all`]: compio_io::AsyncWriteExt::write_all
pub fn stderr() -> Stderr {
    Stderr::new()
}
