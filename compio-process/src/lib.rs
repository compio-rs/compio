//! Process library for compio. It is an extension to [`std::process`].

#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(
    all(feature = "linux_pidfd", target_os = "linux"),
    feature(linux_pidfd)
)]
#![warn(missing_docs)]

cfg_if::cfg_if! {
    if #[cfg(windows)] {
        #[path = "windows.rs"]
        mod sys;
    } else if #[cfg(target_os = "linux")] {
        #[path = "linux.rs"]
        mod sys;
    } else {
        #[path = "unix.rs"]
        mod sys;
    }
}

#[cfg(unix)]
use std::os::unix::process::CommandExt;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::{ffi::OsStr, io, path::Path, process};

use compio_buf::{BufResult, IntoInner};
use compio_driver::{AsFd, AsRawFd, BorrowedFd, RawFd, SharedFd, ToSharedFd};
use compio_io::AsyncReadExt;
use compio_runtime::Attacher;
use futures_util::future::Either;

/// A process builder, providing fine-grained control
/// over how a new process should be spawned.
///
/// A default configuration can be
/// generated using `Command::new(program)`, where `program` gives a path to the
/// program to be executed. Additional builder methods allow the configuration
/// to be changed (for example, by adding arguments) prior to spawning:
///
/// ```
/// use compio_process::Command;
///
/// # compio_runtime::Runtime::new().unwrap().block_on(async move {
/// let output = if cfg!(windows) {
///     Command::new("cmd")
///         .args(["/C", "echo hello"])
///         .output()
///         .await
///         .expect("failed to execute process")
/// } else {
///     Command::new("sh")
///         .args(["-c", "echo hello"])
///         .output()
///         .await
///         .expect("failed to execute process")
/// };
///
/// let hello = output.stdout;
/// # })
/// ```
///
/// `Command` can be reused to spawn multiple processes. The builder methods
/// change the command without needing to immediately spawn the process.
///
/// ```no_run
/// use compio_process::Command;
///
/// # compio_runtime::Runtime::new().unwrap().block_on(async move {
/// let mut echo_hello = Command::new("sh");
/// echo_hello.arg("-c").arg("echo hello");
/// let hello_1 = echo_hello
///     .output()
///     .await
///     .expect("failed to execute process");
/// let hello_2 = echo_hello
///     .output()
///     .await
///     .expect("failed to execute process");
/// # })
/// ```
///
/// Similarly, you can call builder methods after spawning a process and then
/// spawn a new process with the modified settings.
///
/// ```no_run
/// use compio_process::Command;
///
/// # compio_runtime::Runtime::new().unwrap().block_on(async move {
/// let mut list_dir = Command::new("ls");
///
/// // Execute `ls` in the current directory of the program.
/// list_dir.status().await.expect("process failed to execute");
///
/// println!();
///
/// // Change `ls` to execute in the root directory.
/// list_dir.current_dir("/");
///
/// // And then execute `ls` again but in the root directory.
/// list_dir.status().await.expect("process failed to execute");
/// # })
/// ```
///
/// See [`std::process::Command`] for detailed documents.
#[derive(Debug)]
pub struct Command(process::Command);

