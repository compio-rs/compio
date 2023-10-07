use std::{future::Future, io, panic::resume_unwind, sync::Arc, thread::JoinHandle};

use crossbeam_channel::{unbounded, SendError, Sender};

pub struct Dispatcher<T> {
    sender: Sender<SendWrapper<T>>,
    threads: Vec<JoinHandle<io::Result<()>>>,
}

impl<T: 'static> Dispatcher<T> {
    pub fn new<F: Future<Output = io::Result<()>>, W: Fn(T) -> F>(
        n: usize,
        f: impl (Fn(usize) -> W) + Send + Sync + 'static,
    ) -> Self {
        let f = Arc::new(f);
        let (sender, receiver) = unbounded::<SendWrapper<T>>();
        let threads = (0..n)
            .map({
                |i| {
                    let receiver = receiver.clone();
                    let f = f.clone();
                    std::thread::spawn(move || {
                        let f = f(i);
                        while let Ok(value) = receiver.recv() {
                            compio_runtime::block_on(async { f(value.0).await })?;
                        }
                        Ok(())
                    })
                }
            })
            .collect();
        Self { sender, threads }
    }

    pub fn dispatch(&self, value: T) -> Result<(), SendError<T>> {
        self.sender
            .send(SendWrapper(value))
            .map_err(|e| SendError(e.0.0))
    }

    pub fn join(self) -> Vec<io::Result<()>> {
        drop(self.sender);
        self.threads
            .into_iter()
            .map(|thread| thread.join().unwrap_or_else(|e| resume_unwind(e)))
            .collect()
    }
}

struct SendWrapper<T>(pub T);

unsafe impl<T> Send for SendWrapper<T> {}
