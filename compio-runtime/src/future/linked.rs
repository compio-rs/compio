//! Future for submitting heterogeneous linked operations.

use std::{
    cell::RefCell,
    future::Future,
    io,
    rc::Rc,
    task::{Context, Poll, Waker},
};

use compio_buf::BufResult;
use compio_driver::{HBranch, HLeaf, HNil, Key, Link, LinkedOps, OpCode, Proactor, PushEntry};

use super::{ContextExt, poll_task};
use crate::CancelToken;

mod sealed {
    pub trait Submit {}
    pub trait Entries {}
}

/// A heterogeneous list of `(operation, link)` members that the runtime can
/// drive as one linked chain.
///
/// This trait is the runtime counterpart of [`LinkedOps`]: it describes the
/// typed results of a chain while it is in flight. It is implemented for
/// [`HNil`], [`HLeaf`] members and recursively for [`HBranch`] subtrees, and
/// is sealed.
pub trait LinkedSubmit: LinkedOps + sealed::Submit + 'static {
    /// The results and operations of every member, in submission order.
    ///
    /// Preserves the input tree's shape with [`BufResult`] leaves, so it can
    /// be destructured with [`hlist_pat!`](crate::hlist_pat).
    type Output;

    /// The per-member state retained while the chain is in flight.
    type Entries: LinkedEntries<Ops = Self>;

    #[doc(hidden)]
    fn output_from_error(ops: Self, error: &io::Error) -> Self::Output;
}

/// The per-member state of an in-flight linked chain.
///
/// This trait is sealed and implemented for the runtime's internal state
/// lists. It exists so that [`SubmitLinked`] can drive a chain without knowing
/// its shape.
pub trait LinkedEntries: sealed::Entries + Sized {
    /// The list of operations this state belongs to.
    type Ops: LinkedSubmit<Entries = Self>;

    #[doc(hidden)]
    fn from_keys(keys: <Self::Ops as LinkedOps>::Keys) -> Self;

    #[doc(hidden)]
    fn poll_all(&mut self, driver: &mut Proactor, waker: &Waker) -> Poll<()>;

    #[doc(hidden)]
    fn cancel_all(&mut self, driver: &mut Proactor);

    #[doc(hidden)]
    fn register_all(&self, token: &CancelToken);

    #[doc(hidden)]
    fn into_output(self) -> <Self::Ops as LinkedSubmit>::Output;
}

impl sealed::Submit for HNil {}
impl sealed::Entries for HNil {}

impl LinkedSubmit for HNil {
    type Entries = HNil;
    type Output = HNil;

    fn output_from_error(ops: Self, _: &io::Error) -> Self::Output {
        ops
    }
}

impl LinkedEntries for HNil {
    type Ops = HNil;

    fn from_keys(_: <Self::Ops as LinkedOps>::Keys) -> Self {
        HNil
    }

    fn poll_all(&mut self, _: &mut Proactor, _: &Waker) -> Poll<()> {
        Poll::Ready(())
    }

    fn cancel_all(&mut self, _: &mut Proactor) {}

    fn register_all(&self, _: &CancelToken) {}

    fn into_output(self) -> <Self::Ops as LinkedSubmit>::Output {
        HNil
    }
}

impl<Op: OpCode + 'static> sealed::Submit for HLeaf<(Op, Link)> {}

impl<Op: OpCode + 'static> LinkedSubmit for HLeaf<(Op, Link)> {
    type Entries = HLeaf<Option<PushEntry<Key<Op>, BufResult<usize, Op>>>>;
    type Output = HLeaf<BufResult<usize, Op>>;

    fn output_from_error(ops: Self, error: &io::Error) -> Self::Output {
        HLeaf {
            value: BufResult(Err(clone_io_error(error)), ops.value.0),
        }
    }
}

impl<Left: LinkedSubmit, Right: LinkedSubmit> sealed::Submit for HBranch<Left, Right> {}

impl<Left: LinkedSubmit, Right: LinkedSubmit> LinkedSubmit for HBranch<Left, Right> {
    type Entries = HBranch<Left::Entries, Right::Entries>;
    type Output = HBranch<Left::Output, Right::Output>;

    fn output_from_error(ops: Self, error: &io::Error) -> Self::Output {
        HBranch {
            left: Left::output_from_error(ops.left, error),
            right: Right::output_from_error(ops.right, error),
        }
    }
}

impl<Op: OpCode + 'static> sealed::Entries
    for HLeaf<Option<PushEntry<Key<Op>, BufResult<usize, Op>>>>
{
}

