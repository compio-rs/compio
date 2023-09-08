//! Unix-specific types for signal handling.

#[cfg(feature = "lazy_cell")]
use std::cell::LazyCell;
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
};

#[cfg(not(feature = "lazy_cell"))]
use once_cell::unsync::Lazy as LazyCell;

use crate::{op::ReadAt, task::RUNTIME};

thread_local! {
    #[allow(clippy::type_complexity)]
    static HANDLER: LazyCell<RefCell<HashMap<i32, HashSet<RawFd>>>> =
        LazyCell::new(|| RefCell::new(HashMap::new()));
}

unsafe extern "C" fn signal_handler(sig: i32) {
    HANDLER.with(|handler| {
        let mut handler = handler.borrow_mut();
        if let Some(fds) = handler.get_mut(&sig) {
            if !fds.is_empty() {
                let fds = std::mem::take(fds);
                for fd in fds {
                    let data = 1u64;
                    libc::write(fd, &data as *const _ as *const _, 8);
                }
            }
        }
    });
}

unsafe fn init(sig: i32) {
    libc::signal(sig, signal_handler as *const () as usize);
}

unsafe fn uninit(sig: i32) {
    libc::signal(sig, libc::SIG_DFL);
}

fn register(sig: i32, fd: RawFd) {
    unsafe { init(sig) };
    HANDLER.with(|handler| handler.borrow_mut().entry(sig).or_default().insert(fd));
}

fn unregister(sig: i32, fd: RawFd) {
    let need_uninit = HANDLER.with(|handler| {
        let mut handler = handler.borrow_mut();
        if let Some(fds) = handler.get_mut(&sig) {
            fds.remove(&fd);
            if !fds.is_empty() {
                return false;
            }
        }
        true
    });
    if need_uninit {
        unsafe { uninit(sig) };
    }
}

/// Represents a listener to unix signal event.
#[derive(Debug)]
pub struct SignalFd {
    sig: i32,
    fd: OwnedFd,
}

impl SignalFd {
    pub(crate) fn new(sig: i32) -> io::Result<Self> {
        let fd = unsafe { libc::eventfd(0, 0) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        register(sig, fd);
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        Ok(Self { sig, fd })
    }
}

impl Drop for SignalFd {
    fn drop(&mut self) {
        unregister(self.sig, self.fd.as_raw_fd());
    }
}

/// Creates a new listener which will receive notifications when the current
/// process receives the specified signal.
pub async fn signal(sig: i32) -> io::Result<()> {
    let fd = SignalFd::new(sig)?;
    let buffer = Vec::with_capacity(8);
    let op = ReadAt::new(fd.fd.as_raw_fd(), 0, buffer);
    let (res, _) = RUNTIME.with(|runtime| runtime.submit(op)).await;
    res?;
    Ok(())
}
