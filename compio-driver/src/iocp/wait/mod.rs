cfg_if::cfg_if! {
    if #[cfg(feature = "iocp-wait-packet")] {
        #[path = "packet.rs"]
        mod sys;
    } else {
        #[path = "thread_pool.rs"]
        mod sys;
    }
}

pub use sys::*;
