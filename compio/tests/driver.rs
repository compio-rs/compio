use std::{io, mem::MaybeUninit, net::SocketAddr, time::Duration};

use compio::{
    buf::{arrayvec::ArrayVec, BufResult, IntoInner},
    driver::{
        op::{ReadAt, Recv, Send},
        AsRawFd, Entry, Proactor,
    },
    fs::File,
    net::UdpSocket,
};
use compio_driver::PushEntry;

#[test]
fn udp_io() {
    let first_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let second_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();

    // bind sockets
    let socket = UdpSocket::bind(first_addr).unwrap();
    let first_addr = socket.local_addr().unwrap();
    let other_socket = UdpSocket::bind(second_addr).unwrap();
    let second_addr = other_socket.local_addr().unwrap();

    // connect sockets
    socket.connect(second_addr).unwrap();
    other_socket.connect(first_addr).unwrap();

    let mut driver = Proactor::new().unwrap();
    driver.attach(socket.as_raw_fd()).unwrap();
    driver.attach(other_socket.as_raw_fd()).unwrap();

    // operations need to wait
    let mut need_wait = 0;

    // write data
    let op_write = Send::new(socket.as_raw_fd(), "hello world");
    let key_write = match driver.push(op_write) {
        PushEntry::Pending(key) => {
            need_wait += 1;
            key
        }
        PushEntry::Ready(res) => {
            res.unwrap();
            usize::MAX
        }
    };

    // read data
    let buf = Vec::with_capacity(32);
    let op_read = Recv::new(other_socket.as_raw_fd(), buf);
    let mut n_bytes = 0;
    let mut buf = MaybeUninit::uninit();
    let key_read = match driver.push(op_read) {
        PushEntry::Pending(key) => {
            need_wait += 1;
            key
        }
        PushEntry::Ready(BufResult(res, op)) => {
            n_bytes = res.unwrap();
            buf.write(op.into_inner());
            usize::MAX
        }
    };

    let mut entries = ArrayVec::<Entry, 2>::new();

    while entries.len() < need_wait {
        driver.poll(None, &mut entries).unwrap();
    }

    for BufResult(res, op) in driver.pop(&mut entries.into_iter()) {
        let key = op.user_data();
        if key == key_write {
            res.unwrap();
        } else if key == key_read {
            n_bytes = res.unwrap();
            buf.write(unsafe { op.into_op::<Recv<Vec<u8>>>() }.into_inner());
        }
    }

    let mut buf = unsafe { buf.assume_init() };
    unsafe { buf.set_len(n_bytes) };
    assert_eq!(buf, b"hello world");
}

#[test]
fn cancel_before_poll() {
    let mut driver = Proactor::new().unwrap();

    let file = File::open("Cargo.toml").unwrap();
    driver.attach(file.as_raw_fd()).unwrap();

    driver.cancel(0);

    let op = ReadAt::new(file.as_raw_fd(), 0, Vec::with_capacity(8));
    let BufResult(res, _) = match driver.push(op) {
        PushEntry::Ready(res) => res,
        PushEntry::Pending(key) => {
            let mut entries = ArrayVec::<Entry, 1>::new();
            driver.poll(None, &mut entries).unwrap();
            driver
                .pop(&mut entries.into_iter())
                .next()
                .unwrap()
                .map_buffer(|op| {
                    assert_eq!(op.user_data(), key);
                    unsafe { op.into_op() }
                })
        }
    };

    assert!(res.is_ok() || res.unwrap_err().kind() == io::ErrorKind::TimedOut);
}

#[test]
fn timeout() {
    let mut driver = Proactor::new().unwrap();

    let mut entries = ArrayVec::<Entry, 1>::new();
    let err = driver
        .poll(Some(Duration::from_secs(1)), &mut entries)
        .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::TimedOut);
}

#[test]
fn register_multiple() {
    const TASK_LEN: usize = 5;

    let mut driver = Proactor::new().unwrap();

    let file = File::open("Cargo.toml").unwrap();
    driver.attach(file.as_raw_fd()).unwrap();

    let mut need_wait = 0;

    for _i in 0..TASK_LEN {
        match driver.push(ReadAt::new(file.as_raw_fd(), 0, Vec::with_capacity(1024))) {
            PushEntry::Pending(_) => need_wait += 1,
            PushEntry::Ready(res) => {
                res.unwrap();
            }
        }
    }

    let mut entries = ArrayVec::<Entry, TASK_LEN>::new();
    while entries.len() < need_wait {
        driver.poll(None, &mut entries).unwrap();
    }
}
