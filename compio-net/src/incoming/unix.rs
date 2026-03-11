use std::{
    io,
    os::fd::FromRawFd,
    pin::Pin,
    task::{Context, Poll, ready},
};

use compio_buf::{BufResult, IntoInner};
use compio_driver::{SharedFd, ToSharedFd, op::AcceptMulti};
use compio_runtime::SubmitMulti;
use futures_util::{Stream, StreamExt, stream::FusedStream};
use socket2::Socket as Socket2;

use crate::Socket;

pub struct Incoming<'a> {
    listener: &'a Socket,
    op: Option<SubmitMulti<AcceptMulti<SharedFd<Socket2>>>>,
}

impl<'a> Incoming<'a> {
    pub fn new(listener: &'a Socket) -> Self {
        Self { listener, op: None }
    }

    fn create_op(listener: &'a Socket) -> SubmitMulti<AcceptMulti<SharedFd<Socket2>>> {
        compio_runtime::submit_multi(AcceptMulti::new(listener.to_shared_fd()))
    }
}

impl Stream for Incoming<'_> {
    type Item = io::Result<Socket>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            if let Some(op) = &mut this.op {
                let res = ready!(op.poll_next_unpin(cx));
                if let Some(BufResult(res, _)) = res {
                    let socket = if op.is_terminated() && res.is_ok() {
                        let old_op = std::mem::replace(op, Self::create_op(this.listener));
                        old_op
                            .try_take()
                            .map_err(|_| ())
                            .expect("AcceptMulti has not completed")
                            .into_inner()
                    } else {
                        unsafe { Socket2::from_raw_fd(res? as _) }
                    };
                    return Poll::Ready(Some(Socket::from_socket2(socket)));
                } else {
                    this.op = None;
                }
            } else {
                this.op = Some(Self::create_op(this.listener));
            }
        }
    }
}

impl FusedStream for Incoming<'_> {
    fn is_terminated(&self) -> bool {
        false
    }
}
