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
        buf_ring: { all(target_os = "linux", feature = "io-uring", feature = "io-uring-buf-ring") },
        fusion: { all(target_os = "linux", feature = "io-uring", feature = "polling") }
    }
}
