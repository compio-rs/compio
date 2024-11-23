use std::{
    cell::RefCell,
    fmt::Debug,
    future::Future,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use compio_buf::BufResult;
use compio_driver::{Key, OpCode, Proactor, PushEntry};
use compio_log::instrument;

pub struct OpFuture<T: OpCode> {
    key: Option<Key<T>>,
    driver: Rc<RefCell<Proactor>>,
}

impl<T: OpCode> OpFuture<T> {
    pub fn new(key: Key<T>, driver: Rc<RefCell<Proactor>>) -> Self {
        Self {
            key: Some(key),
            driver,
        }
    }

    fn poll_task(&mut self, cx: &mut Context) -> PushEntry<Key<T>, (BufResult<usize, T>, u32)> {
        let op = self.key.take().unwrap();
        instrument!(compio_log::Level::DEBUG, "poll_task", ?op);
        let mut driver = self.driver.borrow_mut();
        driver.pop(op).map_pending(|mut k| {
            driver.update_waker(&mut k, cx.waker().clone());
            k
        })
    }
}

impl<T: OpCode> Future for OpFuture<T> {
    type Output = (BufResult<usize, T>, u32);

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let res = self.poll_task(cx);
        match res {
            PushEntry::Pending(key) => {
                self.key = Some(key);
                Poll::Pending
            }
            PushEntry::Ready(res) => Poll::Ready(res),
        }
    }
}

impl<T: OpCode> Drop for OpFuture<T> {
    fn drop(&mut self) {
        if let Some(key) = self.key.take() {
            self.driver.borrow_mut().cancel(key);
        }
    }
}

impl<T: OpCode> Debug for OpFuture<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpFuture")
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}
