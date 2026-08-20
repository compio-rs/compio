use std::{
    fs::File,
    io,
    task::{Context, Poll},
};

use compio_driver::SharedFd;
use compio_runtime::Runtime;

use super::{
    super::{TermType, parse::Parser},
    ReadState,
    multishot::Input as MultishotInput,
    poll::Input as PollInput,
};

pub(crate) enum Input {
    Multishot(MultishotInput),
    Poll(PollInput),
}

impl Input {
    pub(crate) fn new(
        terminal: SharedFd<File>,
        runtime: Runtime,
        term_type: TermType,
    ) -> io::Result<Self> {
        match term_type {
            TermType::Stdin => {
                MultishotInput::new(terminal, runtime, term_type).map(Self::Multishot)
            }
            TermType::Tty => PollInput::new(terminal, runtime, term_type).map(Self::Poll),
        }
    }

    pub(crate) fn poll_read(
        &mut self,
        cx: &mut Context<'_>,
        parser: &mut Parser,
    ) -> Poll<io::Result<ReadState>> {
        match self {
            Self::Multishot(input) => input.poll_read(cx, parser),
            Self::Poll(input) => input.poll_read(cx, parser),
        }
    }
}
