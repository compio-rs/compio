#![cfg(linux_all)]

use std::{
    io::Read,
    net::{TcpListener, TcpStream},
    os::fd::AsRawFd,
};

use compio_buf::BufResult;
use compio_driver::{Proactor, PushEntry, SharedFd, op::SendZc};
use rustix::net::SendFlags;

#[test]
fn send_zc() {
    let mut driver = Proactor::new().unwrap();

    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = server.local_addr().unwrap();

    let handle = std::thread::spawn(move || {
        let (mut stream, _) = server.accept().unwrap();
        let mut buffer = [0u8; 12];
        stream.read_exact(&mut buffer).unwrap();
        assert_eq!(&buffer, b"Hello world!");
    });

    let stream = TcpStream::connect(addr).unwrap();
    let stream = socket2::Socket::from(stream);
    if driver.driver_type().is_polling() {
        stream.set_nonblocking(true).unwrap();
    }
    let stream = SharedFd::new(stream);

    driver.attach(stream.as_raw_fd()).unwrap();

    let buffer: &'static [u8; 12] = b"Hello world!";
    let op = SendZc::new(stream.clone(), buffer, SendFlags::empty());
    let res = match driver.push(op) {
        PushEntry::Ready(BufResult(res, _)) => res.unwrap(),
        PushEntry::Pending(mut key) => {
            let mut len = None;
            while len.is_none() {
                driver.poll(None).unwrap();
                if let Some(BufResult(res, _)) = driver.pop_multishot(&key) {
                    len = Some(res.unwrap());
                }
                match driver.pop(key) {
                    PushEntry::Pending(k) => key = k,
                    PushEntry::Ready(BufResult(res, _)) => {
                        if len.is_none() {
                            len = Some(res.unwrap())
                        } else {
                            res.unwrap();
                        }
                        break;
                    }
                }
            }
            len.unwrap()
        }
    };
    assert_eq!(res, 12);
    if let Err(e) = handle.join() {
        std::panic::resume_unwind(e)
    }
}
