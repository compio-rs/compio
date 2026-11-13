//! `IORING_CQE_F_BUF_MORE` reaches the op through `Extra`.
//!
//! The test registers a provided buffer ring of its own: the driver's buffer
//! pool refuses `IOU_PBUF_RING_INC`.

#![cfg(io_uring)]

use std::{
    io::{self, Write},
    mem::size_of,
    num::NonZeroU16,
    os::{fd::RawFd, unix::net::UnixStream},
    ptr::{self, NonNull},
    sync::atomic::{AtomicU16, Ordering},
};

use compio_buf::BufResult;
use compio_driver::{AsRawFd, Extra, IourOpCode, OpEntry, Proactor, ProactorBuilder, PushEntry};
use io_uring::{
    opcode,
    squeue::Flags,
    types::{BufRingEntry, Fd, IOU_PBUF_RING_INC},
};
use linux_raw_sys::io_uring::{io_uring_buf_reg, io_uring_register_op};
use rustix::mm::{MapFlags, ProtFlags, mmap_anonymous, munmap};

/// A buffer group the driver's own pool does not use.
const BUFFER_GROUP: u16 = 0x7e57;
const BUFFER_LEN: usize = 4096;

/// A provided buffer ring holding one buffer, registered with the driver's
/// ring. It must be dropped before the driver.
struct BufRing {
    ring_fd: RawFd,
    entries: NonNull<BufRingEntry>,
    buffer: Box<[u8]>,
}

fn register(ring_fd: RawFd, op: io_uring_register_op, reg: &io_uring_buf_reg) -> io::Result<()> {
    // SAFETY: `reg` is a valid `io_uring_buf_reg` for the duration of the call.
    let res = unsafe {
        libc::syscall(
            libc::SYS_io_uring_register,
            ring_fd,
            op as libc::c_uint,
            ptr::from_ref(reg),
            1,
        )
    };
    if res < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

impl BufRing {
    fn new(driver: &Proactor, flags: u16) -> io::Result<Self> {
        let ring_fd = driver.as_raw_fd();
        // SAFETY: a fresh private anonymous map, zeroed by the kernel.
        let entries = unsafe {
            mmap_anonymous(
                ptr::null_mut(),
                size_of::<BufRingEntry>(),
                ProtFlags::READ | ProtFlags::WRITE,
                MapFlags::PRIVATE,
            )
        }?;
        let entries = NonNull::new(entries.cast::<BufRingEntry>()).expect("mmap returned null");
        let mut buffer = vec![0u8; BUFFER_LEN].into_boxed_slice();
        // SAFETY: the map holds one entry, and the ring's tail is the last
        // field of the first entry.
        unsafe {
            let entry = &mut *entries.as_ptr();
            entry.set_addr(buffer.as_mut_ptr() as u64);
            entry.set_len(BUFFER_LEN as u32);
            entry.set_bid(0);
            (*BufRingEntry::tail(entries.as_ptr()).cast::<AtomicU16>()).store(1, Ordering::Release);
        }
        let reg = io_uring_buf_reg {
            ring_addr: entries.as_ptr() as u64,
            ring_entries: 1,
            bgid: BUFFER_GROUP,
            flags,
            resv: [0; 3],
        };
        if let Err(e) = register(
            ring_fd,
            io_uring_register_op::IORING_REGISTER_PBUF_RING,
            &reg,
        ) {
            // SAFETY: the map was not registered.
            unsafe { munmap(entries.as_ptr().cast(), size_of::<BufRingEntry>()) }?;
            return Err(e);
        }
        Ok(Self {
            ring_fd,
            entries,
            buffer,
        })
    }
}

impl Drop for BufRing {
    fn drop(&mut self) {
        let reg = io_uring_buf_reg {
            ring_addr: 0,
            ring_entries: 0,
            bgid: BUFFER_GROUP,
            flags: 0,
            resv: [0; 3],
        };
        register(
            self.ring_fd,
            io_uring_register_op::IORING_UNREGISTER_PBUF_RING,
            &reg,
        )
        .expect("unregister the buffer ring");
        // SAFETY: the kernel no longer refers to the map.
        unsafe { munmap(self.entries.as_ptr().cast(), size_of::<BufRingEntry>()) }
            .expect("unmap the buffer ring");
    }
}

/// Receive into a buffer the kernel selects from [`BUFFER_GROUP`].
struct RecvSelect {
    fd: RawFd,
    /// The buffer id and the `buffer_more` of the completion.
    selected: Option<(u16, bool)>,
}

unsafe impl IourOpCode for RecvSelect {
    type Control = ();

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        // A length of 0 receives up to the selected buffer's length.
        opcode::Recv::new(Fd(self.fd), ptr::null_mut(), 0)
            .buf_group(BUFFER_GROUP)
            .build()
            .flags(Flags::BUFFER_SELECT)
            .into()
    }

    unsafe fn set_result(&mut self, _: &mut Self::Control, _: &io::Result<usize>, extra: &Extra) {
        self.selected = Some((extra.buffer_id().unwrap(), extra.buffer_more().unwrap()));
    }
}

