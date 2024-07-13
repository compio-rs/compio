use compio_buf::IoBuf;
use compio_net::{CMsgBuilder, CMsgIter};

#[test]
fn test_cmsg() {
    let buf = vec![0u8; 64];
    let mut builder = CMsgBuilder::new(buf);

    unsafe {
        builder.try_push(0, 0, ()).unwrap(); // 16 / 12
        builder.try_push(1, 1, u32::MAX).unwrap(); // 16 + 4 + 4 / 12 + 4
        builder.try_push(2, 2, i64::MIN).unwrap(); // 16 + 8 / 12 + 8
    }
    let (buf, len) = builder.build();
    assert!(len == 64 || len == 48);

    let buf = buf.slice(..len);
    let mut iter = unsafe { CMsgIter::new(&buf) };

    let cmsg = iter.next().unwrap();
    assert_eq!(
        (cmsg.level(), cmsg.ty(), unsafe { cmsg.data::<()>() }),
        (0, 0, &())
    );
    let cmsg = iter.next().unwrap();
    assert_eq!(
        (cmsg.level(), cmsg.ty(), unsafe { cmsg.data::<u32>() }),
        (1, 1, &u32::MAX)
    );
    let cmsg = iter.next().unwrap();
    assert_eq!(
        (cmsg.level(), cmsg.ty(), unsafe { cmsg.data::<i64>() }),
        (2, 2, &i64::MIN)
    );
    assert!(iter.next().is_none());
}

#[test]
#[should_panic]
fn invalid_buffer_length() {
    let buf = vec![0u8; 1];
    CMsgBuilder::new(buf);
}

#[test]
#[should_panic]
fn invalid_buffer_alignment() {
    let buf = vec![0u8; 64];
    CMsgBuilder::new(buf.slice(1..));
}
