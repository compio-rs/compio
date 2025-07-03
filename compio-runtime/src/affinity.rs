/// Bind current thread to given cpus
#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "windows",
    target_os = "macos",
    target_os = "freebsd",
    target_os = "netbsd"
))]
pub fn bind_to_cpu_set(cpus: Vec<usize>) {
    cpus.into_iter().for_each(|id| {
        use core_affinity::CoreId;
        let result = core_affinity::set_for_current(CoreId { id });
        if !result {
            // return error ?
        }
    });
}

/// Bind current thread to given cpus
#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_os = "windows",
    target_os = "macos",
    target_os = "freebsd",
    target_os = "netbsd"
)))]
pub fn bind_to_cpu_set(cpus: Vec<usize>) {}

#[cfg(test)]
mod tests {}
