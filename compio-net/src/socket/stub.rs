#[derive(Debug)]
pub(super) struct SocketState;

impl SocketState {
    pub(super) fn new() -> Self {
        SocketState
    }

    pub(super) fn recv_nonempty(&self) -> Option<bool> {
        None
    }

    pub(super) fn set_recv(&self, _: &compio_driver::Extra) {}

    pub(super) fn accept_nonempty(&self) -> Option<bool> {
        None
    }

    pub(super) fn set_accept(&self, _: &compio_driver::Extra) {}
}
