#![allow(semicolon_in_expressions_from_macros)]

use cfg_aliases::cfg_aliases;

fn main() {
    cfg_aliases! {
        // Feature
        aio: { any(freebsd, solarish) },
        datasync: { all(unix, not(apple)) },

        // Platform
        apple : { any(
            target_os = "macos",
            target_os = "ios",
            target_os = "tvos",
            target_os = "visionos",
            target_os = "watchos"
        ) },
        linux_all: { any(target_os = "linux", target_os = "android") },
        gnulinux: { all(target_os = "linux", target_env = "gnu") },
        freebsd: { target_os = "freebsd" },
        solarish: { any(target_os = "illumos", target_os = "solaris") },
        // Platforms whose `struct stat` carries the birth time in `st_birthtime`.
        st_birthtime: { any(
            target_os = "freebsd",
            target_os = "openbsd",
            target_vendor = "apple"
        ) },

        // Driver
        polling: { all(unix, any(not(target_os = "linux"), feature = "polling")) },
        io_uring: { all(target_os = "linux", feature = "io-uring") },
        fusion: { all(target_os = "linux", feature = "io-uring", feature = "polling") },
        stub: { all(target_os = "linux", not(feature = "io-uring"), not(feature = "polling")) }
    }
}
