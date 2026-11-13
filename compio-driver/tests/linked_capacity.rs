#![cfg(io_uring)]

use std::{
    io::Write as _,
    os::{fd::OwnedFd, unix::net::UnixStream},
    time::Duration,
};

use compio_buf::{BufResult, IntoInner};
use compio_driver::{
    HBranch, HLeaf, HNil, Key, Link, OpCode, Proactor, PushEntry, SharedFd, hlist, op::Read,
};

// Ten doublings produce a statically typed balanced list of 1,024 operations,
// not an array or a runtime-length list.
macro_rules! hlist_1024 {
    ($member:expr) => {
        hlist_1024!(@double [0 1 2 3 4 5 6 7 8 9] [$member,])
    };
    (@double [$step:tt $($rest:tt)*] [$($member:expr,)*]) => {
        hlist_1024!(@double [$($rest)*] [$($member,)* $($member,)*])
    };
    (@double [] [$($member:expr,)*]) => {
        hlist![$($member),*]
    };
}

fn complete<Op: OpCode>(driver: &mut Proactor, mut key: Key<Op>) -> BufResult<usize, Op> {
    loop {
        match driver.pop(key) {
            PushEntry::Ready(result) => return result,
            PushEntry::Pending(pending) => key = pending,
        }
        driver.poll(Some(Duration::from_secs(2))).unwrap();
    }
}

trait VerifyReads {
    fn verify_reads(self, driver: &mut Proactor, expected: &mut impl Iterator<Item = u8>);
}

impl VerifyReads for HNil {
    fn verify_reads(self, _: &mut Proactor, _: &mut impl Iterator<Item = u8>) {}
}

impl VerifyReads for HLeaf<(Key<Read<Vec<u8>, SharedFd<OwnedFd>>>, Link)> {
    fn verify_reads(self, driver: &mut Proactor, expected: &mut impl Iterator<Item = u8>) {
        let (count, op) = complete(driver, self.value.0).unwrap();
        assert_eq!(count, 1);
        assert_eq!(
            op.into_inner(),
            [expected.next().expect("unexpected extra completion")]
        );
    }
}

impl<Left: VerifyReads, Right: VerifyReads> VerifyReads for HBranch<Left, Right> {
    fn verify_reads(self, driver: &mut Proactor, expected: &mut impl Iterator<Item = u8>) {
        self.left.verify_reads(driver, expected);
        self.right.verify_reads(driver, expected);
    }
}

#[test]
fn default_queue_accepts_1024_linked_operations() {
    let mut driver = Proactor::new().unwrap();
    let (mut writer, reader) = UnixStream::pair().unwrap();
    let bytes: Vec<u8> = (0..1024).map(|index| index as u8).collect();
    writer.write_all(&bytes).unwrap();
    let reader = SharedFd::new(OwnedFd::from(reader));
    let ops = hlist_1024!((Read::new(reader.clone(), vec![0; 1]), Link::Soft));
    let keys = match driver.push_linked(ops) {
        Ok(keys) => keys,
        Err((error, _)) => panic!("default-capacity chain rejected: {error}"),
    };
    let mut expected = bytes.into_iter();
    keys.verify_reads(&mut driver, &mut expected);
    assert_eq!(expected.next(), None, "every input byte needs a completion");
}
