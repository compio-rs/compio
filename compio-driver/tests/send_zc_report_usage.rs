//! A zerocopy notification reports its usage in the sign bit of `res`, which
//! must not be read as an errno.

#![cfg(io_uring)]

use std::{
    io::{self, Read},
    net::{TcpListener, TcpStream},
    os::fd::AsRawFd,
};

use compio_buf::BufResult;
use compio_driver::{Extra, IourOpCode, OpEntry, Proactor, PushEntry};
use io_uring::{opcode, types::Fd};
use linux_raw_sys::io_uring::IORING_SEND_ZC_REPORT_USAGE;

const PAYLOAD: &[u8; 12] = b"Hello world!";

struct SendZcReportUsage {
    stream: TcpStream,
    sent: Option<io::Result<usize>>,
    notification: Option<bool>,
    copied: Option<bool>,
}

unsafe impl IourOpCode for SendZcReportUsage {
    type Control = ();

    fn create_entry(&mut self, _: &mut Self::Control) -> OpEntry {
        opcode::SendZc::new(
            Fd(self.stream.as_raw_fd()),
            PAYLOAD.as_ptr(),
            PAYLOAD.len() as _,
        )
        .zc_flags(IORING_SEND_ZC_REPORT_USAGE as _)
        .build()
        .into()
    }

    fn call_blocking(&mut self, _: &mut Self::Control) -> io::Result<usize> {
        Err(io::ErrorKind::Unsupported.into())
    }

    unsafe fn set_result(&mut self, _: &mut Self::Control, _: &io::Result<usize>, extra: &Extra) {
        self.notification = Some(extra.is_notification().unwrap());
        self.copied = Some(extra.zc_copied().unwrap());
    }

    unsafe fn push_multishot(&mut self, _: &mut Self::Control, res: io::Result<usize>, _: Extra) {
        self.sent = Some(res);
    }

    fn pop_multishot(&mut self, _: &mut Self::Control) -> Option<BufResult<usize, Extra>> {
        None
    }
}

/// The fusion driver needs a polling half too; the test only pushes the op
/// on io_uring.
#[cfg(fusion)]
unsafe impl compio_driver::PollOpCode for SendZcReportUsage {
    type Control = ();

    fn pre_submit(&mut self, _: &mut Self::Control) -> io::Result<compio_driver::Decision> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn operate(&mut self, _: &mut Self::Control) -> std::task::Poll<io::Result<usize>> {
        unreachable!("pre_submit never submits")
    }
}

#[test]
fn send_zc_report_usage() {
    let mut driver = Proactor::new().unwrap();
    if !driver.driver_type().is_iouring() {
        eprintln!("the driver is not io_uring, skipping test");
        return;
    }

    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = server.local_addr().unwrap();
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = server.accept().unwrap();
        let mut buffer = [0u8; PAYLOAD.len()];
        stream.read_exact(&mut buffer).unwrap();
        assert_eq!(&buffer, PAYLOAD);
    });

    let op = SendZcReportUsage {
        stream: TcpStream::connect(addr).unwrap(),
        sent: None,
        notification: None,
        copied: None,
    };
    let BufResult(res, op) = match driver.push(op) {
        PushEntry::Ready(res) => res,
        PushEntry::Pending(mut key) => loop {
            driver.poll(None).unwrap();
            match driver.pop(key) {
                PushEntry::Pending(k) => key = k,
                PushEntry::Ready(res) => break res,
            }
        },
    };

    let Some(sent) = op.sent else {
        // Without a notification, the kernel rejected the send: `SendZc` needs
        // Linux 6.0 and `IORING_SEND_ZC_REPORT_USAGE` needs Linux 6.2.
        eprintln!("zerocopy send with usage report is unsupported ({res:?}), skipping test");
        return;
    };
    assert_eq!(sent.unwrap(), PAYLOAD.len());
    assert_eq!(op.notification, Some(true));
    assert_eq!(res.unwrap(), 0);
    // Loopback never sends in place, so the kernel always reports a copy.
    assert_eq!(op.copied, Some(true));

    if let Err(e) = handle.join() {
        std::panic::resume_unwind(e)
    }
}
