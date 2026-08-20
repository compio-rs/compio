use std::{
    fs::{File, OpenOptions},
    future::Future,
    io::{self, IsTerminal},
    os::unix::fs::OpenOptionsExt,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use compio_driver::SharedFd;
use compio_runtime::{
    Runtime,
    time::{Sleep, sleep},
};
use futures_core::Stream;
use rustix::{
    fs::{OFlags, fcntl_getfl, fcntl_setfl},
    process::Signal,
    termios::{LocalModes, tcgetattr, tcgetwinsize},
};

use self::{
    input::{Input, ReadState},
    parse::Parser,
};
use crate::event::{Event, InternalEvent};

mod input;
mod parse;

const ESCAPE_TIMEOUT: Duration = Duration::from_millis(20);
#[derive(Clone, Copy)]
enum TermType {
    Tty,
    Stdin,
}

impl TermType {
    const fn path(self) -> &'static str {
        match self {
            Self::Tty => "/dev/tty",
            Self::Stdin => "/dev/stdin",
        }
    }
}

type ResizeListener = Pin<Box<dyn Future<Output = io::Result<()>>>>;

pub(crate) struct EventSource {
    input: Input,
    terminal: SharedFd<File>,
    parser: Parser,
    escape_timer: Option<Sleep>,
    resize: ResizeListener,
    input_closed: bool,
}

impl EventSource {
    pub(crate) fn new() -> io::Result<Self> {
        let runtime = Runtime::try_current().ok_or_else(|| {
            io::Error::other("EventStream must be created inside a Compio runtime")
        })?;

        let term_type = if io::stdin().is_terminal() {
            TermType::Stdin
        } else {
            TermType::Tty
        };
        let mut options = OpenOptions::new();
        options
            .read(true)
            .custom_flags(OFlags::NONBLOCK.bits() as i32);
        let terminal = SharedFd::new(options.open(term_type.path())?);
        let termios = tcgetattr(&terminal)?;

        // liburing issue #1185 found that setting O_NONBLOCK at open time was
        // not sufficient for tty multishot reads. Repeat it with F_SETFL.
        let flags = fcntl_getfl(&terminal)?;
        fcntl_setfl(&terminal, flags | OFlags::NONBLOCK)?;

        let input = Input::new(terminal.clone(), runtime, term_type)?;

        Ok(Self {
            input,
            terminal,
            parser: Parser::new(!termios.local_modes.contains(LocalModes::ICANON)),
            escape_timer: None,
            resize: resize_listener(),
            input_closed: false,
        })
    }

    fn update_escape_timer(&mut self, cx: &mut Context<'_>) {
        if !self.parser.has_ambiguous_escape() {
            self.escape_timer = None;
            return;
        }
        if self.escape_timer.is_none() {
            let mut timer = sleep(ESCAPE_TIMEOUT);
            let _ = Pin::new(&mut timer).poll(cx);
            self.escape_timer = Some(timer);
        }
    }

    fn poll_escape_timer(&mut self, cx: &mut Context<'_>) {
        let expired = self
            .escape_timer
            .as_mut()
            .is_some_and(|timer| Pin::new(timer).poll(cx).is_ready());
        if expired {
            self.escape_timer = None;
            self.parser.resolve_escape();
        }
    }

    fn poll_resize(&mut self, cx: &mut Context<'_>) -> Poll<Option<io::Result<InternalEvent>>> {
        let Poll::Ready(result) = self.resize.as_mut().poll(cx) else {
            return Poll::Pending;
        };

        self.resize = resize_listener();
        let _ = self.resize.as_mut().poll(cx);
        if let Err(error) = result {
            return Poll::Ready(Some(Err(error)));
        }
        let size = match tcgetwinsize(&self.terminal) {
            Ok(size) => size,
            Err(error) => return Poll::Ready(Some(Err(error.into()))),
        };
        Poll::Ready(Some(Ok(InternalEvent::Event(Event::Resize(
            size.ws_col,
            size.ws_row,
        )))))
    }
}

impl Stream for EventSource {
    type Item = io::Result<InternalEvent>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            let this = self.as_mut().get_mut();
            if let Some(event) = this.parser.next() {
                return Poll::Ready(Some(Ok(event)));
            }

            this.poll_escape_timer(cx);
            if let Some(event) = this.parser.next() {
                return Poll::Ready(Some(Ok(event)));
            }

            if !this.input_closed {
                match this.input.poll_read(cx, &mut this.parser) {
                    Poll::Ready(Ok(ReadState::Data)) => {
                        this.update_escape_timer(cx);
                        continue;
                    }
                    Poll::Ready(Ok(ReadState::Closed)) => {
                        this.input_closed = true;
                        this.parser.resolve_escape();
                        continue;
                    }
                    Poll::Ready(Err(error)) => return Poll::Ready(Some(Err(error))),
                    Poll::Pending => {}
                }
            }

            if let Poll::Ready(event) = this.poll_resize(cx) {
                return Poll::Ready(event);
            }
            if this.input_closed {
                return Poll::Ready(None);
            }

            this.update_escape_timer(cx);
            return Poll::Pending;
        }
    }
}

fn resize_listener() -> ResizeListener {
    Box::pin(compio_signal::unix::signal(Signal::WINCH.as_raw()))
}
