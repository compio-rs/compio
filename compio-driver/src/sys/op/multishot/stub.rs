use crate::{OpCode, sys::prelude::*};

/// Accept multiple connections.
pub struct AcceptMulti<S> {
    _fd: S,
}

impl<S> AcceptMulti<S> {
    /// Create [`AcceptMulti`].
    pub fn new(fd: S) -> Self {
        Self { _fd: fd }
    }
}

impl<S> IntoInner for AcceptMulti<S> {
    type Inner = Socket2;

    fn into_inner(self) -> Self::Inner {
        stub_unimpl()
    }
}

impl<S: AsFd> OpCode for AcceptMulti<S> {
    type Control = ();

    unsafe fn init(&mut self, _: &mut Self::Control) {}
}
