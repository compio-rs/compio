use cfg_aliases::cfg_aliases;

fn main() {
    cfg_aliases! {
        affinity: { any(
            target_os = "linux",
            target_os = "android",
            target_os = "windows",
            target_os = "macos",
            target_os = "freebsd",
            target_os = "netbsd"
        ) }
    }
}
