use cfg_aliases::cfg_aliases;

fn main() {
    cfg_aliases! {
        aix: { target_os = "aix" },
        linux: { target_os = "linux" },
        linuxall: { any(target_os = "linux", target_os = "android") },
        freebsd: { target_os = "freebsd" },
        netbsd: { target_os = "netbsd" },
        nonfreebsd: { any(target_os = "openbsd", target_os = "netbsd") },
        bsd: { any(freebsd, nonfreebsd) },
        solarish: { any(target_os = "illumos", target_os = "solaris") },
        apple: { target_vendor = "apple" }
    }
}
