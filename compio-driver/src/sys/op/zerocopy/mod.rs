cfg_select! {
    fusion => {
        mod iour;
        #[path = "fallback.rs"]
        mod poll;
        mod fusion;
        pub use fusion::*;
    }
    io_uring => {
        mod iour;
        pub use iour::*;
    }
    _ => {
        mod fallback;
        pub use fallback::*;
    }
}
