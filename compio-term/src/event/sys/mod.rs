cfg_select! {
    windows => {
        mod windows;
        pub(super) use windows::EventSource;
    }
    unix => {
        mod unix;
        pub(super) use unix::EventSource;
    }
    _ => {
        compile_error!("compio-term supports Unix and Windows targets");
    }
}
