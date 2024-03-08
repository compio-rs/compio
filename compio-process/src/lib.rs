//! Process library for compio. It is an extension to [`std::process`].

#![warn(missing_docs)]

cfg_if::cfg_if! {
    if #[cfg(windows)] {
        #[path = "windows.rs"]
        mod sys;
    } else if #[cfg(any(target_os="linux",target_os="android"))] {
        #[path = "linux.rs"]
        mod sys;
    } else {
        #[path = "unix.rs"]
        mod sys;
    }
}

use std::{
    ffi::OsStr,
    io,
    path::Path,
    process::{self},
};

use compio_buf::BufResult;
use compio_io::AsyncReadExt;

pub struct Command(process::Command);

impl Command {
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        Self(process::Command::new(program))
    }

    pub fn arg(&mut self, arg: impl AsRef<OsStr>) -> &mut Self {
        self.0.arg(arg);
        self
    }

    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.0.args(args);
        self
    }

    pub fn env<K, V>(&mut self, key: K, val: V) -> &mut Self
    where
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.0.env(key, val);
        self
    }

    pub fn envs<I, K, V>(&mut self, vars: I) -> &mut Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.0.envs(vars);
        self
    }

    pub fn env_remove(&mut self, key: impl AsRef<OsStr>) -> &mut Self {
        self.0.env_remove(key);
        self
    }

    pub fn env_clear(&mut self) -> &mut Self {
        self.0.env_clear();
        self
    }

    pub fn current_dir(&mut self, dir: impl AsRef<Path>) -> &mut Self {
        self.0.current_dir(dir);
        self
    }

    pub fn stdin(&mut self, cfg: impl Into<process::Stdio>) -> &mut Self {
        self.0.stdin(cfg);
        self
    }

    pub fn stdout(&mut self, cfg: impl Into<process::Stdio>) -> &mut Self {
        self.0.stdout(cfg);
        self
    }

    pub fn stderr(&mut self, cfg: impl Into<process::Stdio>) -> &mut Self {
        self.0.stderr(cfg);
        self
    }

    pub fn get_program(&self) -> &OsStr {
        self.0.get_program()
    }

    pub fn get_args(&self) -> process::CommandArgs {
        self.0.get_args()
    }

    pub fn get_envs(&self) -> process::CommandEnvs {
        self.0.get_envs()
    }

    pub fn get_current_dir(&self) -> Option<&Path> {
        self.0.get_current_dir()
    }

    pub fn spawn(&mut self) -> io::Result<Child> {
        let mut child = self.0.spawn()?;
        let stdin = child.stdin.take().map(ChildStdin);
        let stdout = child.stdout.take().map(ChildStdout);
        let stderr = child.stderr.take().map(ChildStderr);
        Ok(Child {
            child,
            stdin,
            stdout,
            stderr,
        })
    }

    pub async fn status(&mut self) -> io::Result<process::ExitStatus> {
        let child = self.spawn()?;
        child.wait().await
    }

    pub async fn output(&mut self) -> io::Result<process::Output> {
        let child = self.spawn()?;
        child.wait_with_output().await
    }
}

pub struct Child {
    child: process::Child,
    pub stdin: Option<ChildStdin>,
    pub stdout: Option<ChildStdout>,
    pub stderr: Option<ChildStderr>,
}

impl Child {
    pub fn kill(&mut self) -> io::Result<()> {
        self.child.kill()
    }

    pub fn id(&self) -> u32 {
        self.child.id()
    }

    pub async fn wait(self) -> io::Result<process::ExitStatus> {
        sys::child_wait(self.child).await
    }

    pub async fn wait_with_output(mut self) -> io::Result<process::Output> {
        let status = sys::child_wait(self.child).await?;
        let stdout_buffer = if let Some(stdout) = &mut self.stdout {
            let BufResult(res, buffer) = stdout.read_to_end(vec![]).await;
            res?;
            buffer
        } else {
            vec![]
        };
        let stderr_buffer = if let Some(stderr) = &mut self.stderr {
            let BufResult(res, buffer) = stderr.read_to_end(vec![]).await;
            res?;
            buffer
        } else {
            vec![]
        };
        Ok(process::Output {
            status,
            stdout: stdout_buffer,
            stderr: stderr_buffer,
        })
    }
}

pub struct ChildStdout(process::ChildStdout);

impl From<ChildStdout> for process::Stdio {
    fn from(value: ChildStdout) -> Self {
        Self::from(value.0)
    }
}

pub struct ChildStderr(process::ChildStderr);

impl From<ChildStderr> for process::Stdio {
    fn from(value: ChildStderr) -> Self {
        Self::from(value.0)
    }
}

pub struct ChildStdin(process::ChildStdin);

impl From<ChildStdin> for process::Stdio {
    fn from(value: ChildStdin) -> Self {
        Self::from(value.0)
    }
}
