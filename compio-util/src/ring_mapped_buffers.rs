use std::io;

use compio_driver::ring_mapped_buffers::{Builder, RawRingMappedBuffers, RingMappedBuffer};
use compio_runtime::{register_ring_mapped_buffers, unregister_ring_mapped_buffers};

#[derive(Clone)]
pub struct RingMappedBuffers(RawRingMappedBuffers);

impl RingMappedBuffers {
    pub fn build(builder: Builder) -> io::Result<Self> {
        let raw_ring_mapped_buffers = register_ring_mapped_buffers(builder)?;

        Ok(Self(raw_ring_mapped_buffers))
    }

    pub fn buf_len(&self) -> usize {
        self.0.buf_len()
    }

    pub fn bgid(&self) -> u16 {
        self.0.bgid()
    }

    pub fn as_raw(&self) -> &RawRingMappedBuffers {
        &self.0
    }

    #[doc(hidden)]
    /// Safety: user should make sure the `len` is correct
    pub unsafe fn get_buf(&self, len: u32, flags: u32) -> io::Result<RingMappedBuffer> {
        self.0.get_buf(len, flags)
    }
}

impl Drop for RingMappedBuffers {
    fn drop(&mut self) {
        let _ = unregister_ring_mapped_buffers(&mut self.0);
    }
}
