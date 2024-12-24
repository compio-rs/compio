//! Linux-specific types for signal handling.

use std::{
    cell::RefCell, collections::HashMap, io, mem::MaybeUninit, os::fd::FromRawFd, ptr::null_mut,
    thread_local,
};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut, SetBufInit};
use compio_driver::{OwnedFd, SharedFd, op::Recv, syscall};

thread_local! {
    static REG_MAP: RefCell<HashMap<i32, usize>> = RefCell::new(HashMap::new());
}

fn sigset(sig: i32) -> io::Result<libc::sigset_t> {
    let mut set: MaybeUninit<libc::sigset_t> = MaybeUninit::uninit();
    syscall!(libc::sigemptyset(set.as_mut_ptr()))?;
    syscall!(libc::sigaddset(set.as_mut_ptr(), sig))?;
    // SAFETY: sigemptyset initializes the set.
    Ok(unsafe { set.assume_init() })
}

fn register_signal(sig: i32) -> io::Result<libc::sigset_t> {
    REG_MAP.with_borrow_mut(|map| {
        let count = map.entry(sig).or_default();
        let set = sigset(sig)?;
        if *count == 0 {
            syscall!(libc::pthread_sigmask(libc::SIG_BLOCK, &set, null_mut()))?;
        }
        *count += 1;
        Ok(set)
    })
}

fn unregister_signal(sig: i32) -> io::Result<libc::sigset_t> {
    REG_MAP.with_borrow_mut(|map| {
        let count = map.entry(sig).or_default();
        if *count > 0 {
            *count -= 1;
        }
        let set = sigset(sig)?;
        if *count == 0 {
            syscall!(libc::pthread_sigmask(libc::SIG_UNBLOCK, &set, null_mut()))?;
        }
        Ok(set)
    })
}

/// Represents a listener to unix signal event.
#[derive(Debug)]
struct SignalFd {
    fd: SharedFd<OwnedFd>,
    sig: i32,
}

impl SignalFd {
    fn new(sig: i32) -> io::Result<Self> {
        let set = register_signal(sig)?;
        let mut flag = libc::SFD_CLOEXEC;
        if cfg!(not(feature = "io-uring")) || compio_driver::DriverType::is_polling() {
            flag |= libc::SFD_NONBLOCK;
        }
        let fd = syscall!(libc::signalfd(-1, &set, flag))?;
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        Ok(Self {
            fd: SharedFd::new(fd),
            sig,
        })
    }

    async fn wait(self) -> io::Result<()> {
        const INFO_SIZE: usize = std::mem::size_of::<libc::signalfd_siginfo>();

        struct SignalInfo(MaybeUninit<libc::signalfd_siginfo>);

        unsafe impl IoBuf for SignalInfo {
            fn as_buf_ptr(&self) -> *const u8 {
                self.0.as_ptr().cast()
            }

            fn buf_len(&self) -> usize {
                0
            }

            fn buf_capacity(&self) -> usize {
                INFO_SIZE
            }
        }

        unsafe impl IoBufMut for SignalInfo {
            fn as_buf_mut_ptr(&mut self) -> *mut u8 {
                self.0.as_mut_ptr().cast()
            }
        }

        impl SetBufInit for SignalInfo {
            unsafe fn set_buf_init(&mut self, len: usize) {
                debug_assert!(len <= INFO_SIZE)
            }
        }

        let info = SignalInfo(MaybeUninit::<libc::signalfd_siginfo>::uninit());
        let op = Recv::new(self.fd.clone(), info);
        let BufResult(res, op) = compio_runtime::submit(op).await;
        let len = res?;
        debug_assert_eq!(len, INFO_SIZE);
        let info = op.into_inner();
        let info = unsafe { info.0.assume_init() };
        debug_assert_eq!(info.ssi_signo, self.sig as u32);
        Ok(())
    }
}

impl Drop for SignalFd {
    fn drop(&mut self) {
        unregister_signal(self.sig).ok();
    }
}

/// Creates a new listener which will receive notifications when the current
/// process receives the specified signal.
///
/// It sets the signal mask of the current thread.
pub async fn signal(sig: i32) -> io::Result<()> {
    let fd = SignalFd::new(sig)?;
    fd.wait().await?;
    Ok(())
}
