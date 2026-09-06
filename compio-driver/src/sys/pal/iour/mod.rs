#[cfg(feature = "once_cell_try")]
use std::sync::OnceLock;
use std::{io, os::fd::AsRawFd, sync::LazyLock};

use io_uring::{
    squeue::{Entry, Entry128},
    types::Fd,
};
use linux_raw_sys::io_uring::{
    IORING_ACCEPT_MULTISHOT, IORING_ACCEPT_POLL_FIRST, IORING_RECV_MULTISHOT,
    IORING_RECVSEND_POLL_FIRST, io_uring_sqe,
};
#[cfg(not(feature = "once_cell_try"))]
use once_cell::sync::OnceCell as OnceLock;
use rustix::net::{AddressFamily, SendFlags, SocketAddrUnix, SocketFlags, SocketType};

pub struct OpDesc {
    code: u8,
    ioprio: u16,
}

impl From<u8> for OpDesc {
    fn from(value: u8) -> Self {
        Self {
            code: value,
            ioprio: 0,
        }
    }
}

impl From<(u8, u16)> for OpDesc {
    fn from(value: (u8, u16)) -> Self {
        Self {
            code: value.0,
            ioprio: value.1,
        }
    }
}

impl From<&Entry> for OpDesc {
    fn from(value: &Entry) -> Self {
        let sqe = value as *const Entry as *const io_uring_sqe;
        unsafe {
            Self {
                code: (*sqe).opcode,
                ioprio: (*sqe).ioprio,
            }
        }
    }
}

impl From<&Entry128> for OpDesc {
    fn from(value: &Entry128) -> Self {
        let sqe = value as *const Entry128 as *const io_uring_sqe;
        unsafe {
            Self {
                code: (*sqe).opcode,
                ioprio: (*sqe).ioprio,
            }
        }
    }
}

pub fn is_op_supported(desc: impl Into<OpDesc>) -> bool {
    let desc = desc.into();

    static PROBE: OnceLock<io_uring::Probe> = OnceLock::new();

    let op_supported = PROBE
        .get_or_try_init(|| {
            let mut probe = io_uring::Probe::new();

            io_uring::IoUring::new(2)?
                .submitter()
                .register_probe(&mut probe)?;

            std::io::Result::Ok(probe)
        })
        .map(|probe| probe.is_supported(desc.code))
        .unwrap_or_default();
    match desc.code {
        io_uring::opcode::Accept::CODE => op_supported && is_accept_ioprio_supported(desc.ioprio),
        io_uring::opcode::Recv::CODE => op_supported && is_recv_ioprio_supported(desc.ioprio),
        io_uring::opcode::RecvMsg::CODE => op_supported && is_recvmsg_ioprio_supported(desc.ioprio),
        _ => op_supported,
    }
}

fn is_accept_ioprio_supported(ioprio: u16) -> bool {
    fn is_supported(ioprio: u16) -> io::Result<bool> {
        let server = rustix::net::socket_with(
            AddressFamily::UNIX,
            SocketType::STREAM,
            SocketFlags::CLOEXEC,
            None,
        )?;
        rustix::net::bind(&server, &SocketAddrUnix::new_unnamed())?;
        let addr = rustix::net::getsockname(&server)?;

        let mut ring = io_uring::IoUring::new(2)?;
        let mut accept_addr: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
        let mut accept_addrlen = 0;
        let mut entry = io_uring::opcode::Accept::new(
            Fd(server.as_raw_fd()),
            &mut accept_addr as *mut _ as _,
            &mut accept_addrlen,
        )
        .flags(libc::SOCK_CLOEXEC)
        .build();
        let sqe = &raw mut entry as *mut io_uring_sqe;
        unsafe {
            (*sqe).ioprio |= ioprio;
            ring.submission().push(&entry).map_err(io::Error::other)?;
        }

        let client = rustix::net::socket_with(
            AddressFamily::UNIX,
            SocketType::STREAM,
            SocketFlags::CLOEXEC,
            None,
        )?;
        rustix::net::connect(&client, &addr)?;
        ring.submit_and_wait(1).map_err(io::Error::other)?;

        let cqe = ring
            .completion()
            .next()
            .ok_or_else(|| io::Error::other("No completion event received"))?;
        let result = cqe.result();
        if result < 0 {
            Ok(false)
        } else {
            unsafe { libc::close(result) };
            Ok(true)
        }
    }

    static PROBE: [(u16, LazyLock<bool>); 2] = [
        (
            IORING_ACCEPT_POLL_FIRST as u16,
            LazyLock::new(|| is_supported(IORING_ACCEPT_POLL_FIRST as u16).unwrap_or_default()),
        ),
        (
            IORING_ACCEPT_MULTISHOT as u16,
            LazyLock::new(|| is_supported(IORING_ACCEPT_MULTISHOT as u16).unwrap_or_default()),
        ),
    ];

    let mut res = true;
    for (single_ioprio, probe) in &PROBE {
        if (ioprio & single_ioprio) != 0 {
            res &= **probe;
        }
    }
    res
}

