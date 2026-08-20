use std::{
    future::Future,
    io,
    os::windows::io::{AsRawHandle, RawHandle},
    pin::Pin,
    task::{Context, Poll},
};

use compio_buf::BufResult;
use compio_driver::{OpCode, OpType};
use compio_runtime::{Runtime, Submit};
use futures_core::Stream;
use windows_sys::Win32::System::Console::{
    GetConsoleMode, GetNumberOfConsoleInputEvents, INPUT_RECORD, ReadConsoleInputW,
};

use self::parse::Parser;
use crate::event::InternalEvent;

mod parse;

pub(crate) struct EventSource {
    handle: RawHandle,
    read: Option<Submit<ReadInput>>,
    parser: Parser,
}

impl EventSource {
    pub(crate) fn new() -> io::Result<Self> {
        if Runtime::try_current().is_none() {
            return Err(io::Error::other(
                "EventStream must be created inside a Compio runtime",
            ));
        }

        let handle = io::stdin().as_raw_handle();
        let mut mode = 0;
        if unsafe { GetConsoleMode(handle, &mut mode) } == 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(Self {
            handle,
            read: None,
            parser: Parser::default(),
        })
    }
}

impl Stream for EventSource {
    type Item = io::Result<InternalEvent>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.as_mut().get_mut();
        if this.read.is_none() {
            this.read = Some(compio_runtime::submit(ReadInput::new(this.handle)));
        }

        let Poll::Ready(BufResult(result, input)) =
            Pin::new(this.read.as_mut().expect("read future was created")).poll(cx)
        else {
            return Poll::Pending;
        };
        this.read = None;
        if let Err(error) = result {
            return Poll::Ready(Some(Err(error)));
        }
        match this.parser.parse(input.record) {
            Ok(Some(event)) => Poll::Ready(Some(Ok(InternalEvent::Event(event)))),
            Ok(None) => Poll::Ready(Some(Ok(InternalEvent::Ignored))),
            Err(error) => Poll::Ready(Some(Err(error))),
        }
    }
}

struct ReadInput {
    handle: RawHandle,
    record: INPUT_RECORD,
}

impl ReadInput {
    fn new(handle: RawHandle) -> Self {
        Self {
            handle,
            record: INPUT_RECORD::default(),
        }
    }
}

// Compio waits for the console handle before calling `operate`. The operation
// owns the record storage until it completes.
unsafe impl OpCode for ReadInput {
    type Control = ();

    fn op_type(&self, _: &Self::Control) -> OpType {
        OpType::Event(self.handle as _)
    }

    unsafe fn operate(
        &mut self,
        _: &mut Self::Control,
        _: *mut windows_sys::Win32::System::IO::OVERLAPPED,
    ) -> Poll<io::Result<usize>> {
        let mut available = 0;
        if unsafe { GetNumberOfConsoleInputEvents(self.handle, &mut available) } == 0 {
            return Poll::Ready(Err(io::Error::last_os_error()));
        }
        if available == 0 {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "console input was consumed by another reader",
            )));
        }

        let mut read = 0;
        if unsafe { ReadConsoleInputW(self.handle, &mut self.record, 1, &mut read) } == 0 {
            Poll::Ready(Err(io::Error::last_os_error()))
        } else if read == 1 {
            Poll::Ready(Ok(1))
        } else {
            Poll::Ready(Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "console input returned no event",
            )))
        }
    }
}
