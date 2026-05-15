#[cfg(target_os = "linux")]
use std::os::fd::OwnedFd;
use std::{
    io,
    ops::Deref,
    os::fd::{AsFd, AsRawFd, BorrowedFd, RawFd},
};

use compio_runtime::Runtime;
use mod_use::mod_use;

#[cfg(feature = "tokio")]
mod_use![tokio];

#[cfg(feature = "futures")]
mod_use![futures];

struct UnixAdapter {
    runtime: Runtime,
    #[cfg(target_os = "linux")]
    efd: Option<OwnedFd>,
}

#[cfg(target_os = "linux")]
impl UnixAdapter {
    fn new(runtime: Runtime) -> io::Result<Self> {
        if runtime.driver_type().is_iouring() {
            use rustix::{
                event::{EventfdFlags, eventfd},
                io_uring::{IoringRegisterOp, io_uring_register},
            };

            let efd = eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK)?;
            let efd_raw = efd.as_raw_fd();
            unsafe {
                io_uring_register(
                    BorrowedFd::borrow_raw(runtime.as_raw_fd()),
                    IoringRegisterOp::RegisterEventfd,
                    (&raw const efd_raw).cast(),
                    1,
                )?;
            }
            Ok(Self {
                runtime,
                efd: Some(efd),
            })
        } else {
            Ok(Self { runtime, efd: None })
        }
    }

    fn clear(&self) -> io::Result<()> {
        if let Some(efd) = &self.efd {
            let mut buf = [0u8; 8];
            match rustix::io::read(efd, &mut buf) {
                Ok(_) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(e) => return Err(io::Error::from(e)),
            }
        }
        Ok(())
    }
}

#[cfg(not(target_os = "linux"))]
impl UnixAdapter {
    fn new(runtime: Runtime) -> io::Result<Self> {
        Ok(Self { runtime })
    }

    fn clear(&self) -> io::Result<()> {
        Ok(())
    }
}

impl AsRawFd for UnixAdapter {
    fn as_raw_fd(&self) -> RawFd {
        #[cfg(target_os = "linux")]
        {
            self.efd
                .as_ref()
                .map(|f| f.as_raw_fd())
                .unwrap_or_else(|| self.runtime.as_raw_fd())
        }
        #[cfg(not(target_os = "linux"))]
        {
            self.runtime.as_raw_fd()
        }
    }
}

impl AsFd for UnixAdapter {
    fn as_fd(&self) -> BorrowedFd<'_> {
        unsafe { BorrowedFd::borrow_raw(self.as_raw_fd()) }
    }
}

impl Deref for UnixAdapter {
    type Target = Runtime;

    fn deref(&self) -> &Self::Target {
        &self.runtime
    }
}