/// The fusion driver needs a polling half too; the test only pushes the op
/// on io_uring.
#[cfg(fusion)]
unsafe impl compio_driver::PollOpCode for RecvSelect {
    type Control = ();

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<compio_driver::Decision> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn operate(&mut self, _: &mut Self::Control) -> std::task::Poll<io::Result<usize>> {
        unreachable!("pre_submit never submits")
    }
}

fn push_and_wait<O: compio_driver::OpCode + 'static>(
    driver: &mut Proactor,
    op: O,
) -> BufResult<usize, O> {
    match driver.push(op) {
        PushEntry::Ready(res) => res,
        PushEntry::Pending(mut key) => loop {
            driver.poll(None).unwrap();
            match driver.pop(key) {
                PushEntry::Pending(k) => key = k,
                PushEntry::Ready(res) => break res,
            }
        },
    }
}

/// Receive `data` into the single 4 KiB buffer of a ring registered with
/// `flags`, and return the `buffer_more` of the completion.
fn recv_buffer_more(flags: u16, data: &[u8]) -> Option<bool> {
    let mut driver = Proactor::new().unwrap();
    if !driver.driver_type().is_iouring() {
        eprintln!("the driver is not io_uring, skipping test");
        return None;
    }
    let ring = match BufRing::new(&driver, flags) {
        Ok(ring) => ring,
        Err(e) => {
            // `IOU_PBUF_RING_INC` needs Linux 6.12.
            eprintln!("cannot register the buffer ring with flags {flags:#x} ({e}), skipping test");
            return None;
        }
    };

    let (rx, mut tx) = UnixStream::pair().unwrap();
    driver.attach(rx.as_raw_fd()).unwrap();
    tx.write_all(data).unwrap();

    let op = RecvSelect {
        fd: rx.as_raw_fd(),
        selected: None,
    };
    let BufResult(res, op) = push_and_wait(&mut driver, op);
    assert_eq!(res.unwrap(), data.len());
    let (buffer_id, more) = op.selected.unwrap();
    assert_eq!(buffer_id, 0);
    assert_eq!(&ring.buffer[..data.len()], data);

    drop(ring);
    Some(more)
}

#[test]
fn buffer_more_incremental() {
    // A short receive leaves most of the buffer to the kernel.
    if let Some(more) = recv_buffer_more(IOU_PBUF_RING_INC as _, b"hello") {
        assert!(more);
    }
}

#[test]
fn buffer_more_whole() {
    if let Some(more) = recv_buffer_more(0, b"hello") {
        assert!(!more);
    }
}

#[test]
fn buffer_pool_refuses_incremental() {
    let mut driver = ProactorBuilder::new()
        .buffer_pool_size(NonZeroU16::new(1).unwrap())
        .buffer_pool_flag(IOU_PBUF_RING_INC as _)
        .build()
        .unwrap();
    if !driver.driver_type().is_iouring() {
        eprintln!("the driver is not io_uring, skipping test");
        return;
    }
    let err = driver.buffer_pool().unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{err}");
}
