#[cfg(unix)]
#[path = "unix.rs"]
mod sys;

use std::{fs::OpenOptions as StdOpenOptions, io, path::Path};

use crate::File;

/// Options and flags which can be used to configure how a file is opened.
///
/// This builder exposes the ability to configure how a [`File`] is opened and
/// what operations are permitted on the open file. The [`File::open`] and
/// [`File::create`] methods are aliases for commonly used options using this
/// builder.
///
/// Generally speaking, when using `OpenOptions`, you'll first call
/// [`OpenOptions::new`], then chain calls to methods to set each option, then
/// call [`OpenOptions::open`], passing the path of the file you're trying to
/// open. This will give you a [`std::io::Result`] with a [`File`] inside that
/// you can further operate on.
///
/// # Examples
///
/// Opening a file to read:
///
/// ```no_run
/// use compio_fs::OpenOptions;
///
/// let file = OpenOptions::new().read(true).open("foo.txt").await.unwrap();
/// ```
///
/// Opening a file for both reading and writing, as well as creating it if it
/// doesn't exist:
///
/// ```no_run
/// use compio_fs::OpenOptions;
///
/// let file = OpenOptions::new()
///     .read(true)
///     .write(true)
///     .create(true)
///     .open("foo.txt")
///     .await
///     .unwrap();
/// ```
#[derive(Debug, Clone)]
pub struct OpenOptions(sys::OpenOptions);

impl OpenOptions {
    /// Creates a blank new set of options ready for configuration.
    #[allow(clippy::new_without_default)]
    #[must_use]
    pub fn new() -> Self {
        Self(sys::OpenOptions::new())
    }

    /// Sets the option for read access.
    ///
    /// This option, when true, will indicate that the file should be
    /// `read`-able if opened.
    pub fn read(mut self, read: bool) -> Self {
        self.0.read(read);
        self
    }

    /// Sets the option for write access.
    ///
    /// This option, when true, will indicate that the file should be
    /// `write`-able if opened.
    pub fn write(mut self, write: bool) -> Self {
        self.0.write(write);
        self
    }

    /// Sets the option for truncating a previous file.
    ///
    /// If a file is successfully opened with this option set it will truncate
    /// the file to 0 length if it already exists.
    ///
    /// The file must be opened with write access for truncate to work.
    pub fn truncate(mut self, truncate: bool) -> Self {
        self.0.truncate(truncate);
        self
    }

    /// Sets the option to create a new file, or open it if it already exists.
    ///
    /// In order for the file to be created, [`OpenOptions::write`] access must
    /// be used.
    pub fn create(mut self, create: bool) -> Self {
        self.0.create(create);
        self
    }

    /// Sets the option to create a new file, failing if it already exists.
    ///
    /// No file is allowed to exist at the target location, also no (dangling)
    /// symlink. In this way, if the call succeeds, the file returned is
    /// guaranteed to be new.
    ///
    /// This option is useful because it is atomic. Otherwise between checking
    /// whether a file exists and creating a new one, the file may have been
    /// created by another process (a TOCTOU race condition / attack).
    ///
    /// If `.create_new(true)` is set, [`.create()`] and [`.truncate()`] are
    /// ignored.
    ///
    /// The file must be opened with write or append access in order to create
    /// a new file.
    ///
    /// [`.create()`]: OpenOptions::create
    /// [`.truncate()`]: OpenOptions::truncate
    pub fn create_new(mut self, create_new: bool) -> Self {
        self.0.create_new(create_new);
        self
    }

    /// Pass custom flags to the `flags` argument of `open`.
    ///
    /// The bits that define the access mode are masked out with `O_ACCMODE`, to
    /// ensure they do not interfere with the access mode set by Rusts options.
    ///
    /// Custom flags can only set flags, not remove flags set by Rusts options.
    /// This options overwrites any previously set custom flags.
    #[cfg(unix)]
    pub fn custom_flags(mut self, flags: i32) -> Self {
        self.0.custom_flags(flags);
        self
    }

    /// Sets the mode bits that a new file will be created with.
    ///
    /// If a new file is created as part of an `OpenOptions::open` call then
    /// this specified `mode` will be used as the permission bits for the
    /// new file. If no `mode` is set, the default of `0o666` will be used.
    /// The operating system masks out bits with the system's `umask`, to
    /// produce the final permissions.
    #[cfg(unix)]
    pub fn mode(mut self, mode: u32) -> Self {
        self.0.mode(mode);
        self
    }

    /// Opens a file at `path` with the options specified by `self`.
    pub async fn open(self, path: impl AsRef<Path>) -> io::Result<File> {
        self.0.open(path).await
    }
}