fn is_recv_ioprio_supported(ioprio: u16) -> bool {
    fn is_supported(ioprio: u16) -> io::Result<bool> {
        let (server, client) = rustix::net::socketpair(
            AddressFamily::UNIX,
            SocketType::STREAM,
            SocketFlags::CLOEXEC,
            None,
        )?;

        let mut ring = io_uring::IoUring::new(2)?;

        let mut buffer = [0u8; 1];
        let mut entry = io_uring::opcode::Recv::new(
            Fd(server.as_raw_fd()),
            buffer.as_mut_ptr(),
            buffer.len() as _,
        )
        .build();
        let sqe = &raw mut entry as *mut io_uring_sqe;
        unsafe {
            (*sqe).ioprio |= ioprio;
            ring.submission().push(&entry).map_err(io::Error::other)?;
        }

        rustix::net::send(&client, &[1u8], SendFlags::empty())?;
        ring.submit_and_wait(1).map_err(io::Error::other)?;

        let cqe = ring
            .completion()
            .next()
            .ok_or_else(|| io::Error::other("No completion event received"))?;
        Ok(cqe.result() >= 0)
    }

    static PROBE: [(u16, LazyLock<bool>); 2] = [
        (
            IORING_RECVSEND_POLL_FIRST as u16,
            LazyLock::new(|| is_supported(IORING_RECVSEND_POLL_FIRST as u16).unwrap_or_default()),
        ),
        (
            IORING_RECV_MULTISHOT as u16,
            LazyLock::new(|| is_supported(IORING_RECV_MULTISHOT as u16).unwrap_or_default()),
        ),
    ];

    let mut res = true;
    for (single_ioprio, probe) in &PROBE {
        if (ioprio & single_ioprio) != 0 {
            res &= **probe;
        }
    }
    res
}

fn is_recvmsg_ioprio_supported(ioprio: u16) -> bool {
    fn is_supported(ioprio: u16) -> io::Result<bool> {
        let (server, client) = rustix::net::socketpair(
            AddressFamily::UNIX,
            SocketType::STREAM,
            SocketFlags::CLOEXEC,
            None,
        )?;

        let mut ring = io_uring::IoUring::new(2)?;

        let mut buffer = [0u8; 1];
        let mut addr: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
        let mut msghdr: libc::msghdr = unsafe { std::mem::zeroed() };
        let mut iov: libc::iovec = unsafe { std::mem::zeroed() };
        iov.iov_base = buffer.as_mut_ptr() as *mut _;
        iov.iov_len = buffer.len();
        msghdr.msg_iov = &mut iov;
        msghdr.msg_name = &mut addr as *mut _ as *mut _;
        msghdr.msg_namelen = std::mem::size_of_val(&addr) as _;

        let mut entry = io_uring::opcode::RecvMsg::new(Fd(server.as_raw_fd()), &mut msghdr).build();
        let sqe = &raw mut entry as *mut io_uring_sqe;
        unsafe {
            (*sqe).ioprio |= ioprio;
            ring.submission().push(&entry).map_err(io::Error::other)?;
        }

        rustix::net::send(&client, &[1u8], SendFlags::empty())?;
        ring.submit_and_wait(1).map_err(io::Error::other)?;

        let cqe = ring
            .completion()
            .next()
            .ok_or_else(|| io::Error::other("No completion event received"))?;
        Ok(cqe.result() >= 0)
    }

    static PROBE: [(u16, LazyLock<bool>); 2] = [
        (
            IORING_RECVSEND_POLL_FIRST as u16,
            LazyLock::new(|| is_supported(IORING_RECVSEND_POLL_FIRST as u16).unwrap_or_default()),
        ),
        (
            IORING_RECV_MULTISHOT as u16,
            LazyLock::new(|| is_supported(IORING_RECV_MULTISHOT as u16).unwrap_or_default()),
        ),
    ];

    let mut res = true;
    for (single_ioprio, probe) in &PROBE {
        if (ioprio & single_ioprio) != 0 {
            res &= **probe;
        }
    }
    res
}

pub(crate) fn set_poll_first(mut entry: Entry, flag: bool) -> Entry {
    if flag {
        let ioprio = match entry.get_opcode() as u8 {
            io_uring::opcode::Accept::CODE
                if is_accept_ioprio_supported(IORING_ACCEPT_POLL_FIRST as u16) =>
            {
                IORING_ACCEPT_POLL_FIRST
            }
            io_uring::opcode::Recv::CODE
                if is_recv_ioprio_supported(IORING_RECVSEND_POLL_FIRST as u16) =>
            {
                IORING_RECVSEND_POLL_FIRST
            }
            io_uring::opcode::RecvMsg::CODE
                if is_recvmsg_ioprio_supported(IORING_RECVSEND_POLL_FIRST as u16) =>
            {
                IORING_RECVSEND_POLL_FIRST
            }
            _ => 0,
        };
        let sqe = &raw mut entry as *mut io_uring_sqe;
        unsafe {
            (*sqe).ioprio |= ioprio as u16;
        }
    }
    entry
}
