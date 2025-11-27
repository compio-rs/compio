use cfg_aliases::cfg_aliases;

fn main() {
    cfg_aliases! {
        aix: { target_os = "aix" },
        linux: { target_os = "linux" },
        linux_all: { any(target_os = "linux", target_os = "android") },
        freebsd: { target_os = "freebsd" },
        netbsd: { target_os = "netbsd" },
        non_freebsd: { any(target_os = "openbsd", target_os = "netbsd") },
        bsd: { any(freebsd, non_freebsd) },
        solarish: { any(target_os = "illumos", target_os = "solaris") },
        apple: { target_vendor = "apple" },
        rustls: { feature = "ring" }
    }
}
