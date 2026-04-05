use std::{
    io::{self, Write},
    net::{TcpListener, TcpStream, UdpSocket},
    os::{fd::AsRawFd, unix::net::UnixStream},
    ptr::NonNull,
    sync::LazyLock,
};

use io_uring::{
    IoUring,
    opcode::{AcceptMulti, RecvMsgMulti, RecvMulti},
    squeue::EntryMarker,
    types::Fd,
};

use crate::syscall;

#[repr(C)]
struct RawSqeHeader {
    pub opcode: u8,
    pub flags: u8,
    pub ioprio: u16,
    pub fd: i32,
    pub off: u64,
    pub addr: u64,
    pub len: u32,
}

#[repr(C)]
struct RecvMsgOut {
    pub namelen: u32,
    pub controllen: u32,
    pub payloadlen: u32,
    pub flags: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectResult {
    NotDetected(u8),
    Supported,
    NotSupported,
}

macro_rules! detect_branch {
    ($header:ident, $field:ident, $flag:ident, $support:ident) => {
        if ($header.$field & $flag) != 0 {
            if *$support {
                DetectResult::Supported
            } else {
                DetectResult::NotSupported
            }
        } else {
            DetectResult::NotDetected($header.opcode)
        }
    };
}

pub fn detect<E: EntryMarker>(sqe: &E) -> DetectResult {
    const IORING_ACCEPT_MULTISHOT: u16 = 0x1;
    const IORING_RECV_MULTISHOT: u16 = 0x2;

    // SAFETY: the same layout.
    let header = unsafe { std::mem::transmute::<&E, &RawSqeHeader>(sqe) };
    match header.opcode {
        RecvMulti::CODE => detect_branch!(
            header,
            ioprio,
            IORING_RECV_MULTISHOT,
            SUPPORT_RECV_MULTISHOT
        ),
        RecvMsgMulti::CODE => detect_branch!(
            header,
            ioprio,
            IORING_RECV_MULTISHOT,
            SUPPORT_RECVMSG_MULTISHOT
        ),
        AcceptMulti::CODE => detect_branch!(
            header,
            ioprio,
            IORING_ACCEPT_MULTISHOT,
            SUPPORT_ACCEPT_MULTISHOT
        ),
        _ => DetectResult::NotDetected(header.opcode),
    }
}

static SUPPORT_ACCEPT_MULTISHOT: LazyLock<bool> =
    LazyLock::new(|| detect_accept_multishot().is_ok());
static SUPPORT_RECV_MULTISHOT: LazyLock<bool> = LazyLock::new(|| detect_recv_multishot().is_ok());
static SUPPORT_RECVMSG_MULTISHOT: LazyLock<bool> =
    LazyLock::new(|| detect_recvmsg_multishot().is_ok());

fn check_result(ring: &mut IoUring) -> io::Result<i32> {
    let cqe = loop {
        if let Some(cqe) = ring.completion().next() {
            break cqe;
        } else {
            ring.submit_and_wait(1)?;
        }
    };
    let res = cqe.result();
    if res < 0 {
        Err(io::Error::from_raw_os_error(-res))
    } else {
        Ok(res)
    }
}

fn detect_accept_multishot() -> io::Result<()> {
    let mut ring = IoUring::new(4)?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let local_addr = listener.local_addr()?;
    let op = AcceptMulti::new(Fd(listener.as_raw_fd())).build();
    unsafe { ring.submission().push(&op).map_err(io::Error::other)? };
    ring.submit()?;
    let _client = TcpStream::connect(local_addr)?;
    let res = check_result(&mut ring)?;
    syscall!(libc::close(res))?;
    Ok(())
}

fn detect_recv_multishot() -> io::Result<()> {
    let mut ring = IoUring::new(4)?;
    let (mut tx, rx) = UnixStream::pair()?;
    let mut buffer = [0u8; 4];
    let pool = unsafe {
        super::buffer_pool::BufControl::new_inner(
            ring.submitter(),
            &[Some(NonNull::new(buffer.as_mut_ptr().cast()).unwrap())],
            buffer.len() as _,
            0,
        )?
    };
    let op = RecvMulti::new(Fd(rx.as_raw_fd()), pool.buffer_group()).build();
    unsafe { ring.submission().push(&op).map_err(io::Error::other)? };
    ring.submit()?;
    tx.write_all(b"ping")?;
    check_result(&mut ring)?;
    Ok(())
}

fn detect_recvmsg_multishot() -> io::Result<()> {
    const SOCKADDR_STORAGE_SIZE: usize = size_of::<libc::sockaddr_storage>();
    const RECVMSG_HEADER_SIZE: usize = size_of::<RecvMsgOut>();

    let mut ring = IoUring::new(4)?;
    let rx = UdpSocket::bind("127.0.0.1:0")?;
    let mut buffer = [0u8; const { RECVMSG_HEADER_SIZE + SOCKADDR_STORAGE_SIZE + 4 }];
    let pool = unsafe {
        super::buffer_pool::BufControl::new_inner(
            ring.submitter(),
            &[Some(NonNull::new(buffer.as_mut_ptr().cast()).unwrap())],
            buffer.len() as _,
            0,
        )?
    };
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_controllen = 0;
    msg.msg_namelen = SOCKADDR_STORAGE_SIZE as _;
    let op = RecvMsgMulti::new(Fd(rx.as_raw_fd()), &msg, pool.buffer_group()).build();
    unsafe { ring.submission().push(&op).map_err(io::Error::other)? };
    ring.submit()?;
    let tx = UdpSocket::bind("127.0.0.1:0")?;
    tx.send_to(b"ping", rx.local_addr()?)?;
    check_result(&mut ring)?;
    Ok(())
}
