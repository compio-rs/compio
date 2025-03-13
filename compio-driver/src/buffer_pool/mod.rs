cfg_if::cfg_if! {
    if #[cfg(buf_ring)] {
        cfg_if::cfg_if! {
            if #[cfg(fusion)] {
                mod fusion;
                pub use fusion::*;
            } else {
                mod iour;
                pub use iour::*;
            }
        }
    } else {
        mod fallback;
        pub use fallback::*;
    }
}

/// Trait to get the selected buffer of an io operation.
pub trait TakeBuffer {
    /// Selected buffer type. It keeps the reference to the buffer pool and
    /// returns the buffer back on drop.
    type Buffer<'a>;

    /// Buffer pool type.
    type BufferPool;

    /// Take the selected buffer with `buffer_pool`, io `result` and `flags`, if
    /// io operation is success.
    fn take_buffer(
        self,
        buffer_pool: &Self::BufferPool,
        result: std::io::Result<usize>,
        flags: u32,
    ) -> std::io::Result<Self::Buffer<'_>>;
}
