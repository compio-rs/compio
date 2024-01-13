//! Unix-specific types for signal handling.

#[cfg(feature = "lazy_cell")]
use std::sync::LazyLock;
use std::{
    collections::HashMap,
    io::{self, Read, Write},
    ops::Deref,
    sync::Mutex,
};

use futures_channel::oneshot::{channel, Receiver, Sender};
#[cfg(not(feature = "lazy_cell"))]
use once_cell::sync::Lazy as LazyLock;
use os_pipe::{PipeReader, PipeWriter};
use slab::Slab;

static HANDLER: LazyLock<Mutex<HashMap<i32, Slab<Sender<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static PIPE: LazyLock<Pipe> = LazyLock::new(|| Pipe::new().unwrap());

struct Pipe {
    sender: PipeWriter,
}

impl Pipe {
    pub fn new() -> io::Result<Self> {
        let (receiver, sender) = os_pipe::pipe()?;

        std::thread::spawn(move || {
            real_signal_handler(receiver);
        });

        Ok(Self { sender })
    }

    pub fn send(&self, sig: i32) -> io::Result<()> {
        (&self.sender).write_all(&sig.to_ne_bytes())?;
        Ok(())
    }
}

unsafe extern "C" fn signal_handler(sig: i32) {
    PIPE.send(sig).unwrap();
}

fn real_signal_handler(mut receiver: PipeReader) {
    loop {
        let mut buffer = [0u8; 4];
        let res = receiver.read_exact(&mut buffer);
        if let Ok(()) = res {
            let sig = i32::from_ne_bytes(buffer);
            let mut handler = HANDLER.lock().unwrap();
            if let Some(fds) = handler.get_mut(&sig) {
                if !fds.is_empty() {
                    let fds = std::mem::take(fds);
                    for (_, fd) in fds {
                        fd.send(()).ok();
                    }
                }
            }
        } else {
            break;
        }
    }
}

unsafe fn init(sig: i32) {
    let _ = PIPE.deref();
    libc::signal(sig, signal_handler as *const () as usize);
}

unsafe fn uninit(sig: i32) {
    libc::signal(sig, libc::SIG_DFL);
}

fn register(sig: i32, sender: Sender<()>) -> io::Result<usize> {
    unsafe { init(sig) };
    let key = HANDLER
        .lock()
        .unwrap()
        .entry(sig)
        .or_default()
        .insert(sender);
    Ok(key)
}

fn unregister(sig: i32, key: usize) {
    let need_uninit = (|| {
        let mut handler = HANDLER.lock().unwrap();
        if let Some(fds) = handler.get_mut(&sig) {
            fds.try_remove(key);
            if !fds.is_empty() {
                return false;
            }
        }
        true
    })();
    if need_uninit {
        unsafe { uninit(sig) };
    }
}

/// Represents a listener to unix signal event.
#[derive(Debug)]
struct SignalFd {
    sig: i32,
    key: usize,
    receiver: Option<Receiver<()>>,
}

impl SignalFd {
    fn new(sig: i32) -> io::Result<Self> {
        let (sender, receiver) = channel();
        let key = register(sig, sender)?;
        Ok(Self {
            sig,
            key,
            receiver: Some(receiver),
        })
    }

    async fn wait(mut self) -> io::Result<()> {
        self.receiver
            .take()
            .expect("event could not be None")
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
    }
}

impl Drop for SignalFd {
    fn drop(&mut self) {
        unregister(self.sig, self.key);
    }
}

/// Creates a new listener which will receive notifications when the current
/// process receives the specified signal.
pub async fn signal(sig: i32) -> io::Result<()> {
    let fd = SignalFd::new(sig)?;
    fd.wait().await
}
