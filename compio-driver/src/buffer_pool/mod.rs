cfg_if::cfg_if! {
    if #[cfg(all(io_uring, fusion))] {
        mod iour;
        mod fallback;
        mod fusion;
        pub use fusion::*;
    } else if #[cfg(io_uring)] {
        mod iour;
        pub use iour::*;
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

    /// Take the selected buffer with `buffer_pool`, io `result` and
    /// `buffer_id`, if io operation is success.
    fn take_buffer(
        self,
        buffer_pool: &Self::BufferPool,
        result: std::io::Result<usize>,
        buffer_id: u16,
    ) -> std::io::Result<Self::Buffer<'_>>;
}
