use std::ops::{Deref, DerefMut};

use super::*;
use crate::{Decision, OpType, op::VectoredControl};

/// Meta of AIO operations.
#[cfg(aio)]
#[derive(Debug, Clone, Copy)]
pub struct AioArg {
    /// Pointer of the control block.
    pub aiocbp: NonNull<libc::aiocb>,
    /// The aio_* submit function.
    pub submit: unsafe extern "C" fn(*mut libc::aiocb) -> i32,
}

pub struct AioControl<B = ()> {
    pub base: B,
    #[cfg(aio)]
    pub aiocb: libc::aiocb,
}

impl AioControl {
    cfg_if! {
        if #[cfg(aio)] {
            pub fn init_fd<Fd: AsFd>(&mut self, fd: Fd) {
                self.aiocb.aio_fildes = fd.as_fd().as_raw_fd();
            }

            pub fn init<Fd: AsFd, B: IoBuf>(&mut self, fd: Fd, buf: &B, offset: u64) {
                let slice = buf.sys_slice();
                self.init_fd(fd);
                self.aiocb.aio_offset = offset as _;
                self.aiocb.aio_buf = slice.ptr().cast();
                self.aiocb.aio_nbytes = slice.len();
            }

            pub fn init_mut<Fd: AsFd, B: IoBufMut>(&mut self, fd: Fd, buf: &mut B, offset: u64) {
                let slice = buf.sys_slice_mut();
                self.init_fd(fd);
                self.aiocb.aio_offset = offset as _;
                self.aiocb.aio_buf = slice.ptr().cast();
                self.aiocb.aio_nbytes = slice.len();
            }

            pub fn op_type(&mut self) -> Option<OpType> {
                Some(OpType::Aio(NonNull::from_mut(&mut self.aiocb)))
            }

            pub fn decide_read(&mut self) -> io::Result<Decision> {
                Ok(Decision::aio(&mut self.aiocb, libc::aio_read))
            }

            pub fn decide_write(&mut self) -> io::Result<Decision> {
                Ok(Decision::aio(&mut self.aiocb, libc::aio_write))
            }

            pub fn decide_sync(&mut self, datasync: bool) -> io::Result<Decision> {
                unsafe extern "C" fn aio_fsync(aiocbp: *mut libc::aiocb) -> i32 {
                    unsafe { libc::aio_fsync(libc::O_SYNC, aiocbp) }
                }

                unsafe extern "C" fn aio_fdatasync(aiocbp: *mut libc::aiocb) -> i32 {
                    unsafe { libc::aio_fsync(libc::O_DSYNC, aiocbp) }
                }

                let f = if datasync {
                    aio_fdatasync
                } else {
                    aio_fsync
                };

                Ok(Decision::aio(&mut self.aiocb, f))
            }
        } else {
            pub fn init_fd<Fd: AsFd>(&mut self, _: Fd) {}

            pub fn init<Fd: AsFd, B: IoBuf>(&mut self, _: Fd, _: &B, _: u64) {}

            pub fn init_mut<Fd: AsFd, B: IoBufMut>(&mut self, _: Fd, _: &mut B, _: u64) {}

            pub fn op_type(&mut self) -> Option<OpType> {
                None
            }

            pub fn decide_read(&mut self) -> io::Result<Decision> {
                Ok(Decision::Blocking)
            }

            pub fn decide_write(&mut self) -> io::Result<Decision> {
                Ok(Decision::Blocking)
            }

            pub fn decide_sync(&mut self, _: bool) -> io::Result<Decision> {
                Ok(Decision::Blocking)
            }
        }
    }
}

impl AioControl<VectoredControl> {
    cfg_if! {
        if #[cfg(freebsd)] {
            pub fn op_type(&mut self) -> Option<OpType> {
                Some(OpType::Aio(NonNull::from_mut(&mut self.aiocb)))
            }

            pub fn decide_read(&mut self) -> io::Result<Decision> {
                Ok(Decision::aio(&mut self.aiocb, libc::aio_readv))
            }

            pub fn decide_write(&mut self) -> io::Result<Decision> {
                Ok(Decision::aio(&mut self.aiocb, libc::aio_writev))
            }
        } else {
            pub fn op_type(&mut self) -> Option<OpType> {
                None
            }

            pub fn decide_read(&mut self) -> io::Result<Decision> {
                Ok(Decision::Blocking)
            }

            pub fn decide_write(&mut self) -> io::Result<Decision> {
                Ok(Decision::Blocking)
            }
        }
    }

    pub fn init_vec<Fd: AsFd, B: IoVectoredBuf>(&mut self, fd: Fd, buf: &B, offset: u64) {
        self.base.slices = buf.sys_slices();

        cfg_if! {
            if #[cfg(freebsd)] {
                self.aiocb.aio_fildes = fd.as_fd().as_raw_fd();
                self.aiocb.aio_offset = offset as _;
                self.aiocb.aio_buf = self.base.slices.as_ptr().cast_mut().cast();
                self.aiocb.aio_nbytes = self.base.slices.len();
            } else {
                _ = (fd, offset);
            }
        }
    }

    pub fn init_vec_mut<Fd: AsFd, B: IoVectoredBufMut>(
        &mut self,
        fd: Fd,
        buf: &mut B,
        offset: u64,
    ) {
        self.base.slices = buf.sys_slices_mut();

        cfg_if! {
            if #[cfg(freebsd)] {
                self.aiocb.aio_fildes = fd.as_fd().as_raw_fd();
                self.aiocb.aio_offset = offset as _;
                self.aiocb.aio_buf = self.base.slices.as_ptr().cast_mut().cast();
                self.aiocb.aio_nbytes = self.base.slices.len();
            } else {
                _ = (fd, offset);
            }
        }
    }
}

impl<B: Default> Default for AioControl<B> {
    fn default() -> Self {
        Self {
            base: Default::default(),
            #[cfg(aio)]
            aiocb: unsafe { std::mem::zeroed() },
        }
    }
}

impl<B> DerefMut for AioControl<B> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}

impl<B> Deref for AioControl<B> {
    type Target = B;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}
