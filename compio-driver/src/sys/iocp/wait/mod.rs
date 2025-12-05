cfg_if::cfg_if! {
    if #[cfg(feature = "iocp-wait-packet")] {
        mod packet;
        use packet as sys;
    } else {
        mod thread_pool;
        use thread_pool as sys;
    }
}

pub use sys::*;
