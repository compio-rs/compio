use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};

use super::{DispatchError, Dispatchable};

type BoxedDispatchable = Box<dyn Dispatchable + Send>;

struct CounterGuard(Arc<AtomicUsize>);

impl Drop for CounterGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

fn worker(
    receiver: Receiver<BoxedDispatchable>,
    counter: Arc<AtomicUsize>,
    timeout: Duration,
) -> impl FnOnce() {
    move || {
        counter.fetch_add(1, Ordering::AcqRel);
        let _guard = CounterGuard(counter);
        while let Ok(f) = receiver.recv_timeout(timeout) {
            f.run();
        }
    }
}

#[derive(Debug, Clone)]
pub struct AsyncifyPool {
    sender: Sender<BoxedDispatchable>,
    receiver: Receiver<BoxedDispatchable>,
    counter: Arc<AtomicUsize>,
    thread_limit: usize,
    recv_timeout: Duration,
}

impl AsyncifyPool {
    pub fn new(thread_limit: usize, recv_timeout: Duration) -> io::Result<Self> {
        let (sender, receiver) = bounded(0);
        Ok(Self {
            sender,
            receiver,
            counter: Arc::new(AtomicUsize::new(0)),
            thread_limit,
            recv_timeout,
        })
    }

    pub fn dispatch<D: Dispatchable>(&self, f: D) -> Result<(), DispatchError<D>> {
        match self.sender.try_send(Box::new(f) as BoxedDispatchable) {
            Ok(_) => Ok(()),
            Err(e) => match e {
                TrySendError::Full(f) => {
                    if self.thread_limit == 0 {
                        panic!("the thread pool is needed but no worker thread is running");
                    } else if self.counter.load(Ordering::Acquire) >= self.thread_limit {
                        // Safety: we can ensure the type
                        Err(DispatchError(*unsafe {
                            Box::from_raw(Box::into_raw(f).cast())
                        }))
                    } else {
                        std::thread::spawn(worker(
                            self.receiver.clone(),
                            self.counter.clone(),
                            self.recv_timeout,
                        ));
                        self.sender.send(f).expect("the channel should not be full");
                        Ok(())
                    }
                }
                TrySendError::Disconnected(_) => {
                    unreachable!("receiver should not all disconnected")
                }
            },
        }
    }
}
