use std::{
    cell::RefCell,
    future::Future,
    rc::Rc,
    task::{Context, Poll},
};

use compio_buf::BufResult;
use compio_driver::{Key, OpCode, Proactor, PushEntry};

use super::{ContextExt, poll_task};

/// Future returned by [`crate::Runtime::submit_linked`] and
/// [`crate::submit_linked`].
///
/// Resolves after every operation completes, returning each result and its
/// operation in submission order. Dropping this future requests cancellation
/// of every pending member; the driver retains their resources until the
/// kernel finishes using them. Completed I/O is not rolled back.
pub struct SubmitLinked<T: OpCode, const N: usize> {
    driver: Rc<RefCell<Proactor>>,
    state: Option<State<T, N>>,
}

// Operations are pinned in driver-owned keys, never in this future's fields.
impl<T: OpCode, const N: usize> Unpin for SubmitLinked<T, N> {}

impl<T: OpCode, const N: usize> Drop for SubmitLinked<T, N> {
    fn drop(&mut self) {
        if let Some(State::Submitted { keys, .. }) = self.state.take() {
            let mut driver = self.driver.borrow_mut();
            for key in keys.into_iter().flatten() {
                driver.cancel(key);
            }
        }
    }
}

enum State<T: OpCode, const N: usize> {
    Idle {
        ops: [T; N],
        hardlinks: [bool; N],
    },
    Submitted {
        keys: [Option<Key<T>>; N],
        results: [Option<BufResult<usize, T>>; N],
    },
}

impl<T: OpCode, const N: usize> SubmitLinked<T, N> {
    pub(crate) fn new(driver: Rc<RefCell<Proactor>>, ops: [T; N], hardlinks: [bool; N]) -> Self {
        Self {
            driver,
            state: Some(State::Idle { ops, hardlinks }),
        }
    }
}

impl<T: OpCode + 'static, const N: usize> Future for SubmitLinked<T, N> {
    type Output = [BufResult<usize, T>; N];

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        loop {
            match this.state.take().expect("Cannot poll after ready") {
                State::Idle { ops, hardlinks } => {
                    let submitted = {
                        let mut driver = this.driver.borrow_mut();
                        let extras = std::array::from_fn(|_| {
                            cx.as_extra(|| driver.default_extra())
                                .unwrap_or_else(|| driver.default_extra())
                        });
                        driver.push_linked(ops, hardlinks, extras)
                    };
                    let keys = match submitted {
                        Ok(keys) => keys,
                        Err((error, ops)) => {
                            return Poll::Ready(ops.map(|op| {
                                let error = error.raw_os_error().map_or_else(
                                    || error.kind().into(),
                                    std::io::Error::from_raw_os_error,
                                );
                                BufResult(Err(error), op)
                            }));
                        }
                    };
                    // Registration can borrow the driver for an
                    // already-cancelled token.
                    if let Some(cancel) = cx.get_cancel() {
                        for key in &keys {
                            cancel.register(key);
                        }
                    }
                    this.state = Some(State::Submitted {
                        keys: keys.map(Some),
                        results: std::array::from_fn(|_| None),
                    });
                }
                State::Submitted {
                    mut keys,
                    mut results,
                } => {
                    let mut all_ready = true;
                    for (slot, result) in keys.iter_mut().zip(&mut results) {
                        let Some(key) = slot.take() else { continue };
                        match poll_task(&mut this.driver.borrow_mut(), cx.get_waker(), key) {
                            PushEntry::Pending(key) => {
                                *slot = Some(key);
                                all_ready = false;
                            }
                            PushEntry::Ready(value) => *result = Some(value),
                        }
                    }
                    if all_ready {
                        return Poll::Ready(
                            results.map(|result| result.expect("all members completed")),
                        );
                    }
                    this.state = Some(State::Submitted { keys, results });
                    return Poll::Pending;
                }
            }
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::{
        io::Write,
        os::{fd::OwnedFd, unix::net::UnixStream},
        sync::mpsc,
        time::Duration,
    };

    use compio_driver::{
        SharedFd,
        op::{Interest, PollOnce},
    };
    use futures_util::poll;

    use crate::{CancelToken, FutureExt, Runtime};

    #[test]
    fn cancellation_reaches_every_member_with_a_full_submission_queue() {
        for sqpoll in [false, true] {
            let mut proactor = compio_driver::Proactor::builder();
            proactor.capacity(16);
            if sqpoll {
                proactor.sqpoll_idle(Duration::from_millis(1));
            }
            let runtime = match Runtime::builder().with_proactor(proactor).build() {
                Ok(runtime) => runtime,
                Err(error) if sqpoll && error.kind() == std::io::ErrorKind::PermissionDenied => {
                    eprintln!("SQPOLL unavailable: {error}");
                    continue;
                }
                Err(error) => panic!("runtime setup failed: {error}"),
            };
            if !runtime.driver_type().is_iouring() {
                eprintln!("io_uring unavailable with SQPOLL={sqpoll}");
                continue;
            }
            let (reader, mut writer) = UnixStream::pair().unwrap();
            let reader = SharedFd::new(OwnedFd::from(reader));
            let (done, wait) = mpsc::channel();
            // A lost cancellation becomes an assertion failure, not a hung
            // test.
            let watchdog = std::thread::spawn(move || {
                if wait.recv_timeout(Duration::from_secs(5)).is_err() {
                    let _ = writer.write_all(b"x");
                }
            });
            let results = runtime.block_on(async {
                let cancel = CancelToken::new();
                let ops = std::array::from_fn::<_, 16, _>(|_| {
                    PollOnce::new(reader.clone(), Interest::Readable)
                });
                let mut batch = std::pin::pin!(
                    crate::submit_linked(ops, [true; 16]).with_cancel(cancel.clone())
                );
                assert!(poll!(&mut batch).is_pending());
                cancel.cancel();
                batch.await
            });
            let _ = done.send(());
            watchdog.join().unwrap();
            for result in results {
                assert_eq!(result.0.unwrap_err().raw_os_error(), Some(libc::ECANCELED));
            }
        }
    }

    #[test]
    fn dropping_a_full_chain_releases_its_descriptors() {
        let mut proactor = compio_driver::Proactor::builder();
        proactor.capacity(16);
        let runtime = Runtime::builder().with_proactor(proactor).build().unwrap();
        if !runtime.driver_type().is_iouring() {
            return;
        }
        let (reader, writer) = UnixStream::pair().unwrap();
        let reader = SharedFd::new(OwnedFd::from(reader));
        let mut wake = writer.try_clone().unwrap();
        let (done, wait) = mpsc::channel();
        let watchdog = std::thread::spawn(move || {
            let expired = wait.recv_timeout(Duration::from_secs(5)).is_err();
            if expired {
                let _ = wake.write_all(b"x");
            }
            expired
        });
        let result = runtime.block_on(async move {
            let ops = std::array::from_fn::<_, 16, _>(|_| {
                PollOnce::new(reader.clone(), Interest::Readable)
            });
            {
                let mut batch = std::pin::pin!(crate::submit_linked(ops, [true; 16]));
                assert!(poll!(&mut batch).is_pending());
            }
            drop(reader);
            crate::submit(compio_driver::op::Read::new(
                SharedFd::new(writer),
                Vec::with_capacity(1),
            ))
            .await
            .0
        });
        let _ = done.send(());
        assert!(
            !watchdog.join().unwrap(),
            "cancelled chain retained its socket"
        );
        assert_eq!(
            result.unwrap(),
            0,
            "peer must observe EOF after cancellation"
        );
    }
}
