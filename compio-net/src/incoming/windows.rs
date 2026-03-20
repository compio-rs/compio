use std::{
    io,
    pin::Pin,
    task::{Context, Poll, ready},
};

use compio_buf::BufResult;
use compio_driver::{SharedFd, ToSharedFd, op::Accept};
use compio_runtime::{JoinHandle, Submit};
use futures_util::{FutureExt, Stream, stream::FusedStream};
use socket2::Socket as Socket2;

use crate::Socket;

#[allow(clippy::large_enum_variant)]
enum IncomingState {
    Idle,
    CreatingSocket(JoinHandle<io::Result<Socket2>>),
    Accepting(Submit<Accept<SharedFd<Socket2>>>),
}

pub struct Incoming<'a> {
    listener: &'a Socket,
    state: IncomingState,
}

impl<'a> Incoming<'a> {
    pub fn new(listener: &'a Socket) -> Self {
        Self {
            listener,
            state: IncomingState::Idle,
        }
    }
}

impl Stream for Incoming<'_> {
    type Item = io::Result<Socket>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            match &mut this.state {
                IncomingState::Idle => {
                    let domain = this.listener.local_addr().map(|addr| addr.domain())?;
                    let ty = this.listener.socket.r#type()?;
                    let protocol = this.listener.socket.protocol()?;
                    let handle =
                        compio_runtime::spawn_blocking(move || Socket2::new(domain, ty, protocol));
                    this.state = IncomingState::CreatingSocket(handle);
                }
                IncomingState::CreatingSocket(handle) => match ready!(handle.poll_unpin(cx)) {
                    Ok(Ok(socket)) => {
                        let op = compio_runtime::submit(Accept::new(
                            this.listener.to_shared_fd(),
                            socket,
                        ));
                        this.state = IncomingState::Accepting(op);
                    }
                    Ok(Err(e)) => {
                        this.state = IncomingState::Idle;
                        return Poll::Ready(Some(Err(e)));
                    }
                    Err(e) => {
                        this.state = IncomingState::Idle;
                        e.resume_unwind();
                    }
                },
                IncomingState::Accepting(op) => {
                    let BufResult(res, op) = ready!(op.poll_unpin(cx));
                    match res {
                        Ok(_) => {
                            this.state = IncomingState::Idle;
                            op.update_context()?;
                            let (accept_sock, _) = op.into_addr()?;
                            return Poll::Ready(Some(Ok(Socket::from_socket2(accept_sock)?)));
                        }
                        Err(e) => {
                            this.state = IncomingState::Idle;
                            return Poll::Ready(Some(Err(e)));
                        }
                    }
                }
            }
        }
    }
}

impl FusedStream for Incoming<'_> {
    fn is_terminated(&self) -> bool {
        false
    }
}
