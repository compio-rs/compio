pub mod buf;
pub mod driver;
pub mod fs;
pub mod op;
pub mod runtime;

/// A specialized `Result` type for operations with buffers.
///
/// This type is used as a return value for asynchronous IOCP methods that
/// require passing ownership of a buffer to the runtime. When the operation
/// completes, the buffer is returned whether or not the operation completed
/// successfully.
pub type BufResult<T, B> = (std::io::Result<T>, B);
