use compio_driver::{AsSocketState, Extra};

use super::SocketWithState;

#[derive(Debug, Default)]
pub(super) struct SocketState;

#[allow(dead_code)]
impl SocketState {
    pub(super) fn new() -> Self {
        SocketState
    }

    pub(super) fn set_recv(&self, _: &Extra) {}

    pub(super) fn set_accept(&self, _: &Extra) {}
}

impl AsSocketState for SocketWithState {}
