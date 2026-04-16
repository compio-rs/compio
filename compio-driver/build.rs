use cfg_aliases::cfg_aliases;

fn main() {
    cfg_aliases! {
        // Feature
        aio: { any(freebsd, solarish) },
        datasync: { any(
            target_os = "android",
            target_os = "freebsd",
            target_os = "fuchsia",
            target_os = "illumos",
            target_os = "linux",
            target_os = "netbsd"
        ) },

        // Platform
        linux_all: { any(target_os = "linux", target_os = "android") },
        gnulinux: { all(target_os = "linux", target_env = "gnu") },
        freebsd: { target_os = "freebsd" },
        solarish: { any(target_os = "illumos", target_os = "solaris") },

        // Driver
        polling: { all(unix, any(not(target_os = "linux"), feature = "polling")) },
        io_uring: { all(target_os = "linux", feature = "io-uring") },
        fusion: { all(target_os = "linux", feature = "io-uring", feature = "polling") },
        stub: { all(target_os = "linux", not(feature = "io_uring"), not(feature = "polling")) }
    }
}
