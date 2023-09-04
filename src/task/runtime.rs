use crate::{
    driver::{Driver, Entry, OpCode, Poller, RawFd},
    task::op::{OpFuture, OpRuntime, RawOp},
    BufResult,
};
use async_task::{Runnable, Task};
use futures_util::future::Either;
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    future::{ready, Future},
    io,
    mem::MaybeUninit,
    task::{Context, Poll},
};

#[cfg(feature = "time")]
use crate::task::time::{TimerFuture, TimerRuntime};

pub(crate) struct Runtime {
    driver: Driver,
    runnables: RefCell<VecDeque<Runnable>>,
    op_runtime: RefCell<OpRuntime>,
    #[cfg(feature = "time")]
    timer_runtime: RefCell<TimerRuntime>,
    results: RefCell<HashMap<usize, BufResult<usize, Option<RawOp>>>>,
}

impl Runtime {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            driver: Driver::new()?,
            runnables: RefCell::default(),
            op_runtime: RefCell::default(),
            #[cfg(feature = "time")]
            timer_runtime: RefCell::new(TimerRuntime::new()),
            results: RefCell::default(),
        })
    }

    #[allow(dead_code)]
    pub fn driver(&self) -> &Driver {
        &self.driver
    }

    unsafe fn spawn_unchecked<F: Future>(&self, future: F) -> (Runnable, Task<F::Output>) {
        let schedule = move |runnable| self.runnables.borrow_mut().push_back(runnable);
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
            self.poll();
        }
    }

    pub fn spawn<F: Future + 'static>(&self, future: F) -> Task<F::Output> {
        let (runnable, task) = unsafe { self.spawn_unchecked(future) };
        runnable.schedule();
        task
    }

    pub fn attach(&self, fd: RawFd) -> io::Result<()> {
        self.driver.attach(fd)
    }

    pub fn submit<T: OpCode + 'static>(
        &self,
        op: T,
    ) -> impl Future<Output = (io::Result<usize>, T)> {
        let mut op_runtime = self.op_runtime.borrow_mut();
        let (user_data, op) = op_runtime.insert(op);
        let res = unsafe { self.driver.push(op.as_mut::<T>(), user_data) };
        match res {
            Ok(()) => {
                let (runnable, task) = unsafe { self.spawn_unchecked(OpFuture::new(user_data)) };
                runnable.schedule();
                Either::Left(task)
            }
            Err(e) => {
                let (op, _) = op_runtime.remove(user_data);
                Either::Right(ready((Err(e), unsafe { op.unwrap().into_inner::<T>() })))
            }
        }
    }

    #[allow(dead_code)]
    pub fn submit_dummy(&self) -> usize {
        self.op_runtime.borrow_mut().insert_dummy()
    }

    #[cfg(feature = "time")]
    pub fn create_timer(&self, delay: std::time::Duration) -> impl Future<Output = ()> {
        let mut timer_runtime = self.timer_runtime.borrow_mut();
        if let Some(key) = timer_runtime.insert(delay) {
            Either::Left(TimerFuture::new(key))
        } else {
            Either::Right(ready(()))
        }
    }

    pub fn cancel_op(&self, user_data: usize) {
        self.op_runtime.borrow_mut().cancel(user_data);
    }

    #[cfg(feature = "time")]
    pub fn cancel_timer(&self, key: usize) {
        self.timer_runtime.borrow_mut().cancel(key);
    }

    pub fn poll_task<T: OpCode>(
        &self,
        cx: &mut Context,
        user_data: usize,
    ) -> Poll<(io::Result<usize>, T)> {
        if let Some((res, op)) = self.results.borrow_mut().remove(&user_data) {
            Poll::Ready((res, unsafe { op.unwrap().into_inner::<T>() }))
        } else {
            self.op_runtime
                .borrow_mut()
                .update_waker(user_data, cx.waker().clone());
            Poll::Pending
        }
    }

    #[allow(dead_code)]
    pub fn poll_dummy(&self, cx: &mut Context, user_data: usize) -> Poll<io::Result<usize>> {
        if let Some((res, _)) = self.results.borrow_mut().remove(&user_data) {
            Poll::Ready(res)
        } else {
            self.op_runtime
                .borrow_mut()
                .update_waker(user_data, cx.waker().clone());
            Poll::Pending
        }
    }

    #[cfg(feature = "time")]
    pub fn poll_timer(&self, cx: &mut Context, key: usize) -> Poll<()> {
        let mut timer_runtime = self.timer_runtime.borrow_mut();
        if timer_runtime.contains(key) {
            timer_runtime.update_waker(key, cx.waker().clone());
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    }

    fn poll(&self) {
        #[cfg(not(feature = "time"))]
        let timeout = None;
        #[cfg(feature = "time")]
        let timeout = self.timer_runtime.borrow().min_timeout();

        const UNINIT_ENTRY: MaybeUninit<Entry> = MaybeUninit::uninit();
        let mut entries = [UNINIT_ENTRY; 16];
        match self.driver.poll(timeout, &mut entries) {
            Ok(len) => {
                for entry in &mut entries[..len] {
                    let entry = unsafe { std::mem::replace(entry, UNINIT_ENTRY).assume_init() };
                    let (op, waker) = self.op_runtime.borrow_mut().remove(entry.user_data());
                    if let Some(waker) = waker {
                        waker.wake();
                        self.results
                            .borrow_mut()
                            .insert(entry.user_data(), (entry.into_result(), op));
                    }
                }
            }
            Err(e) => {
                if e.kind() != io::ErrorKind::TimedOut {
                    panic!("{:?}", e);
                }
            }
        }
        #[cfg(feature = "time")]
        self.timer_runtime.borrow_mut().wake();
    }
}
