cfg_if::cfg_if! {
    if #[cfg(all(target_os = "linux", feature = "io-uring", feature = "io-uring-buf-ring"))] {
        mod iour;
        pub use iour::*;
    } else {
        mod fallback;
        pub use fallback::*;
    }
}

/// Trait to get the selected buffer of an io operation.
pub trait TakeBuffer {
    /// Selected buffer type
    type Buffer<'a>;

    /// Buffer pool type
    type BufferPool;

    /// Take the selected buffer with `buffer_pool`, io `result` and `flags`, if
    /// io operation is success
    fn take_buffer(
        self,
        buffer_pool: &Self::BufferPool,
        result: usize,
        flags: u32,
    ) -> Self::Buffer<'_>;
}
