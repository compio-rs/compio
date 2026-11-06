use compio_buf::{IntoInner, IoBufMutExt};

#[test]
fn extend_uninit_twice() {
    let mut buf = Vec::<u8>::with_capacity(16).uninit();
    buf.extend_from_slice(&[1, 2, 3, 4]).unwrap();
    buf.extend_from_slice(&[5, 6, 7, 8]).unwrap();

    assert_eq!(buf.into_inner(), [1, 2, 3, 4, 5, 6, 7, 8]);
}
