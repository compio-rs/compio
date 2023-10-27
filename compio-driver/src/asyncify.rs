use std::thread::JoinHandle;

use crossbeam_channel::{bounded, Receiver, Sender, TrySendError};

type BoxClosure = Box<dyn FnOnce() + Send>;

fn worker(receiver: Receiver<BoxClosure>) -> impl FnOnce() {
    move || {
        while let Ok(f) = receiver.recv() {
            f();
        }
    }
}

pub struct AsyncifyPool {
    sender: Sender<BoxClosure>,
    receiver: Receiver<BoxClosure>,
    threads: Vec<JoinHandle<()>>,
}

impl AsyncifyPool {
    pub fn new() -> Self {
        let (sender, receiver) = bounded(0);
        let threads = vec![std::thread::spawn(worker(receiver.clone()))];
        Self {
            sender,
            receiver,
            threads,
        }
    }

    pub fn dispatch(&mut self, f: impl FnOnce() + Send + 'static) {
        match self.sender.try_send(Box::new(f) as BoxClosure) {
            Ok(_) => {}
            Err(e) => match e {
                TrySendError::Full(f) => {
                    self.threads
                        .push(std::thread::spawn(worker(self.receiver.clone())));
                    self.sender.send(f).expect("the channel should not be full");
                }
                TrySendError::Disconnected(_) => {
                    unreachable!("receiver should not all disconnected")
                }
            },
        }
    }
}
