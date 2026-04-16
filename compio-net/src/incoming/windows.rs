use std::{
    io,
    pin::Pin,
    task::{Context, Poll, ready},
};

use compio_buf::BufResult;
use compio_driver::{SharedFd, ToSharedFd, op::Accept};
use compio_runtime::Submit;
use futures_util::{FutureExt, Stream, stream::FusedStream};
use socket2::Socket as Socket2;

use crate::Socket;

pub struct Incoming<'a> {
    listener: &'a Socket,
    state: Option<Submit<Accept<SharedFd<Socket2>>>>,
}

impl<'a> Incoming<'a> {
    pub fn new(listener: &'a Socket) -> Self {
        Self {
            listener,
            state: None,
        }
    }
}

impl Stream for Incoming<'_> {
    type Item = io::Result<Socket>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            match &mut this.state {
                None => {
                    let domain = this.listener.local_addr().map(|addr| addr.domain())?;
                    let ty = this.listener.socket.r#type()?;
                    let protocol = this.listener.socket.protocol()?;
                    let socket = Socket2::new(domain, ty, protocol)?;
                    let op =
                        compio_runtime::submit(Accept::new(this.listener.to_shared_fd(), socket));
                    this.state = Some(op);
                }
                Some(op) => {
                    let BufResult(res, op) = ready!(op.poll_unpin(cx));
                    match res {
                        Ok(_) => {
                            this.state = None;
                            op.update_context()?;
                            let (accept_sock, _) = op.into_addr()?;
                            return Poll::Ready(Some(Ok(Socket::from_socket2(accept_sock)?)));
                        }
                        Err(e) => {
                            this.state = None;
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
