cfg_select! {
    feature = "iocp-wait-packet" => {
        mod packet;
        use packet as sys;
    }
    _ => {
        mod thread_pool;
        use thread_pool as sys;
    }
}

pub use sys::*;
