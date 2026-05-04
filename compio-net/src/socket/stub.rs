use compio_driver::{Extra, PollFirst};

#[derive(Debug)]
pub(super) struct SocketState;

impl SocketState {
    pub(super) fn new() -> Self {
        SocketState
    }

    pub(super) fn set_recv(&self, _: &Extra) {}

    pub(super) fn set_recv_op(&self, _: &mut impl PollFirst) {}

    pub(super) fn set_accept(&self, _: &Extra) {}

    pub(super) fn set_accept_op(&self, _: &mut impl PollFirst) {}
}
