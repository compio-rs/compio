use compio_driver::AsSocketState;

use super::SocketWithState;

#[derive(Debug, Default)]
pub(super) struct SocketState;

impl SocketState {
    pub(super) fn new() -> Self {
        SocketState
    }
}

impl AsSocketState for SocketWithState {}
