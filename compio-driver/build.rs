use cfg_aliases::cfg_aliases;

fn main() {
    cfg_aliases! {
        datasync: { any(
            target_os = "android",
            target_os = "freebsd",
            target_os = "fuchsia",
            target_os = "illumos",
            target_os = "linux",
            target_os = "netbsd"
        ) },
        gnulinux: { all(target_os = "linux", target_env = "gnu") },
        freebsd: { target_os = "freebsd" },
        solarish: { any(target_os = "illumos", target_os = "solaris") },
        aio: { any(freebsd, solarish) },
        io_uring: { all(target_os = "linux", feature = "io-uring") },
        fusion: { all(target_os = "linux", feature = "io-uring", feature = "polling") },

        // fuchsia & aix also support splice, but it's OK here because we cannot test them.
        linux_all: { any(target_os = "linux", target_os = "android") },
    }
}
