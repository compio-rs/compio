use std::mem::MaybeUninit;

use compio_buf::{IoBuf, IoBufMut, SetLen};
use compio_io::ancillary::{AncillaryBuf, AncillaryBuilder, AncillaryIter};

fn build_cmsg<B: IoBufMut + ?Sized>(mut builder: AncillaryBuilder<B>) {
    builder.push(0, 0, &()).unwrap(); // 16 / 12
    builder.push(1, 1, &u8::MAX).unwrap(); // 16 + 1 + 7 / 12 + 1 + 3
    builder.push(2, 2, &u32::MAX).unwrap(); // 16 + 4 + 4 / 12 + 4
    builder.push(3, 3, &i64::MIN).unwrap(); // 16 + 8 / 12 + 8
    builder.push(4, 4, &[0; 1]).unwrap(); // 16 + 1 + 7 / 12 + 1 + 3
}

unsafe fn check_cmsg(buf: &[u8]) {
    let mut iter = unsafe { AncillaryIter::new(buf) };
    let cmsg = iter.next().unwrap();
    assert_eq!(
        (cmsg.level(), cmsg.ty(), cmsg.data::<()>().unwrap()),
        (0, 0, ())
    );
    let cmsg = iter.next().unwrap();
    assert_eq!(
        (cmsg.level(), cmsg.ty(), cmsg.data::<u8>().unwrap()),
        (1, 1, u8::MAX)
    );
    let cmsg = iter.next().unwrap();
    assert_eq!(
        (cmsg.level(), cmsg.ty(), cmsg.data::<u32>().unwrap()),
        (2, 2, u32::MAX)
    );
    let cmsg = iter.next().unwrap();
    assert_eq!(
        (cmsg.level(), cmsg.ty(), cmsg.data::<i64>().unwrap()),
        (3, 3, i64::MIN)
    );
    let cmsg = iter.next().unwrap();
    assert_eq!(
        (cmsg.level(), cmsg.ty(), cmsg.data().unwrap()),
        (4, 4, [0; 1])
    );
    assert!(iter.next().is_none());
}

#[test]
fn test_cmsg() {
    let mut buf = AncillaryBuf::<128>::new();
    let builder = buf.builder();

    build_cmsg(builder);
    assert!(buf.buf_len() == 112 || buf.buf_len() == 80);

    unsafe { check_cmsg(&buf) }
}

// Test a custom DST buffer. It checks the compatibility for the previous
// `CMsgBuilder`.
#[test]
fn test_custom_buffer_cmsg() {
    struct MaybeUninitBuffer<T: ?Sized> {
        len: usize,
        inner: T,
    }

    impl<T: AsRef<[MaybeUninit<u8>]> + ?Sized + 'static> IoBuf for MaybeUninitBuffer<T> {
        fn as_init(&self) -> &[u8] {
            unsafe { self.inner.as_ref()[..self.len].assume_init_ref() }
        }
    }

    impl<T: ?Sized> SetLen for MaybeUninitBuffer<T> {
        unsafe fn set_len(&mut self, new_len: usize) {
            self.len = new_len;
        }
    }

    impl<T: AsRef<[MaybeUninit<u8>]> + AsMut<[MaybeUninit<u8>]> + ?Sized + 'static> IoBufMut
        for MaybeUninitBuffer<T>
    {
        fn as_uninit(&mut self) -> &mut [MaybeUninit<u8>] {
            self.inner.as_mut()
        }
    }

    let mut buf = MaybeUninitBuffer {
        len: 0,
        inner: [MaybeUninit::zeroed(); 128],
    };
    let builder = AncillaryBuilder::new(&mut buf as &mut MaybeUninitBuffer<[MaybeUninit<u8>]>);

    build_cmsg(builder);
    assert!(buf.buf_len() == 112 || buf.buf_len() == 80);

    unsafe { check_cmsg(buf.as_init()) }
}

#[test]
#[should_panic]
fn invalid_buffer_length() {
    AncillaryBuf::<1>::new().builder();
}
