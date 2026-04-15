cfg_if::cfg_if! {
    if #[cfg(windows)] {
        mod iocp;
        pub use iocp::*;
    } else if #[cfg(fusion)] {
        mod fusion;
        mod poll;
        mod iour;
        pub use fusion::*;
    } else if #[cfg(io_uring)] {
        mod iour;
        pub use iour::*;
    } else if #[cfg(stub)] {
        mod stub;
        pub use stub::*;
    } else if #[cfg(unix)] {
        mod poll;
        pub use poll::*;
    }
}

crate::assert_not_impl!(Driver, Send);
crate::assert_not_impl!(Driver, Sync);
