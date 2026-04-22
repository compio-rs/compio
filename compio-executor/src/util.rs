use std::task::Poll;

/// Create a guard that abort the process when the thread panicked before it's
/// out of scope.
///
/// If loom is enabled, this does nothing.
macro_rules! panic_guard {
    ($f:expr) => {
        if cfg!(loom) {
            $f()
        } else {
            ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| $f()))
                .unwrap_or_else(|_| ::std::process::abort())
        }
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
        if var_os("COMPIO_TEST_ABORT").is_some() {
            panic_guard!(|| {
                panic!("This should abort");
            })
        }

        let exe = current_exe().unwrap();
        let mut cmd = Command::new(exe);
        cmd.arg("test_panic_guard")
            .env("COMPIO_TEST_ABORT", "1")
            .stderr(Stdio::piped())
            .stdout(Stdio::piped());

        unsafe {
            cmd.pre_exec(|| {
                // Disable core dump for aborted subprocess
                setrlimit(Resource::RLIMIT_CORE, 0, 0).map_err(Into::into)
            })
        };

        let status = cmd.status().unwrap();

        assert_eq!(status.signal(), Some(Signal::SIGABRT as i32))
    }
}
