use std::mem::MaybeUninit;

use compio_buf::IoBufMut;

#[inline]
pub(crate) fn copy(src: &[u8], dst: &mut [MaybeUninit<u8>]) -> usize {
    let len = src.len().min(dst.len());
    unsafe {
        std::ptr::copy_nonoverlapping(src.as_ptr() as _, dst.as_mut_ptr(), len);
    }
    len
}

#[inline]
pub(crate) fn slice_to_buf(src: &[u8], buf: &mut impl IoBufMut) -> usize {
    let len = copy(src, buf.as_uninit_slice());
    unsafe { buf.set_buf_init(len) };

    len
}
