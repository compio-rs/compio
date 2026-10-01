#![cfg(io_uring)]

use std::{
    io::{self, Write as _},
    os::{fd::OwnedFd, unix::net::UnixStream},
    time::Duration,
};

use compio_buf::{BufResult, IntoInner};
use compio_driver::{
    HBranch, HLeaf, HNil, Key, Link, LinkedOps, OpCode, Proactor, PushEntry, SharedFd, hlist,
    hlist_pat,
    op::{Read, Write},
};

fn source(bytes: &[u8]) -> (SharedFd<OwnedFd>, UnixStream) {
    let (mut writer, reader) = UnixStream::pair().unwrap();
    writer.write_all(bytes).unwrap();
    (SharedFd::new(reader.into()), writer)
}

fn complete<T: OpCode>(driver: &mut Proactor, mut key: Key<T>) -> BufResult<usize, T> {
    loop {
        match driver.pop(key) {
            PushEntry::Ready(result) => return result,
            PushEntry::Pending(pending) => key = pending,
        }
        driver.poll(Some(Duration::from_secs(2))).unwrap();
    }
}

fn linked<L: LinkedOps>(driver: &mut Proactor, ops: L) -> L::Keys {
    match driver.push_linked(ops) {
        Ok(keys) => keys,
        Err((error, _)) => panic!("linked submission rejected: {error}"),
    }
}

/// Complete every member of a chain of keys in submission order, discarding
/// the recovered operations.
trait CompleteAll {
    fn complete_all(self, driver: &mut Proactor, results: &mut Vec<io::Result<usize>>);
}

impl CompleteAll for HNil {
    fn complete_all(self, _: &mut Proactor, _: &mut Vec<io::Result<usize>>) {}
}

impl<Op: OpCode> CompleteAll for HLeaf<(Key<Op>, Link)> {
    fn complete_all(self, driver: &mut Proactor, results: &mut Vec<io::Result<usize>>) {
        let BufResult(result, _) = complete(driver, self.value.0);
        results.push(result);
    }
}

impl<Left: CompleteAll, Right: CompleteAll> CompleteAll for HBranch<Left, Right> {
    fn complete_all(self, driver: &mut Proactor, results: &mut Vec<io::Result<usize>>) {
        self.left.complete_all(driver, results);
        self.right.complete_all(driver, results);
    }
}

#[test]
fn short_read_severs_soft_link_but_not_hard_link() {
    for hardlink in [false, true] {
        let mut driver = Proactor::new().unwrap();
        let (first, _first_writer) = source(b"a");
        let (second, _second_writer) = source(b"b");
        let link = if hardlink { Link::Hard } else { Link::Soft };
        let keys = linked(
            &mut driver,
            hlist![
                (Read::new(first, vec![0; 2]), link),
                (Read::new(second, vec![0; 1]), Link::Hard),
            ],
        );
        let hlist_pat![(first, _), (second, _)] = keys;
        let first = complete(&mut driver, first);
        let second = complete(&mut driver, second);
        assert_eq!(first.0.unwrap(), 1);
        if hardlink {
            let (count, op) = second.unwrap();
            assert_eq!(count, 1);
            assert_eq!(op.into_inner()[0], b'b');
        } else {
            assert_eq!(second.0.unwrap_err().raw_os_error(), Some(libc::ECANCELED));
        }
    }
}

#[test]
fn failed_splice_head_completes_every_cancelled_member() {
    use compio_driver::op::{Splice, SpliceFlags};
    use rustix::pipe::{PipeFlags, pipe_with};

    let mut driver = Proactor::new().unwrap();
    let (input, _input_writer) = pipe_with(PipeFlags::NONBLOCK).unwrap();
    let (_output_reader, output) = pipe_with(PipeFlags::NONBLOCK).unwrap();
    let input = SharedFd::new(input);
    let output = SharedFd::new(output);
    let splice = || {
        Splice::new(
            input.clone(),
            -1,
            output.clone(),
            -1,
            1,
            SpliceFlags::NONBLOCK,
        )
    };
    let keys = linked(
        &mut driver,
        hlist![
            (splice(), Link::Soft),
            (splice(), Link::Soft),
            (splice(), Link::Soft),
            (splice(), Link::Soft),
            (splice(), Link::Soft),
            (splice(), Link::Soft),
            (splice(), Link::Soft),
            (splice(), Link::Soft),
            (splice(), Link::Soft),
            (splice(), Link::Soft),
            (splice(), Link::Soft),
            (splice(), Link::Soft),
            (splice(), Link::Soft),
            (splice(), Link::Soft),
            (splice(), Link::Soft),
            (splice(), Link::Soft),
        ],
    );
    let mut results = Vec::new();
    keys.complete_all(&mut driver, &mut results);
    assert_eq!(results.len(), 16);
    for (index, result) in results.into_iter().enumerate() {
        let expected = if index == 0 {
            libc::EAGAIN
        } else {
            libc::ECANCELED
        };
        assert_eq!(
            result.unwrap_err().raw_os_error(),
            Some(expected),
            "member {index}"
        );
    }
}