impl Command {
    /// Create [`Command`].
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        Self(process::Command::new(program))
    }

    /// Adds an argument to pass to the program.
    pub fn arg(&mut self, arg: impl AsRef<OsStr>) -> &mut Self {
        self.0.arg(arg);
        self
    }

    /// Adds multiple arguments to pass to the program.
    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.0.args(args);
        self
    }

    /// Inserts or updates an explicit environment variable mapping.
    pub fn env<K, V>(&mut self, key: K, val: V) -> &mut Self
    where
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.0.env(key, val);
        self
    }

    /// Inserts or updates multiple explicit environment variable mappings.
    pub fn envs<I, K, V>(&mut self, vars: I) -> &mut Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.0.envs(vars);
        self
    }

    /// Removes an explicitly set environment variable and prevents inheriting
    /// it from a parent process.
    pub fn env_remove(&mut self, key: impl AsRef<OsStr>) -> &mut Self {
        self.0.env_remove(key);
        self
    }

    /// Clears all explicitly set environment variables and prevents inheriting
    /// any parent process environment variables.
    pub fn env_clear(&mut self) -> &mut Self {
        self.0.env_clear();
        self
    }

    /// Sets the working directory for the child process.
    pub fn current_dir(&mut self, dir: impl AsRef<Path>) -> &mut Self {
        self.0.current_dir(dir);
        self
    }

    /// Configuration for the child process’s standard input (stdin) handle.
    pub fn stdin<S: TryInto<process::Stdio>>(&mut self, cfg: S) -> Result<&mut Self, S::Error> {
        self.0.stdin(cfg.try_into()?);
        Ok(self)
    }

    /// Configuration for the child process’s standard output (stdout) handle.
    pub fn stdout<S: TryInto<process::Stdio>>(&mut self, cfg: S) -> Result<&mut Self, S::Error> {
        self.0.stdout(cfg.try_into()?);
        Ok(self)
    }

    /// Configuration for the child process’s standard error (stderr) handle.
    pub fn stderr<S: TryInto<process::Stdio>>(&mut self, cfg: S) -> Result<&mut Self, S::Error> {
        self.0.stderr(cfg.try_into()?);
        Ok(self)
    }

    /// Returns the path to the program.
    pub fn get_program(&self) -> &OsStr {
        self.0.get_program()
    }

    /// Returns an iterator of the arguments that will be passed to the program.
    pub fn get_args(&self) -> process::CommandArgs<'_> {
        self.0.get_args()
    }

    /// Returns an iterator of the environment variables explicitly set for the
    /// child process.
    pub fn get_envs(&self) -> process::CommandEnvs<'_> {
        self.0.get_envs()
    }

    /// Returns the working directory for the child process.
    pub fn get_current_dir(&self) -> Option<&Path> {
        self.0.get_current_dir()
    }

    /// Executes the command as a child process, returning a handle to it.
    pub fn spawn(&mut self) -> io::Result<Child> {
        #[cfg(all(target_os = "linux", feature = "linux_pidfd"))]
        {
            use std::os::linux::process::CommandExt;
            self.0.create_pidfd(true);
        }
        let mut child = self.0.spawn()?;
        let stdin = if let Some(stdin) = child.stdin.take() {
            Some(ChildStdin::new(stdin)?)
        } else {
            None
        };
        let stdout = if let Some(stdout) = child.stdout.take() {
            Some(ChildStdout::new(stdout)?)
        } else {
            None
        };
        let stderr = if let Some(stderr) = child.stderr.take() {
            Some(ChildStderr::new(stderr)?)
        } else {
            None
        };
        Ok(Child {
            child,
            stdin,
            stdout,
            stderr,
        })
    }

    /// Executes a command as a child process, waiting for it to finish and
    /// collecting its status. The output of child stdout and child stderr will
    /// be ignored.
    pub async fn status(&mut self) -> io::Result<process::ExitStatus> {
        let child = self.spawn()?;
        child.wait().await
    }

    /// Executes the command as a child process, waiting for it to finish and
    /// collecting all of its output.
    pub async fn output(&mut self) -> io::Result<process::Output> {
        let child = self.spawn()?;
        child.wait_with_output().await
    }
}

#[cfg(windows)]
impl Command {
    /// Sets the [process creation flags][1] to be passed to `CreateProcess`.
    ///
    /// These will always be ORed with `CREATE_UNICODE_ENVIRONMENT`.
    ///
    /// [1]: https://docs.microsoft.com/en-us/windows/win32/procthread/process-creation-flags
    pub fn creation_flags(&mut self, flags: u32) -> &mut Self {
        self.0.creation_flags(flags);
        self
    }

    /// Append literal text to the command line without any quoting or escaping.
    pub fn raw_arg(&mut self, text_to_append_as_is: impl AsRef<OsStr>) -> &mut Self {
        self.0.raw_arg(text_to_append_as_is);
        self
    }
}

#[cfg(unix)]
impl Command {
    /// Sets the child process’s user ID. This translates to a `setuid`` call in
    /// the child process. Failure in the `setuid` call will cause the spawn to
    /// fail.
    pub fn uid(&mut self, id: u32) -> &mut Self {
        self.0.uid(id);
        self
    }

    /// Similar to `uid`, but sets the group ID of the child process. This has
    /// the same semantics as the `uid` field.
    pub fn gid(&mut self, id: u32) -> &mut Self {
        self.0.gid(id);
        self
    }

    /// Schedules a closure to be run just before the `exec` function is
    /// invoked.
    ///
    /// # Safety
    ///
    /// See [`CommandExt::pre_exec`].
    pub unsafe fn pre_exec(
        &mut self,
        f: impl FnMut() -> io::Result<()> + Send + Sync + 'static,
    ) -> &mut Self {
        self.0.pre_exec(f);
        self
    }

    /// `exec` the command without `fork`.
    pub fn exec(&mut self) -> io::Error {
        self.0.exec()
    }

    /// Set the first process argument, `argv[0]`, to something other than the
    /// default executable path.
    pub fn arg0(&mut self, arg: impl AsRef<OsStr>) -> &mut Self {
        self.0.arg0(arg);
        self
    }

    /// Sets the process group ID (PGID) of the child process.
    pub fn process_group(&mut self, pgroup: i32) -> &mut Self {
        self.0.process_group(pgroup);
        self
    }
}

/// Representation of a running or exited child process.
///
/// This structure is used to represent and manage child processes. A child
/// process is created via the [`Command`] struct, which configures the
/// spawning process and can itself be constructed using a builder-style
/// interface.
///
/// There is no implementation of [`Drop`] for child processes,
/// so if you do not ensure the `Child` has exited then it will continue to
/// run, even after the `Child` handle to the child process has gone out of
/// scope.
///
/// Calling [`Child::wait`] (or other functions that wrap around it) will make
/// the parent process wait until the child has actually exited before
/// continuing.
///
/// See [`std::process::Child`] for detailed documents.
pub struct Child {
    child: process::Child,
    /// The handle for writing to the child’s standard input (stdin).
    pub stdin: Option<ChildStdin>,
    /// The handle for reading from the child’s standard output (stdout).
    pub stdout: Option<ChildStdout>,
    /// The handle for reading from the child’s standard error (stderr).
    pub stderr: Option<ChildStderr>,
}

