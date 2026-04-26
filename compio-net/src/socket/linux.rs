use std::{mem::MaybeUninit, ops::Deref, slice, sync::atomic::Ordering};

use libc::utsname;
#[cfg(feature = "sync")]
use synchrony::sync::atomic::{AtomicBool, AtomicI8};
#[cfg(not(feature = "sync"))]
use synchrony::unsync::atomic::{AtomicBool, AtomicI8};

// We are not on the IO_URING driver and hence retrieving socket state is
// not supported.
const UNSUPPORTED: i8 = -1;

// The socket was empty after the receive operation.
const EMPTY: i8 = 0;

// The socket was not-empty after the last receive operation and has more
// data to be read.
const NON_EMPTY: i8 = 1;

#[derive(Debug)]
pub(super) struct SocketState {
    state: AtomicI8,
    poll_first: AtomicBool,
    poll_first_supported: bool,
}

impl Default for SocketState {
    fn default() -> Self {
        Self::new()
    }
}

impl SocketState {
    pub(super) fn new() -> Self {
        let mut utsname = MaybeUninit::<utsname>::zeroed();
        let res = unsafe { libc::uname(utsname.as_mut_ptr()) };
        let poll_first_supported = if res == 0 {
            let res = unsafe { utsname.assume_init() };
            let release = res.release;

            // We separate the two parts as string slice(lexiographic comparison) or floating point
            // comparison can give false results.
            let major = release
                .iter()
                .position(|&b| b == (b'.' as i8))
                .expect("Extracting the major version");

            let minor = release[(major + 1)..]
                .iter()
                .position(|&b| b == (b'.' as i8))
                .expect("Extracting the minor version");

            let ptr = release[(major + 1)..].as_ptr();

            // We ignore the `.` separator while creating the slice.
            let minor = unsafe { slice::from_raw_parts(ptr.cast(), minor) };
            let major = unsafe { slice::from_raw_parts(release.as_ptr().cast(), major) };

            let minor = str::from_utf8(minor)
                .expect("Kernel version byte slice is valid UTF-8")
                .parse::<u32>()
                .expect("Parsed out string slice is valid u32");

            let major = str::from_utf8(major)
                .expect("Kernel version byte slice is valid UTF-8")
                .parse::<u32>()
                .expect("Parsed out string slice is valid u32");

            //`IORING_RECVSEND_POLL_FIRST` is available since the `5.19` release of the `Linux`
            //kernel.
            major >= 5 && minor >= 19
        } else {
            false
        };
        Self {
            state: AtomicI8::new(-1),
            poll_first: AtomicBool::new(false),
            poll_first_supported,
        }
    }

    pub(super) fn get(&self) -> Option<bool> {
        match self.load(Ordering::Relaxed) {
            UNSUPPORTED => None,
            EMPTY => Some(false),
            NON_EMPTY => Some(true),
            _ => unreachable!(),
        }
    }

    pub(super) fn set(&self, extra: &compio_driver::Extra) {
        if let Ok(n) = extra.sock_nonempty() {
            self.store(n as i8, Ordering::Relaxed);
        }
    }

    pub(super) fn set_poll_first(&self, flag: bool) {
        if self.poll_first_supported {
            self.poll_first.store(flag, Ordering::Relaxed);
        }
    }

    pub(super) fn get_poll_first(&self) -> bool {
        self.poll_first.load(Ordering::Relaxed)
    }
}

impl Clone for SocketState {
    fn clone(&self) -> Self {
        let current = self.state.load(Ordering::Relaxed);
        let poll_first = self.poll_first.load(Ordering::Relaxed);
        Self {
            state: AtomicI8::new(current),
            poll_first: AtomicBool::new(poll_first),
            poll_first_supported: self.poll_first_supported,
        }
    }
}

impl Deref for SocketState {
    type Target = AtomicI8;

    fn deref(&self) -> &Self::Target {
        &self.state
    }
}
