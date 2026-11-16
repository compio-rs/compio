#![cfg(io_uring)]

use std::{
    io::Write as _,
    os::{fd::OwnedFd, unix::net::UnixStream},
    time::Duration,
};

use compio_buf::{BufResult, IntoInner};
use compio_driver::{Key, OpCode, Proactor, PushEntry, SharedFd, op::Read};

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

fn linked<T: OpCode + 'static, const N: usize>(
    driver: &mut Proactor,
    ops: [T; N],
    hardlinks: [bool; N],
) -> [Key<T>; N] {
    let extras = std::array::from_fn(|_| driver.default_extra());
    match driver.push_linked(ops, hardlinks, extras) {
        Ok(keys) => keys,
        Err((error, _)) => panic!("linked submission rejected: {error}"),
    }
}

#[test]
fn short_read_severs_soft_link_but_not_hard_link() {
    for hardlink in [false, true] {
        let mut driver = Proactor::new().unwrap();
        let (first, _first_writer) = source(b"a");
        let (second, _second_writer) = source(b"b");
        let keys = linked(
            &mut driver,
            [Read::new(first, vec![0; 2]), Read::new(second, vec![0; 1])],
            [hardlink, true],
        );
        let [first, second] = keys.map(|key| complete(&mut driver, key));
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
    let ops = std::array::from_fn::<_, 16, _>(|_| {
        Splice::new(
            input.clone(),
            -1,
            output.clone(),
            -1,
            1,
            SpliceFlags::NONBLOCK,
        )
    });
    let keys = linked(&mut driver, ops, [false; 16]);
    let results = keys.map(|key| complete(&mut driver, key));
    for (index, result) in results.into_iter().enumerate() {
        let expected = if index == 0 {
            libc::EAGAIN
        } else {
            libc::ECANCELED
        };
        assert_eq!(
            result.0.unwrap_err().raw_os_error(),
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
        [
            Read::new(first, vec![0; 2]),
            Read::new(rest.clone(), vec![0; 1]),
            Read::new(rest.clone(), vec![0; 1]),
            Read::new(rest, vec![0; 1]),
        ],
        [false; 4],
    );
    let [first, second, third, fourth] = keys.map(|key| complete(&mut driver, key));
    assert_eq!(first.0.unwrap(), 1);
    for result in [second, third, fourth] {
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
    let ops = std::array::from_fn::<_, 4, _>(|_| Read::new(input.clone(), vec![0; 1]));
    let extras = std::array::from_fn(|_| driver.default_extra());
    match driver.push_linked(ops, [true; 4], extras) {
        Err((error, _)) => assert_eq!(error.kind(), std::io::ErrorKind::Unsupported),
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
    let [blocked] = linked(&mut driver, [Read::new(blocked, vec![0; 1])], [true]);
    let result = match driver.push(Read::new(ready, vec![0; 1])) {
        PushEntry::Ready(result) => result,
        PushEntry::Pending(key) => complete(&mut driver, key),
    };
    let (count, op) = result.unwrap();
    assert_eq!(count, 1);
    assert_eq!(op.into_inner()[0], b'x');
    driver.cancel(blocked);
}
