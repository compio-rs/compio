use std::{
    cell::RefCell,
    collections::VecDeque,
    future::Future,
    io,
    ptr::NonNull,
    task::{Context, Poll},
};

use async_task::{Runnable, Task};
use smallvec::SmallVec;

#[cfg(feature = "time")]
use crate::task::time::{TimerFuture, TimerRuntime};
use crate::{
    driver::{AsRawFd, Driver, Entry, OpCode, Poller, RawFd},
    task::op::{OpFuture, OpRuntime},
    Key,
};

pub(crate) struct Runtime {
    driver: RefCell<Driver>,
    runnables: RefCell<VecDeque<Runnable>>,
    squeue: RefCell<VecDeque<usize>>,
    op_runtime: RefCell<OpRuntime>,
    #[cfg(feature = "time")]
    timer_runtime: RefCell<TimerRuntime>,
}

impl Runtime {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            driver: RefCell::new(Driver::new()?),
            runnables: RefCell::default(),
            squeue: RefCell::default(),
            op_runtime: RefCell::default(),
            #[cfg(feature = "time")]
            timer_runtime: RefCell::new(TimerRuntime::new()),
        })
    }

    #[allow(dead_code)]
    pub fn raw_driver(&self) -> RawFd {
        self.driver.borrow().as_raw_fd()
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

    pub fn submit<T: OpCode + 'static>(
        &self,
        op: T,
    ) -> impl Future<Output = (io::Result<usize>, T)> {
        let mut op_runtime = self.op_runtime.borrow_mut();
        let user_data = op_runtime.insert(op);
        self.squeue.borrow_mut().push_back(*user_data);
        self.spawn(OpFuture::new(user_data))
    }

    #[allow(dead_code)]
    pub fn submit_dummy(&self) -> Key<()> {
        self.op_runtime.borrow_mut().insert_dummy()
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
        self.op_runtime.borrow_mut().cancel(user_data);
    }

    #[cfg(feature = "time")]
    pub fn cancel_timer(&self, key: usize) {
        self.timer_runtime.borrow_mut().cancel(key);
    }

    pub fn poll_task<T: OpCode>(
        &self,
        cx: &mut Context,
        user_data: Key<T>,
    ) -> Poll<(io::Result<usize>, T)> {
        let mut op_runtime = self.op_runtime.borrow_mut();
        if op_runtime.has_result(user_data) {
            let op = op_runtime.remove(user_data);
            Poll::Ready((op.result.unwrap(), unsafe {
                op.op
                    .expect("`poll_task` called on dummy Op")
                    .into_inner::<T>()
            }))
        } else {
            op_runtime.update_waker(user_data, cx.waker().clone());
            Poll::Pending
        }
    }

    #[allow(dead_code)]
    pub fn poll_dummy(&self, cx: &mut Context, user_data: Key<()>) -> Poll<io::Result<usize>> {
        let mut op_runtime = self.op_runtime.borrow_mut();
        if op_runtime.has_result(user_data) {
            let op = op_runtime.remove(user_data);
            Poll::Ready(op.result.unwrap())
        } else {
            op_runtime.update_waker(user_data, cx.waker().clone());
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

        let mut squeue = self.squeue.borrow_mut();
        let mut ops = std::iter::from_fn(|| {
            squeue.pop_front().map(|user_data| {
                let mut op = NonNull::from(self.op_runtime.borrow_mut().get_raw_op(user_data));
                // Safety: op won't outlive.
                (unsafe { op.as_mut() }.as_pin(), user_data).into()
            })
        });
        let mut entries = SmallVec::<[Entry; 1024]>::new();
        match unsafe {
            self.driver
                .borrow_mut()
                .poll(timeout, &mut ops, &mut entries)
        } {
            Ok(_) => {
                for entry in entries {
                    self.op_runtime
                        .borrow_mut()
                        .update_result(Key::new_dummy(entry.user_data()), entry.into_result());
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