#[test]
fn queue_pressure_does_not_split_soft_linked_tail() {
    let mut driver = Proactor::builder().capacity(4).build().unwrap();
    let (unrelated, _unrelated_writer) = source(b"x");
    let unrelated = driver.push(Read::new(unrelated, vec![0; 1]));
    let (first, _first_writer) = source(b"a");
    let (rest, _rest_writer) = source(b"bcd");
    let keys = linked(
        &mut driver,
        hlist![
            (Read::new(first, vec![0; 2]), Link::Soft),
            (Read::new(rest.clone(), vec![0; 1]), Link::Soft),
            (Read::new(rest.clone(), vec![0; 1]), Link::Soft),
            (Read::new(rest, vec![0; 1]), Link::Soft),
        ],
    );
    let hlist_pat![(first, _), (second, _), (third, _), (fourth, _)] = keys;
    let first = complete(&mut driver, first);
    let rest = [second, third, fourth].map(|key| complete(&mut driver, key));
    assert_eq!(first.0.unwrap(), 1);
    for result in rest {
        assert_eq!(result.0.unwrap_err().raw_os_error(), Some(libc::ECANCELED));
    }
    let unrelated = match unrelated {
        PushEntry::Ready(result) => result,
        PushEntry::Pending(key) => complete(&mut driver, key),
    };
    let (count, op) = unrelated.unwrap();
    assert_eq!(count, 1);
    assert_eq!(op.into_inner()[0], b'x');
}

#[test]
fn oversized_chain_consumes_no_input() {
    let mut driver = Proactor::builder().capacity(2).build().unwrap();
    let (input, _writer) = source(b"abcd");
    let ops = hlist![
        (Read::new(input.clone(), vec![0; 1]), Link::Hard),
        (Read::new(input.clone(), vec![0; 1]), Link::Hard),
        (Read::new(input.clone(), vec![0; 1]), Link::Hard),
        (Read::new(input.clone(), vec![0; 1]), Link::Hard),
    ];
    match driver.push_linked(ops) {
        Err((error, ops)) => {
            assert_eq!(error.kind(), io::ErrorKind::Unsupported);
            let hlist_pat![(first, _), (second, _), (third, _), (fourth, _)] = ops;
            for op in [first, second, third, fourth] {
                assert_eq!(op.into_inner(), vec![0; 1]);
            }
        }
        Ok(_) => panic!("oversized chain was accepted"),
    }
    let result = match driver.push(Read::new(input, vec![0; 4])) {
        PushEntry::Ready(result) => result,
        PushEntry::Pending(key) => complete(&mut driver, key),
    };
    let (count, op) = result.unwrap();
    assert_eq!(count, 4);
    assert_eq!(&op.into_inner()[..4], b"abcd");
}

#[test]
fn final_member_does_not_link_an_unrelated_operation() {
    let mut driver = Proactor::builder().capacity(8).build().unwrap();
    let (blocked, _blocked_writer) = source(b"");
    let (ready, _ready_writer) = source(b"x");
    let keys = linked(
        &mut driver,
        hlist![(Read::new(blocked, vec![0; 1]), Link::Hard)],
    );
    let hlist_pat![(blocked, _)] = keys;
    let result = match driver.push(Read::new(ready, vec![0; 1])) {
        PushEntry::Ready(result) => result,
        PushEntry::Pending(key) => complete(&mut driver, key),
    };
    let (count, op) = result.unwrap();
    assert_eq!(count, 1);
    assert_eq!(op.into_inner()[0], b'x');
    let blocked = match driver.pop(blocked) {
        PushEntry::Pending(key) => key,
        PushEntry::Ready(_) => panic!("blocked member completed without input"),
    };
    driver.cancel(blocked);
}

#[test]
fn heterogeneous_write_then_read_links_in_order() {
    let mut driver = Proactor::new().unwrap();
    let (mut writer, reader) = UnixStream::pair().unwrap();
    // Seed the stream so the read cannot block if it ran before the write: it
    // would then see only these bytes, and the assertions below would fail
    // instead of timing out.
    writer.write_all(b"old!!").unwrap();
    let writer = SharedFd::new(writer.into());
    let reader = SharedFd::new(reader.into());
    let keys = linked(
        &mut driver,
        hlist![
            (Write::new(writer, b"hello".to_vec()), Link::Soft),
            (Read::new(reader, vec![0; 10]), Link::Hard),
        ],
    );
    let hlist_pat![(write_key, _), (read_key, _)] = keys;
    let write_result: BufResult<usize, Write<Vec<u8>, SharedFd<OwnedFd>>> =
        complete(&mut driver, write_key);
    let read_result: BufResult<usize, Read<Vec<u8>, SharedFd<OwnedFd>>> =
        complete(&mut driver, read_key);
    let (written, write_op) = write_result.unwrap();
    assert_eq!(written, 5);
    assert_eq!(write_op.into_inner(), b"hello".to_vec());
    let (read, read_op) = read_result.unwrap();
    assert_eq!(read, 10);
    assert_eq!(&read_op.into_inner()[..], b"old!!hello");
}