impl<Op: OpCode + 'static> LinkedEntries
    for HLeaf<Option<PushEntry<Key<Op>, BufResult<usize, Op>>>>
{
    type Ops = HLeaf<(Op, Link)>;

    fn from_keys(keys: <Self::Ops as LinkedOps>::Keys) -> Self {
        HLeaf {
            value: Some(PushEntry::Pending(keys.value.0)),
        }
    }

    fn poll_all(&mut self, driver: &mut Proactor, waker: &Waker) -> Poll<()> {
        if matches!(&self.value, Some(PushEntry::Ready(_))) {
            return Poll::Ready(());
        }
        let Some(PushEntry::Pending(key)) = self.value.take() else {
            unreachable!("entry is only taken while polling");
        };
        let entry = poll_task(driver, waker, key);
        let ready = entry.is_ready();
        self.value = Some(entry);
        if ready {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }

    fn cancel_all(&mut self, driver: &mut Proactor) {
        match self.value.take() {
            Some(PushEntry::Pending(key)) => {
                driver.cancel(key);
            }
            entry => self.value = entry,
        }
    }

    fn register_all(&self, token: &CancelToken) {
        if let Some(PushEntry::Pending(key)) = &self.value {
            token.register(key);
        }
    }

    fn into_output(self) -> <Self::Ops as LinkedSubmit>::Output {
        let value = match self.value {
            Some(PushEntry::Ready(value)) => value,
            _ => unreachable!("every member completed before output"),
        };
        HLeaf { value }
    }
}

impl<Left: LinkedEntries, Right: LinkedEntries> sealed::Entries for HBranch<Left, Right> {}

impl<Left: LinkedEntries, Right: LinkedEntries> LinkedEntries for HBranch<Left, Right> {
    type Ops = HBranch<Left::Ops, Right::Ops>;

    fn from_keys(keys: <Self::Ops as LinkedOps>::Keys) -> Self {
        HBranch {
            left: Left::from_keys(keys.left),
            right: Right::from_keys(keys.right),
        }
    }

    fn poll_all(&mut self, driver: &mut Proactor, waker: &Waker) -> Poll<()> {
        let left = self.left.poll_all(driver, waker);
        let right = self.right.poll_all(driver, waker);
        if left.is_ready() && right.is_ready() {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }

    fn cancel_all(&mut self, driver: &mut Proactor) {
        self.left.cancel_all(driver);
        self.right.cancel_all(driver);
    }

    fn register_all(&self, token: &CancelToken) {
        self.left.register_all(token);
        self.right.register_all(token);
    }

    fn into_output(self) -> <Self::Ops as LinkedSubmit>::Output {
        HBranch {
            left: self.left.into_output(),
            right: self.right.into_output(),
        }
    }
}

fn clone_io_error(error: &io::Error) -> io::Error {
    error
        .raw_os_error()
        .map_or_else(|| error.kind().into(), io::Error::from_raw_os_error)
}

/// Future returned by [`crate::Runtime::submit_linked`] and
/// [`crate::submit_linked`].
///
/// The submitted list's length and every member's operation type are part of
/// this future's type, so results are typed, ordered and recovered without
/// downcasts, boxed operations or runtime type checks.
///
/// Resolves after every operation completes, returning each result and its
/// operation in submission order. Dropping this future requests cancellation
/// of every pending member; the driver retains their resources until the
/// kernel finishes using them. Completed I/O is not rolled back.
pub struct SubmitLinked<L: LinkedSubmit> {
    driver: Rc<RefCell<Proactor>>,
    state: Option<State<L>>,
}

enum State<L: LinkedSubmit> {
    Idle { ops: L },
    Submitted { entries: L::Entries },
}

// Operations are pinned in driver-owned keys, never in this future's fields.
impl<L: LinkedSubmit> Unpin for SubmitLinked<L> {}

impl<L: LinkedSubmit> Drop for SubmitLinked<L> {
    fn drop(&mut self) {
        if let Some(State::Submitted { entries }) = &mut self.state {
            entries.cancel_all(&mut self.driver.borrow_mut());
        }
    }
}

impl<L: LinkedSubmit> SubmitLinked<L> {
    pub(crate) fn new(driver: Rc<RefCell<Proactor>>, ops: L) -> Self {
        Self {
            driver,
            state: Some(State::Idle { ops }),
        }
    }
}

impl<L: LinkedSubmit> Future for SubmitLinked<L> {
    type Output = L::Output;

    fn poll(self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        loop {
            match this.state.take().expect("Cannot poll after ready") {
                State::Idle { ops } => {
                    let submitted = {
                        let mut driver = this.driver.borrow_mut();
                        driver.push_linked_with_extra(ops, |driver| {
                            cx.as_extra(|| driver.default_extra())
                                .unwrap_or_else(|| driver.default_extra())
                        })
                    };
                    let keys = match submitted {
                        Ok(keys) => keys,
                        Err((error, ops)) => {
                            return Poll::Ready(L::output_from_error(ops, &error));
                        }
                    };
                    let entries = <L::Entries as LinkedEntries>::from_keys(keys);
                    // Registration can borrow the driver for an
                    // already-cancelled token.
                    if let Some(cancel) = cx.get_cancel() {
                        entries.register_all(cancel);
                    }
                    this.state = Some(State::Submitted { entries });
                }
                State::Submitted { mut entries } => {
                    let ready = entries.poll_all(&mut this.driver.borrow_mut(), cx.get_waker());
                    if ready.is_ready() {
                        return Poll::Ready(entries.into_output());
                    }
                    this.state = Some(State::Submitted { entries });
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
        time::{Duration, Instant},
    };

    use compio_buf::{BufResult, IntoInner, SetLenExt};
    use compio_driver::{
        Link, SharedFd,
        op::{Interest, PollOnce, Read},
    };
    use futures_util::poll;

    use crate::{CancelToken, FutureExt, Runtime};

    /// Builds a 16-member chain of readable `PollOnce` operations. The list is
    /// spelled out so that its static shape is obvious.
    macro_rules! readable_chain {
        ($reader:expr, $link:expr) => {
            crate::hlist![
                (PollOnce::new($reader.clone(), Interest::Readable), $link),
                (PollOnce::new($reader.clone(), Interest::Readable), $link),
                (PollOnce::new($reader.clone(), Interest::Readable), $link),
                (PollOnce::new($reader.clone(), Interest::Readable), $link),
                (PollOnce::new($reader.clone(), Interest::Readable), $link),
                (PollOnce::new($reader.clone(), Interest::Readable), $link),
                (PollOnce::new($reader.clone(), Interest::Readable), $link),
                (PollOnce::new($reader.clone(), Interest::Readable), $link),
                (PollOnce::new($reader.clone(), Interest::Readable), $link),
                (PollOnce::new($reader.clone(), Interest::Readable), $link),
                (PollOnce::new($reader.clone(), Interest::Readable), $link),
                (PollOnce::new($reader.clone(), Interest::Readable), $link),
                (PollOnce::new($reader.clone(), Interest::Readable), $link),
                (PollOnce::new($reader.clone(), Interest::Readable), $link),
                (PollOnce::new($reader.clone(), Interest::Readable), $link),
                (PollOnce::new($reader.clone(), Interest::Readable), $link),
            ]
        };
    }

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
                let chain = readable_chain!(reader, Link::Hard);
                let mut batch =
                    std::pin::pin!(crate::submit_linked(chain).with_cancel(cancel.clone()));
                assert!(poll!(&mut batch).is_pending());
                cancel.cancel();
                batch.await
            });
            let _ = done.send(());
            watchdog.join().unwrap();
            let crate::hlist_pat![
                r0, r1, r2, r3, r4, r5, r6, r7, r8, r9, r10, r11, r12, r13, r14, r15
            ] = results;
            for result in [
                r0, r1, r2, r3, r4, r5, r6, r7, r8, r9, r10, r11, r12, r13, r14, r15,
            ] {
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
            let chain = readable_chain!(reader, Link::Hard);
            {
                let mut batch = std::pin::pin!(crate::submit_linked(chain));
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

    #[test]
    fn heterogeneous_chain_waits_for_every_member_and_keeps_its_results() {
        let mut proactor = compio_driver::Proactor::builder();
        proactor.capacity(8);
        let runtime = Runtime::builder().with_proactor(proactor).build().unwrap();
        if !runtime.driver_type().is_iouring() {
            return;
        }
        let (first_reader, mut first_writer) = UnixStream::pair().unwrap();
        let (second_reader, mut second_writer) = UnixStream::pair().unwrap();
        let first_reader = SharedFd::new(OwnedFd::from(first_reader));
        let second_reader = SharedFd::new(OwnedFd::from(second_reader));

        let results = runtime.block_on(async {
            // The first read completes short, but its hard link keeps the
            // second member pending until its separate socket is readable.
            let chain = crate::hlist![
                (
                    Read::new(second_reader.clone(), Vec::with_capacity(4)),
                    Link::Hard
                ),
                (
                    PollOnce::new(first_reader.clone(), Interest::Readable),
                    Link::Soft
                ),
                (
                    PollOnce::new(first_reader.clone(), Interest::Writable),
                    Link::Hard
                ),
            ];
            let mut batch = std::pin::pin!(crate::submit_linked(chain));
            assert!(poll!(&mut batch).is_pending());
            // Complete only the first member, then poll the future while the
            // second member remains blocked. The read's result must survive.
            second_writer.write_all(b"ab").unwrap();
            runtime.poll_with(Some(Duration::from_secs(5)));
            assert!(
                poll!(&mut batch).is_pending(),
                "the chain resolved before every member completed"
            );
            first_writer.write_all(b"x").unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                runtime.poll_with(Some(deadline.saturating_duration_since(Instant::now())));
                match poll!(&mut batch) {
                    std::task::Poll::Ready(results) => break results,
                    std::task::Poll::Pending => assert!(
                        Instant::now() < deadline,
                        "the chain did not resolve after every member completed"
                    ),
                }
            }
        });

        let crate::hlist_pat![second, first, third] = results;

        let BufResult(first_status, _first_op) = first;
        assert!(first_status.is_ok(), "first member: {first_status:?}");

        let BufResult(second_status, second_op) = second;
        assert_eq!(second_status.unwrap(), 2);
        let mut buffer = second_op.into_inner();
        // SAFETY: the read reported two initialized bytes.
        unsafe { buffer.advance_to(2) };
        assert_eq!(buffer, b"ab", "the read returned the buffer it was given");

        let BufResult(third_status, _third_op) = third;
        assert!(third_status.is_ok(), "third member: {third_status:?}");
    }
}
