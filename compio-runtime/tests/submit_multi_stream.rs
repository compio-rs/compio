use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use compio_driver::{BufferRef, op::ReadMulti};
use compio_runtime::{SubmitMultiFactory, SubmitMultiManaged, SubmitMultiStream};
use futures_util::{Stream, task::noop_waker_ref};

type Op = ReadMulti<std::fs::File>;

struct CustomFactory;

impl SubmitMultiFactory for CustomFactory {
    type Buffer = BufferRef;
    type Op = Op;

    fn create(&mut self) -> io::Result<SubmitMultiManaged<Self::Op, Self::Buffer>> {
        Err(io::Error::other("custom factory error"))
    }
}

fn poll_error<S>(stream: Pin<&mut S>) -> io::Error
where
    S: Stream<Item = io::Result<BufferRef>>,
{
    let mut cx = Context::from_waker(noop_waker_ref());
    match stream.poll_next(&mut cx) {
        Poll::Ready(Some(Err(error))) => error,
        _ => panic!("factory error was not returned"),
    }
}

#[test]
fn accepts_custom_factory() {
    let mut stream = Box::pin(SubmitMultiStream::new(CustomFactory));

    let error = poll_error(stream.as_mut());

    assert_eq!(error.to_string(), "custom factory error");
}

#[test]
fn accepts_closure_factory() {
    let factory =
        || -> io::Result<SubmitMultiManaged<Op>> { Err(io::Error::other("closure factory error")) };
    let mut stream = Box::pin(SubmitMultiStream::new(factory));

    let error = poll_error(stream.as_mut());

    assert_eq!(error.to_string(), "closure factory error");
}
