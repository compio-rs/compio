use std::io::{IoSlice, IoSliceMut};

pub trait AsIoSlices<'arena>: 'arena {
    /// # Safety
    ///
    /// The return slice will not live longer than self.
    unsafe fn as_io_slices(&self) -> &[IoSlice<'_>];
}

pub trait AsIoSlicesMut<'arena>: 'arena {
    /// # Safety
    ///
    /// The return slice will not live longer than self.
    unsafe fn as_io_slices_mut(&mut self) -> &mut [IoSliceMut<'arena>];

    fn set_init(&mut self, len: usize);
}
