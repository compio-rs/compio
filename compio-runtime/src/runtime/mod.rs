use std::{
    cell::RefCell,
    collections::VecDeque,
    future::Future,
    io,
    task::{Context, Poll},
};

use async_task::{Runnable, Task};
use compio_driver::{AsRawFd, Entry, OpCode, Proactor, RawFd};
use smallvec::SmallVec;

pub(crate) mod op;
#[cfg(feature = "time")]
pub(crate) mod time;

#[cfg(feature = "time")]
use crate::runtime::time::{TimerFuture, TimerRuntime};
use crate::{
    runtime::op::{OpFuture, OpRuntime},
    BufResult, Key,
};

pub(crate) struct Runtime {
    driver: RefCell<Proactor>,
    runnables: RefCell<VecDeque<Runnable>>,
    op_runtime: RefCell<OpRuntime>,
    #[cfg(feature = "time")]
    timer_runtime: RefCell<TimerRuntime>,
}

impl Runtime {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            driver: RefCell::new(Proactor::new()?),
            runnables: RefCell::default(),
            op_runtime: RefCell::default(),
            #[cfg(feature = "time")]
            timer_runtime: RefCell::new(TimerRuntime::new()),
        })
    }

    // Safety: the return runnable should be scheduled.
    unsafe fn spawn_unchecked<F: Future>(&self, future: F) -> Task<F::Output> {
        let schedule = move |runnable| self.runnables.borrow_mut().push_back(runnable);
        let (runnable, task) = async_task::spawn_unchecked(future, schedule);
        runnable.schedule();
        task
    }

    pub fn block_on<F: Future>(&self, future: F) -> F::Output {
        let mut result = None;
        unsafe { self.spawn_unchecked(async { result = Some(future.await) }) }.detach();
        loop {
            loop {
                let next_task = self.runnables.borrow_mut().pop_front();
                if let Some(task) = next_task {
                    task.run();
                } else {
                    break;
                }
            }
            if let Some(result) = result.take() {
                return result;
            }
            self.poll();
        }
    }

    pub fn spawn<F: Future + 'static>(&self, future: F) -> Task<F::Output> {
        unsafe { self.spawn_unchecked(future) }
    }

    pub fn attach(&self, fd: RawFd) -> io::Result<()> {
        self.driver.borrow_mut().attach(fd)
    }

    pub fn submit_raw<T: OpCode + 'static>(&self, op: T) -> Key<T> {
        let user_data = self.driver.borrow_mut().push(op);
        unsafe { Key::<T>::new(user_data) }
    }

    pub fn submit<T: OpCode + 'static>(&self, op: T) -> impl Future<Output = BufResult<usize, T>> {
        let user_data = self.submit_raw(op);
        OpFuture::new(user_data)
    }

    #[cfg(feature = "time")]
    pub fn create_timer(&self, delay: std::time::Duration) -> impl Future<Output = ()> {
        use futures_util::future::Either;

        let mut timer_runtime = self.timer_runtime.borrow_mut();
        if let Some(key) = timer_runtime.insert(delay) {
            Either::Left(TimerFuture::new(key))
        } else {
            Either::Right(std::future::ready(()))
        }
    }

    pub fn cancel_op<T>(&self, user_data: Key<T>) {
        self.driver.borrow_mut().cancel(*user_data);
        self.op_runtime.borrow_mut().cancel(*user_data);
    }

    #[cfg(feature = "time")]
    pub fn cancel_timer(&self, key: usize) {
        self.timer_runtime.borrow_mut().cancel(key);
    }

    pub fn poll_task<T: OpCode>(
        &self,
        cx: &mut Context,
        user_data: Key<T>,
    ) -> Poll<BufResult<usize, T>> {
        let mut op_runtime = self.op_runtime.borrow_mut();
        if op_runtime.has_result(*user_data) {
            let op = op_runtime.remove(*user_data);
            Poll::Ready(BufResult(op.result.unwrap(), unsafe {
                op.op
                    .expect("`poll_task` called on dummy Op")
                    .into_inner::<T>()
            }))
        } else {
            op_runtime.update_waker(*user_data, cx.waker().clone());
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

        let mut entries = SmallVec::<[Entry; 1024]>::new();
        let mut driver = self.driver.borrow_mut();
        match driver.poll(timeout, &mut entries) {
            Ok(_) => {
                for BufResult(res, op) in driver.pop(&mut entries.into_iter()) {
                    self.op_runtime.borrow_mut().update_result(
                        op.user_data(),
                        op.into_inner(),
                        res,
                    );
                }
            }
            Err(e) => match e.kind() {
                io::ErrorKind::TimedOut | io::ErrorKind::Interrupted => {}
                _ => panic!("{:?}", e),
            },
        }
        #[cfg(feature = "time")]
        self.timer_runtime.borrow_mut().wake();
    }
}

impl AsRawFd for Runtime {
    fn as_raw_fd(&self) -> RawFd {
        self.driver.borrow().as_raw_fd()
    }
}
