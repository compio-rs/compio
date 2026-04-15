cfg_if::cfg_if! {
    if #[cfg(fusion)] {
        mod iour;
        #[path = "fallback.rs"]
        mod poll;
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
