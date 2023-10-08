use std::{future::Future, io, panic::resume_unwind, thread::JoinHandle};

use crossbeam_channel::{unbounded, SendError, Sender};
use futures_util::{future::LocalBoxFuture, FutureExt};

type BoxClosure<'a> = Box<dyn (FnOnce() -> LocalBoxFuture<'a, io::Result<()>>) + Send>;

pub struct Dispatcher {
    sender: Sender<BoxClosure<'static>>,
    threads: Vec<JoinHandle<io::Result<()>>>,
}

impl Dispatcher {
    pub fn new(n: usize) -> Self {
        let (sender, receiver) = unbounded::<BoxClosure<'static>>();
        let threads = (0..n)
            .map({
                |_| {
                    let receiver = receiver.clone();
                    std::thread::spawn(move || {
                        while let Ok(f) = receiver.recv() {
                            compio_runtime::block_on(f())?;
                        }
                        Ok(())
                    })
                }
            })
            .collect();
        Self { sender, threads }
    }

    pub fn dispatch<F: Future<Output = io::Result<()>> + 'static>(
        &self,
        f: impl (FnOnce() -> F) + Send + 'static,
    ) -> Result<(), SendError<BoxClosure<'static>>> {
        self.sender
            .send(Box::new(move || f().boxed_local()) as BoxClosure<'static>)
    }

    pub fn join(self) -> Vec<io::Result<()>> {
        drop(self.sender);
        self.threads
            .into_iter()
            .map(|thread| thread.join().unwrap_or_else(|e| resume_unwind(e)))
            .collect()
    }
}
