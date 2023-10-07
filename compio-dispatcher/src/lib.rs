use std::{io, panic::resume_unwind, sync::Arc, thread::JoinHandle};

use async_channel::{unbounded, Sender};

pub struct Dispatcher<T> {
    sender: Sender<SendWrapper<T>>,
    threads: Vec<JoinHandle<io::Result<()>>>,
}

impl<T: 'static> Dispatcher<T> {
    pub fn new<F: Fn(T) -> io::Result<()>>(
        n: usize,
        f: impl (Fn(usize) -> F) + Send + Sync + 'static,
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
                        compio_runtime::block_on(async {
                            while let Ok(value) = receiver.recv().await {
                                f(value.0)?;
                            }
                            Ok(())
                        })
                    })
                }
            })
            .collect();
        Self { sender, threads }
    }

    pub fn join(self) -> Vec<io::Result<()>> {
        self.sender.close();
        self.threads
            .into_iter()
            .map(|thread| match thread.join() {
                Ok(res) => res,
                Err(e) => resume_unwind(e),
            })
            .collect()
    }
}

struct SendWrapper<T>(pub T);

unsafe impl<T> Send for SendWrapper<T> {}
