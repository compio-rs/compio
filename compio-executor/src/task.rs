use std::{
    fmt::Debug,
    panic::{AssertUnwindSafe, catch_unwind},
    pin::Pin,
    task::{Context, Poll, Waker as StdWaker},
};

use crate::{PanicResult, util::Sender};

/// Type-erased [`Concrete`]
pub trait Pollable: Debug {
    fn poll(self: Box<Self>) -> Option<Box<dyn Pollable>>;
}

#[repr(transparent)]
pub struct Task {
    poll: Option<Box<dyn Pollable>>,
}

impl Debug for Task {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Task").field(&self.poll).finish()
    }
}

const _: () = assert!(std::mem::size_of::<Task>() == 2 * std::mem::size_of::<usize>());

struct Concrete<F: Future> {
    future: F,
    tx: Sender<PanicResult<F::Output>>,
    waker: StdWaker,
    _marker: std::marker::PhantomData<*const ()>,
}

impl<F: Future> Debug for Concrete<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Concrete")
            .field("future", &"<..>")
            .field("tx", &self.tx as _)
            .field("waker", &self.waker as _)
            .finish()
    }
}

impl Task {
    pub fn new<F: Future + 'static>(
        future: F,
        tx: Sender<PanicResult<F::Output>>,
        waker: StdWaker,
    ) -> Self {
        let inner = Concrete::new(future, tx, waker);
        Self {
            poll: Some(Box::new(inner)),
        }
    }

    pub fn take(&mut self) -> Option<Box<dyn Pollable>> {
        self.poll.take()
    }

    pub fn reset(&mut self, inner: Box<dyn Pollable>) {
        self.poll = Some(inner);
    }
}

impl<F: Future> Concrete<F> {
    pub fn new(future: F, tx: Sender<PanicResult<F::Output>>, waker: StdWaker) -> Self {
        Self {
            tx,
            future,
            waker,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<F: Future + 'static> Pollable for Concrete<F> {
    fn poll(mut self: Box<Self>) -> Option<Box<dyn Pollable>> {
        if self.tx.is_canceled() {
            return None;
        }
        let cx = &mut Context::from_waker(&self.waker);
        let mut fut = unsafe { Pin::new_unchecked(&mut self.future) };
        let res = catch_unwind(AssertUnwindSafe(|| fut.as_mut().poll(cx)));
        _ = match res {
            Ok(Poll::Pending) => return Some(self),
            Ok(Poll::Ready(res)) => self.tx.send(Ok(res)),
            Err(e) => self.tx.send(Err(e)),
        };

        None
    }
}