impl Child {
    /// Forces the child process to exit. If the child has already exited,
    /// `Ok(())`` is returned.
    pub fn kill(&mut self) -> io::Result<()> {
        self.child.kill()
    }

    /// Returns the OS-assigned process identifier associated with this child.
    pub fn id(&self) -> u32 {
        self.child.id()
    }

    /// Waits for the child to exit completely, returning the status that it
    /// exited with. This function will consume the child. To get the output,
    /// either take `stdout` and `stderr` out before calling it, or call
    /// [`Child::wait_with_output`].
    pub async fn wait(self) -> io::Result<process::ExitStatus> {
        sys::child_wait(self.child).await
    }

    /// Simultaneously waits for the child to exit and collect all remaining
    /// output on the stdout/stderr handles, returning an Output instance.
    pub async fn wait_with_output(mut self) -> io::Result<process::Output> {
        let status = sys::child_wait(self.child);
        let stdout = if let Some(stdout) = &mut self.stdout {
            Either::Left(stdout.read_to_end(vec![]))
        } else {
            Either::Right(std::future::ready(BufResult(Ok(0), vec![])))
        };
        let stderr = if let Some(stderr) = &mut self.stderr {
            Either::Left(stderr.read_to_end(vec![]))
        } else {
            Either::Right(std::future::ready(BufResult(Ok(0), vec![])))
        };
        let (status, BufResult(out_res, stdout), BufResult(err_res, stderr)) =
            futures_util::future::join3(status, stdout, stderr).await;
        let status = status?;
        out_res?;
        err_res?;
        Ok(process::Output {
            status,
            stdout,
            stderr,
        })
    }
}

/// A handle to a child process's standard output (stdout). See
/// [`std::process::ChildStdout`].
pub struct ChildStdout(Attacher<process::ChildStdout>);

impl ChildStdout {
    fn new(stdout: process::ChildStdout) -> io::Result<Self> {
        Attacher::new(stdout).map(Self)
    }
}

impl TryFrom<ChildStdout> for process::Stdio {
    type Error = ChildStdout;

    fn try_from(value: ChildStdout) -> Result<Self, ChildStdout> {
        value
            .0
            .into_inner()
            .try_unwrap()
            .map(Self::from)
            .map_err(|fd| ChildStdout(unsafe { Attacher::from_shared_fd_unchecked(fd) }))
    }
}

impl AsFd for ChildStdout {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl AsRawFd for ChildStdout {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

impl ToSharedFd<process::ChildStdout> for ChildStdout {
    fn to_shared_fd(&self) -> SharedFd<process::ChildStdout> {
        self.0.to_shared_fd()
    }
}

/// A handle to a child process's stderr. See [`std::process::ChildStderr`].
pub struct ChildStderr(Attacher<process::ChildStderr>);

impl ChildStderr {
    fn new(stderr: process::ChildStderr) -> io::Result<Self> {
        Attacher::new(stderr).map(Self)
    }
}

impl TryFrom<ChildStderr> for process::Stdio {
    type Error = ChildStderr;

    fn try_from(value: ChildStderr) -> Result<Self, ChildStderr> {
        value
            .0
            .into_inner()
            .try_unwrap()
            .map(Self::from)
            .map_err(|fd| ChildStderr(unsafe { Attacher::from_shared_fd_unchecked(fd) }))
    }
}

impl AsFd for ChildStderr {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl AsRawFd for ChildStderr {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

impl ToSharedFd<process::ChildStderr> for ChildStderr {
    fn to_shared_fd(&self) -> SharedFd<process::ChildStderr> {
        self.0.to_shared_fd()
    }
}

/// A handle to a child process's standard input (stdin). See
/// [`std::process::ChildStdin`].
pub struct ChildStdin(Attacher<process::ChildStdin>);

impl ChildStdin {
    fn new(stdin: process::ChildStdin) -> io::Result<Self> {
        Attacher::new(stdin).map(Self)
    }
}

impl TryFrom<ChildStdin> for process::Stdio {
    type Error = ChildStdin;

    fn try_from(value: ChildStdin) -> Result<Self, ChildStdin> {
        value
            .0
            .into_inner()
            .try_unwrap()
            .map(Self::from)
            .map_err(|fd| ChildStdin(unsafe { Attacher::from_shared_fd_unchecked(fd) }))
    }
}

impl AsFd for ChildStdin {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl AsRawFd for ChildStdin {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

impl ToSharedFd<process::ChildStdin> for ChildStdin {
    fn to_shared_fd(&self) -> SharedFd<process::ChildStdin> {
        self.0.to_shared_fd()
    }
}
