use std::{
    fs::File,
    future::Future,
    io,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use compio_driver::SharedFd;
use compio_runtime::{
    Runtime,
    time::{Sleep, sleep},
};
use rustix::io::{Errno, read};

use super::{
    super::{TermType, parse::Parser},
    ReadState,
};

const READ_INTERVAL: Duration = Duration::from_millis(8);
const BUFFER_SIZE: usize = 68;

pub(crate) struct Input {
    terminal: SharedFd<File>,
    buffer: [u8; BUFFER_SIZE],
    timer: Option<Sleep>,
}

impl Input {
    pub(crate) fn new(terminal: SharedFd<File>, _: Runtime, _: TermType) -> io::Result<Self> {
        Ok(Self {
            terminal,
            buffer: [0; BUFFER_SIZE],
            timer: None,
        })
    }

    pub(crate) fn poll_read(
        &mut self,
        cx: &mut Context<'_>,
        parser: &mut Parser,
    ) -> Poll<io::Result<ReadState>> {
        if let Some(timer) = &mut self.timer {
            if Pin::new(timer).poll(cx).is_pending() {
                return Poll::Pending;
            }
            self.timer = None;
        }

        loop {
            match read(&self.terminal, &mut self.buffer) {
                Ok(0) => return Poll::Ready(Ok(ReadState::Closed)),
                Ok(len) => {
                    parser.advance(&self.buffer[..len]);
                    return Poll::Ready(Ok(ReadState::Data));
                }
                Err(Errno::INTR) => {}
                Err(Errno::AGAIN) => {
                    let mut timer = sleep(READ_INTERVAL);
                    if Pin::new(&mut timer).poll(cx).is_ready() {
                        continue;
                    }
                    self.timer = Some(timer);
                    return Poll::Pending;
                }
                Err(error) => return Poll::Ready(Err(error.into())),
            }
        }
    }
}
