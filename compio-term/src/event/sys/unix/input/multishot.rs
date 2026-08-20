use std::{
    fs::File,
    io,
    pin::Pin,
    task::{Context, Poll},
};

use compio_driver::{BufferRef, SharedFd, op::ReadMulti};
use compio_runtime::{Runtime, SubmitMultiFactory, SubmitMultiManaged, SubmitMultiStream};
use futures_core::Stream;

use super::{
    super::{TermType, parse::Parser},
    ReadState,
};

type ReadOp = ReadMulti<SharedFd<File>>;
type ReadStream = SubmitMultiStream<ReadFactory>;

struct ReadFactory {
    terminal: SharedFd<File>,
    runtime: Runtime,
}

impl SubmitMultiFactory for ReadFactory {
    type Buffer = BufferRef;
    type Op = ReadOp;

    fn create(&mut self) -> io::Result<SubmitMultiManaged<Self::Op, Self::Buffer>> {
        let pool = self.runtime.buffer_pool()?;
        let op = ReadMulti::new(self.terminal.clone(), &pool, 0)?;
        Ok(self.runtime.submit_multi(op).into_managed(pool))
    }
}

pub(crate) struct Input {
    stream: ReadStream,
}

impl Input {
    pub(crate) fn new(terminal: SharedFd<File>, runtime: Runtime, _: TermType) -> io::Result<Self> {
        runtime.buffer_pool()?;
        let factory = ReadFactory { terminal, runtime };
        let stream = SubmitMultiStream::new(factory);
        Ok(Self { stream })
    }

    pub(crate) fn poll_read(
        &mut self,
        cx: &mut Context<'_>,
        parser: &mut Parser,
    ) -> Poll<io::Result<ReadState>> {
        match Pin::new(&mut self.stream).poll_next(cx) {
            Poll::Ready(Some(Ok(buffer))) => {
                parser.advance(&buffer);
                Poll::Ready(Ok(ReadState::Data))
            }
            Poll::Ready(Some(Err(error))) => Poll::Ready(Err(error)),
            Poll::Ready(None) => Poll::Ready(Ok(ReadState::Closed)),
            Poll::Pending => Poll::Pending,
        }
    }
}
