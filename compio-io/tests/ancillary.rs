use std::mem::MaybeUninit;

use compio_buf::IoBuf;
use compio_io::ancillary::{AncillaryBuf, AncillaryIter, CMsgBuilder};

#[test]
fn test_cmsg() {
    let mut builder = AncillaryBuf::<64>::builder();

    builder.try_push(0, 0, ()).unwrap(); // 16 / 12
    builder.try_push(1, 1, u32::MAX).unwrap(); // 16 + 4 + 4 / 12 + 4
    builder.try_push(2, 2, i64::MIN).unwrap(); // 16 + 8 / 12 + 8
    let buf = builder.finish();
    assert!(buf.buf_len() == 64 || buf.buf_len() == 48);

    unsafe {
        let mut iter = AncillaryIter::new(&buf);

        let cmsg = iter.next().unwrap();
        assert_eq!((cmsg.level(), cmsg.ty(), cmsg.data::<()>()), (0, 0, &()));
        let cmsg = iter.next().unwrap();
        assert_eq!(
            (cmsg.level(), cmsg.ty(), cmsg.data::<u32>()),
            (1, 1, &u32::MAX)
        );
        let cmsg = iter.next().unwrap();
        assert_eq!(
            (cmsg.level(), cmsg.ty(), cmsg.data::<i64>()),
            (2, 2, &i64::MIN)
        );
        assert!(iter.next().is_none());
    }
}

#[test]
#[should_panic]
fn invalid_buffer_length() {
    AncillaryBuf::<1>::builder();
}

#[test]
#[should_panic]
fn invalid_buffer_alignment() {
    let mut buf = [MaybeUninit::new(0u8); 64];
    CMsgBuilder::new(&mut buf[1..]);
}
