use crate::driver::{Driver, OpCode, Poller, RawFd};
use async_task::{Runnable, Task};
use futures_util::future::Either;
use once_cell::sync::Lazy as LazyLock;
use slab::Slab;
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    future::{poll_fn, ready, Future},
    io,
    task::{Context, Poll, Waker},
};

pub(crate) static RUNTIME: LazyLock<Runtime> = LazyLock::new(|| Runtime::new().unwrap());

pub fn start<F: Future>(future: F) -> F::Output {
    RUNTIME.block_on(future)
}

pub struct Runtime {
    driver: Driver,
    ops: RefCell<Slab<RawOp>>,
    runnables: RefCell<VecDeque<Runnable>>,
    wakers: RefCell<HashMap<usize, Waker>>,
    results: RefCell<HashMap<usize, (io::Result<usize>, RawOp)>>,
}

unsafe impl Send for Runtime {}
unsafe impl Sync for Runtime {}

impl Runtime {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            driver: Driver::new()?,
            ops: RefCell::default(),
            runnables: RefCell::default(),
            wakers: RefCell::default(),
            results: RefCell::default(),
        })
    }

    unsafe fn spawn_unchecked<F: Future>(&self, future: F) -> (Runnable, Task<F::Output>) {
        let schedule = move |runnable| RUNTIME.runnables.borrow_mut().push_back(runnable);
        let (runnable, task) = async_task::spawn_unchecked(future, schedule);
        (runnable, task)
    }

    pub fn block_on<F: Future>(&self, future: F) -> F::Output {
        let (runnable, task) = unsafe { self.spawn_unchecked(future) };
        let waker = runnable.waker();
        runnable.schedule();
        let mut cx = Context::from_waker(&waker);
        let mut task = std::pin::pin!(task);
        loop {
            loop {
                let next_task = self.runnables.borrow_mut().pop_front();
                if let Some(task) = next_task {
                    task.run();
                } else {
                    break;
                }
            }
            if let Poll::Ready(res) = task.as_mut().poll(&mut cx) {
                return res;
            }
            let entry = self.driver.poll(None).unwrap();
            let op = self.ops.borrow_mut().remove(entry.user_data());
            self.wakers
                .borrow_mut()
                .remove(&entry.user_data())
                .unwrap()
                .wake();
            self.results
                .borrow_mut()
                .insert(entry.user_data(), (entry.into_result(), op));
        }
    }

    pub fn attach(&self, fd: RawFd) -> io::Result<()> {
        self.driver.attach(fd)
    }

    pub fn submit<T: OpCode>(&self, op: T) -> impl Future<Output = (io::Result<usize>, T)> {
        let mut ops = self.ops.borrow_mut();
        let user_data = ops.insert(RawOp::new(op));
        let op = ops.get_mut(user_data).unwrap();
        let res = self.driver.submit(unsafe { op.as_mut::<T>() }, user_data);
        if let Poll::Ready(res) = res {
            let op = ops.remove(user_data);
            Either::Left(ready((res, unsafe { op.into_inner::<T>() })))
        } else {
            let (runnable, task) =
                unsafe { self.spawn_unchecked(poll_fn(move |cx| RUNTIME.poll(cx, user_data))) };
            runnable.schedule();
            Either::Right(task)
        }
    }

    pub(crate) fn poll<T: OpCode>(
        &self,
        cx: &mut Context,
        user_data: usize,
    ) -> Poll<(io::Result<usize>, T)> {
        if let Some((res, op)) = self.results.borrow_mut().remove(&user_data) {
            Poll::Ready((res, unsafe { op.into_inner::<T>() }))
        } else {
            self.wakers
                .borrow_mut()
                .insert(user_data, cx.waker().clone());
            Poll::Pending
        }
    }
}

struct RawOp(*mut ());

impl RawOp {
    pub fn new(op: impl OpCode) -> Self {
        let op = Box::new(op);
        Self(Box::leak(op) as *mut _ as *mut ())
    }

    pub unsafe fn as_mut<T: OpCode>(&mut self) -> &mut T {
        unsafe { (self.0 as *mut T).as_mut() }.unwrap()
    }

    pub unsafe fn into_inner<T: OpCode>(self) -> T {
        *Box::from_raw(self.0 as *mut T)
    }
}
