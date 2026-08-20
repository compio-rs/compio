cfg_select! {
    // Darwin kqueue rejects /dev/tty, so use timer polling for that path.
    target_vendor = "apple" => {
        mod poll;
        mod multishot;
        mod fusion;
        pub(super) use fusion::Input;
    }
    any(
        target_os = "linux",
        target_os = "android",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly",
        target_os = "illumos",
        target_os = "solaris",
    ) => {
        mod multishot;
        pub(super) use multishot::Input;
    }
    _ => {
        // Read with a timer where terminal readiness has not been proved.
        mod poll;
        pub(super) use poll::Input;
    }
}

pub(super) enum ReadState {
    Data,
    Closed,
}
