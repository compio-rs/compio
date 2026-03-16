use std::mem::MaybeUninit;

use compio_buf::IoBuf;
use compio_io::ancillary::{AncillaryBuf, AncillaryIter, CMsgBuilder};

#[test]
fn test_cmsg() {
    let mut buf = AncillaryBuf::<128>::new();
    let mut builder = buf.builder();

    builder.push(0, 0, &()).unwrap(); // 16 / 12
    builder.push(1, 1, &u8::MAX).unwrap(); // 16 + 1 + 7 / 12 + 1 + 3
    builder.push(2, 2, &u32::MAX).unwrap(); // 16 + 4 + 4 / 12 + 4
    builder.push(3, 3, &i64::MIN).unwrap(); // 16 + 8 / 12 + 8
    builder.push(4, 4, &true).unwrap(); // 16 + 1 + 7 / 12 + 1 + 3
    assert!(buf.buf_len() == 112 || buf.buf_len() == 80);

    unsafe {
        let mut iter = AncillaryIter::new(&buf);

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
            (cmsg.level(), cmsg.ty(), cmsg.data::<bool>().unwrap()),
            (4, 4, true)
        );
        assert!(iter.next().is_none());
    }
}

#[test]
#[should_panic]
fn invalid_buffer_length() {
    AncillaryBuf::<1>::new().builder();
}

#[test]
#[should_panic]
fn invalid_buffer_alignment() {
    let mut buf = [MaybeUninit::new(0u8); 64];
    CMsgBuilder::new(&mut buf[1..]);
}
