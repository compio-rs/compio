use std::task::Poll;

/// Create a guard that abort the process when the thread panicked before it's
/// out of scope.
pub(crate) struct AbortOnPanic;

impl AbortOnPanic {
    pub(crate) fn unless_unwinding() -> Option<Self> {
        // If the thread is already unwinding, no guard is created.
        if std::thread::panicking() {
            None
        } else {
            Some(Self)
        }
    }
}

/// If loom is enabled, this does nothing.
impl Drop for AbortOnPanic {
    #[cfg(loom)]
    fn drop(&mut self) {}

    #[cfg(not(loom))]
    fn drop(&mut self) {
        if std::thread::panicking() {
            std::process::abort()
        }
    }
}

macro_rules! panic_guard {
    () => {
        let _b = $crate::util::AbortOnPanic;
    };
    (skip_if_unwinding) => {
        let _b = $crate::util::AbortOnPanic::unless_unwinding();
    };
}

pub(crate) use panic_guard;

#[inline(always)]
pub(crate) fn transpose<T, E>(poll: Result<Poll<T>, E>) -> Poll<Result<T, E>> {
    match poll {
        Ok(Poll::Pending) => Poll::Pending,
        Ok(Poll::Ready(t)) => Poll::Ready(Ok(t)),
        Err(e) => Poll::Ready(Err(e)),
    }
}

macro_rules! assert_not_impl {
    ($x:ty, $($t:path),+ $(,)*) => {
        const _: fn() -> () = || {
            struct Check<T: ?Sized>(T);
            trait AmbiguousIfImpl<A> { fn some_item() { } }

            impl<T: ?Sized> AmbiguousIfImpl<()> for Check<T> { }
            impl<T: ?Sized $(+ $t)*> AmbiguousIfImpl<u8> for Check<T> { }

            <Check::<$x> as AmbiguousIfImpl<_>>::some_item()
        };
    };
}

pub(crate) use assert_not_impl;

#[cfg(all(test, not(loom)))]
mod test_unwinding {
    use std::panic::catch_unwind;

    struct GuardInDrop;

    impl Drop for GuardInDrop {
        fn drop(&mut self) {
            panic_guard!(skip_if_unwinding);
        }
    }

    #[test]
    fn guard_created_while_unwinding_does_not_abort() {
        let res = catch_unwind(|| {
            let _g = GuardInDrop;
            panic!("panic before the guard is created");
        });
        assert!(res.is_err());
    }
}

#[cfg(all(test, unix))]
mod test {
    use std::{
        env::{current_exe, var_os},
        os::unix::process::{CommandExt, ExitStatusExt},
        process::{Command, Stdio},
    };

    use nix::sys::{
        resource::{Resource, setrlimit},
        signal::Signal,
    };

    #[test]
    fn test_panic_guard() {
        let mode = var_os("COMPIO_TEST_ABORT").map(|mode| mode.into_string().unwrap());
        match mode.as_deref() {
            Some("default") => {
                panic_guard!();
                panic!("This should abort");
            }
            Some("skip_if_unwinding") => {
                panic_guard!(skip_if_unwinding);
                panic!("This should abort");
            }
            Some(mode) => unreachable!("unknown mode: {mode}"),
            None => {}
        }

        for mode in ["default", "skip_if_unwinding"] {
            let exe = current_exe().unwrap();
            let mut cmd = Command::new(exe);
            cmd.arg("test_panic_guard")
                .env("COMPIO_TEST_ABORT", mode)
                .stderr(Stdio::piped())
                .stdout(Stdio::piped());

            unsafe {
                cmd.pre_exec(|| {
                    // Disable core dump for aborted subprocess
                    setrlimit(Resource::RLIMIT_CORE, 0, 0).map_err(Into::into)
                })
            };

            let status = cmd.status().unwrap();

            assert_eq!(status.signal(), Some(Signal::SIGABRT as i32), "{mode}");
        }
    }
}
