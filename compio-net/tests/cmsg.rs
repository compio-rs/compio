use aligned_array::{A8, Aligned};
use compio_buf::IoBuf;
use compio_net::{CMsgBuilder, CMsgIter};

#[test]
fn test_cmsg() {
    let mut buf: Aligned<A8, [u8; 64]> = Aligned([0u8; 64]);
    let mut builder = CMsgBuilder::new(buf.as_mut_slice());

    builder.try_push(0, 0, ()).unwrap(); // 16 / 12
    builder.try_push(1, 1, u32::MAX).unwrap(); // 16 + 4 + 4 / 12 + 4
    builder.try_push(2, 2, i64::MIN).unwrap(); // 16 + 8 / 12 + 8
    let len = builder.finish();
    assert!(len == 64 || len == 48);

    unsafe {
        let buf = buf.slice(..len);
        let mut iter = CMsgIter::new(&buf);

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
    let mut buf = [0u8; 1];
    CMsgBuilder::new(&mut buf);
}

#[test]
#[should_panic]
fn invalid_buffer_alignment() {
    let mut buf = [0u8; 64];
    CMsgBuilder::new(&mut buf[1..]);
}
