use cfg_aliases::cfg_aliases;

fn main() {
    cfg_aliases! {
        noctime: { any(
            target_os = "freebsd",
            target_os = "openbsd",
            target_vendor = "apple"
        ) },
        solarish: { any(target_os = "illumos", target_os = "solaris") },
    }
}
