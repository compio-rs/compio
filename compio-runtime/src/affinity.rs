use std::collections::HashSet;

use compio_log::*;

/// Bind current thread to given cpus
pub fn bind_to_cpu_set(cpus: &HashSet<usize>) {
    if cpus.is_empty() {
        return;
    }

    let Some(ids) = available_cpus() else {
        return;
    };

    match (ids.iter().max(), cpus.iter().max()) {
        (Some(max_id), Some(max_cpu)) if *max_cpu > *max_id => {
            error!("CPU ID: {max_cpu} exceeds maximum available CPU ID: {max_id}");
        }
        _ => {}
    }

    // Keep only the requested CPUs that actually exist on this machine.
    let cpu_set = ids.intersection(cpus).copied();

    set_affinity(cpu_set);
}

// Returns the set of CPUs the running process/thread is currently *permitted*
// to run on, not every CPU physically present on the machine. If the process
// was launched with a restricted affinity that compio doesn't control (e.g.
// `taskset`, a cgroup/cpuset, or a job object), `bind_to_cpu_set` can only ever
// bind to a subset of that mask — requested CPUs outside it are dropped by the
// intersection.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn available_cpus() -> Option<HashSet<usize>> {
    use std::mem;

    // SAFETY: `cpu_set_t` is a plain bitset; `sched_getaffinity` fills it with the
    // CPUs the current thread (pid 0) is allowed to run on.
    let set = unsafe {
        let mut set: libc::cpu_set_t = mem::zeroed();
        if libc::sched_getaffinity(0, mem::size_of::<libc::cpu_set_t>(), &mut set) != 0 {
            return None;
        }
        set
    };

    // SAFETY: `CPU_ISSET` only reads bits from the initialized `set`.
    let cpu_set_size = {
        #[cfg(target_os = "linux")]
        {
            libc::CPU_SETSIZE as usize
        }
        #[cfg(target_os = "android")]
        {
            libc::CPU_SETSIZE
        }
    };

    Some(
        (0..cpu_set_size)
            .filter(|&i| unsafe { libc::CPU_ISSET(i, &set) })
            .collect(),
    )
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn set_affinity(cpus: impl Iterator<Item = usize>) {
    use std::{io, mem};

    // SAFETY: `cpu_set_t` is a plain bitset, `CPU_SET` only flips bits in it, and
    // `sched_setaffinity` is given a valid pointer with a matching size for the
    // current thread (pid 0).
    let res = unsafe {
        let mut set: libc::cpu_set_t = mem::zeroed();
        for cpu in cpus {
            libc::CPU_SET(cpu, &mut set);
        }
        libc::sched_setaffinity(0, mem::size_of::<libc::cpu_set_t>(), &set)
    };

    if res != 0 {
        warn!(
            "cannot set CPU affinity for current thread: {}",
            io::Error::last_os_error()
        );
    }
}

#[cfg(target_os = "freebsd")]
fn available_cpus() -> Option<HashSet<usize>> {
    use std::mem;

    // SAFETY: `cpuset_t` is a plain bitset; `cpuset_getaffinity` fills it with the
    // CPUs the current thread (`CPU_WHICH_TID` with id -1) is allowed to run on.
    let set = unsafe {
        let mut set: libc::cpuset_t = mem::zeroed();
        let res = libc::cpuset_getaffinity(
            libc::CPU_LEVEL_WHICH,
            libc::CPU_WHICH_TID,
            -1,
            mem::size_of::<libc::cpuset_t>(),
            &mut set,
        );
        if res != 0 {
            return None;
        }
        set
    };

    // SAFETY: `CPU_ISSET` only reads bits from the initialized `set`.
    Some(
        (0..libc::CPU_SETSIZE as usize)
            .filter(|&i| unsafe { libc::CPU_ISSET(i, &set) })
            .collect(),
    )
}

#[cfg(target_os = "freebsd")]
fn set_affinity(cpus: impl Iterator<Item = usize>) {
    use std::{io, mem};

    // SAFETY: `cpuset_t` is a plain bitset, `CPU_SET` only flips bits in it, and
    // `cpuset_setaffinity` is given a valid pointer with a matching size for the
    // current thread (`CPU_WHICH_TID` with id -1).
    let res = unsafe {
        let mut set: libc::cpuset_t = mem::zeroed();
        for cpu in cpus {
            libc::CPU_SET(cpu, &mut set);
        }
        libc::cpuset_setaffinity(
            libc::CPU_LEVEL_WHICH,
            libc::CPU_WHICH_TID,
            -1,
            mem::size_of::<libc::cpuset_t>(),
            &set,
        )
    };

    if res != 0 {
        warn!(
            "cannot set CPU affinity for current thread: {}",
            io::Error::last_os_error()
        );
    }
}

// NetBSD's `cpuset_t` is opaque and heap-allocated; it is manipulated through
// the `_cpuset_*` helpers rather than the `CPU_*` macros, and bound with the
// POSIX `pthread_{get,set}affinity_np` calls.
#[cfg(target_os = "netbsd")]
fn available_cpus() -> Option<HashSet<usize>> {
    // SAFETY: `_cpuset_create` returns an owned set that every path frees with
    // `_cpuset_destroy`. `pthread_getaffinity_np` fills it with the CPUs the
    // current thread is allowed to run on, and `_cpuset_isset` only reads it.
    unsafe {
        let set = libc::_cpuset_create();
        if set.is_null() {
            return None;
        }

        let res = libc::pthread_getaffinity_np(libc::pthread_self(), libc::_cpuset_size(set), set);
        if res != 0 {
            libc::_cpuset_destroy(set);
            return None;
        }

        // `_cpuset_isset` returns >0 when the CPU is in the set, 0 when it isn't,
        // and <0 once the index is past the set's capacity.
        // `cpuid_t` is a public-only-within-libc alias of `c_ulong`, so cast to the
        // underlying type (which unifies with the parameter) rather than naming it.
        let bits = libc::_cpuset_size(set) * 8;
        let cpus = (0..bits)
            .filter(|&i| libc::_cpuset_isset(i as libc::c_ulong, set) > 0)
            .collect();
        libc::_cpuset_destroy(set);
        Some(cpus)
    }
}

#[cfg(target_os = "netbsd")]
fn set_affinity(cpus: impl Iterator<Item = usize>) {
    use std::io;

    // SAFETY: `_cpuset_create` returns an owned set freed with `_cpuset_destroy`;
    // `_cpuset_set` flips bits in it and `pthread_setaffinity_np` applies it to the
    // current thread.
    let res = unsafe {
        let set = libc::_cpuset_create();
        if set.is_null() {
            warn!("cannot allocate cpuset for current thread");
            return;
        }
        for cpu in cpus {
            libc::_cpuset_set(cpu as libc::c_ulong, set);
        }
        let res = libc::pthread_setaffinity_np(libc::pthread_self(), libc::_cpuset_size(set), set);
        libc::_cpuset_destroy(set);
        res
    };

    // The `pthread_*` functions return the error number directly rather than
    // setting `errno`.
    if res != 0 {
        warn!(
            "cannot set CPU affinity for current thread: {}",
            io::Error::from_raw_os_error(res)
        );
    }
}

#[cfg(windows)]
fn available_cpus() -> Option<HashSet<usize>> {
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessAffinityMask};

    // `GetProcessAffinityMask` is `KAFFINITY`-based, so it only reports the
    // process's primary processor group (at most 64 logical processors). CPUs that
    // live in other groups on machines with more than 64 CPUs are not represented;
    // see the note in `set_affinity`.
    let mut process_mask: usize = 0;
    let mut system_mask: usize = 0;

    // SAFETY: `GetCurrentProcess` returns a pseudo-handle for the current process
    // and both masks are valid out-pointers.
    let res =
        unsafe { GetProcessAffinityMask(GetCurrentProcess(), &mut process_mask, &mut system_mask) };
    if res == 0 {
        return None;
    }

    Some(
        (0..usize::BITS as usize)
            .filter(|&i| process_mask & (1usize << i) != 0)
            .collect(),
    )
}

#[cfg(windows)]
fn set_affinity(cpus: impl Iterator<Item = usize>) {
    use std::io;

    use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadAffinityMask};

    // `SetThreadAffinityMask` (like `GetProcessAffinityMask`) is `KAFFINITY`-based,
    // so it can only address CPUs within a single processor group — at most 64
    // logical processors. Machines with more than 64 CPUs split them across
    // multiple groups, which would require the processor-group APIs
    // (`SetThreadGroupAffinity` / `SetThreadSelectedCpuSetMasks`, the latter
    // Windows 11+ only) plus a cross-group CPU numbering scheme. We intentionally
    // keep the simple single-group behavior and skip CPUs outside the 64-bit mask.
    let bits = usize::BITS as usize;
    let mut mask: usize = 0;
    for cpu in cpus {
        if cpu >= bits {
            warn!("CPU {cpu} exceeds the affinity mask width ({bits}); ignoring");
            continue;
        }
        mask |= 1usize << cpu;
    }
    if mask == 0 {
        return;
    }

    // SAFETY: `GetCurrentThread` returns a pseudo-handle for the current thread
    // that needs no clean-up, and the mask is a subset of the process mask.
    let res = unsafe { SetThreadAffinityMask(GetCurrentThread(), mask) };
    if res == 0 {
        warn!(
            "cannot set CPU affinity for current thread: {}",
            io::Error::last_os_error()
        );
    }
}

#[cfg(target_os = "macos")]
fn available_cpus() -> Option<HashSet<usize>> {
    // macOS only exposes affinity *hints* rather than real pinning, so treat any
    // CPU index below the logical CPU count as valid.
    let n = std::thread::available_parallelism().ok()?.get();
    Some((0..n).collect())
}

#[cfg(target_os = "macos")]
fn set_affinity(cpus: impl Iterator<Item = usize>) {
    // macOS cannot pin a thread to a *set* of CPUs; `THREAD_AFFINITY_POLICY` only
    // assigns a single affinity tag used as a cache-locality hint. Use one of the
    // requested CPUs as the tag (best effort).
    let mut cpus = cpus;
    let Some(tag) = cpus.next() else {
        return;
    };

    if cpus.next().is_some() {
        warn!("only setting affinity to the first CPU, ignoring extra provided on this platform");
    }

    let mut info = libc::thread_affinity_policy_data_t {
        affinity_tag: tag as libc::integer_t,
    };

    // SAFETY: `pthread_mach_thread_np` returns the current thread's Mach port (a
    // borrow that needs no deallocation), and `thread_policy_set` reads
    // `THREAD_AFFINITY_POLICY_COUNT` integers from `info`.
    let res = unsafe {
        libc::thread_policy_set(
            libc::pthread_mach_thread_np(libc::pthread_self()),
            libc::THREAD_AFFINITY_POLICY as libc::thread_policy_flavor_t,
            (&mut info as *mut libc::thread_affinity_policy_data_t).cast(),
            libc::THREAD_AFFINITY_POLICY_COUNT,
        )
    };

    if res != 0 {
        warn!("cannot set CPU affinity for current thread: kern_return_t {res}");
    }
}

// ===== Other platforms (iOS, illumos/Solaris, ...) =====

// These platforms do not expose an API to query or pin a thread to a CPU set,
// so affinity is a no-op: `available_cpus` returns `None` and `bind_to_cpu_set`
// bails out before `set_affinity` is ever reached.
#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "macos",
    windows,
)))]
fn available_cpus() -> Option<HashSet<usize>> {
    None
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "macos",
    windows,
)))]
fn set_affinity(_cpus: impl Iterator<Item = usize>) {
    warn!("ignore setting CPU affinity for current thread: not supported on this platform");
}

// On Linux/Android, FreeBSD and NetBSD `available_cpus` reads the *current
// thread's* affinity, so it doubles as a read-back of whatever
// `bind_to_cpu_set` applied — which lets us assert the whole requested set is
// bound, not just one CPU.
#[cfg(all(
    test,
    any(
        target_os = "linux",
        target_os = "android",
        target_os = "freebsd",
        target_os = "netbsd"
    )
))]
mod tests {
    use std::collections::HashSet;

    use super::{available_cpus, bind_to_cpu_set};

    #[test]
    fn available_cpus_is_nonempty() {
        let cpus = available_cpus().expect("thread affinity must be available");
        assert!(!cpus.is_empty());
    }

    #[test]
    fn binds_every_requested_cpu() {
        let available = available_cpus().expect("thread affinity must be available");

        // Pick up to two CPUs to prove the *whole* set is applied, not just the
        // last one (the bug this module guards against).
        let want: HashSet<usize> = available.into_iter().take(2).collect();
        bind_to_cpu_set(&want);

        assert_eq!(available_cpus().unwrap(), want);
    }

    #[test]
    fn ignores_nonexistent_cpus() {
        let available = available_cpus().expect("thread affinity must be available");
        let max = *available.iter().max().unwrap();

        // A CPU that doesn't exist is dropped by the intersection; the binding
        // still applies to the valid CPU.
        let valid = *available.iter().min().unwrap();
        let want = HashSet::from([valid, max + 1000]);
        bind_to_cpu_set(&want);

        assert_eq!(available_cpus().unwrap(), HashSet::from([valid]));
    }
}

#[cfg(all(test, windows))]
mod tests {
    use std::collections::HashSet;

    use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadAffinityMask};

    use super::{available_cpus, bind_to_cpu_set};

    #[test]
    fn available_cpus_is_nonempty() {
        let cpus = available_cpus().expect("process affinity must be available");
        assert!(!cpus.is_empty());
    }

    #[test]
    fn binds_every_requested_cpu() {
        let available = available_cpus().expect("process affinity must be available");

        // Pick up to two CPUs to prove the *whole* set is applied, not just the
        // last one (the bug this module guards against).
        let want: HashSet<usize> = available.into_iter().take(2).collect();
        bind_to_cpu_set(&want);

        let want_mask = want.iter().fold(0usize, |m, &c| m | (1usize << c));
        // Read back the current thread's affinity: `SetThreadAffinityMask` returns
        // the *previous* mask, so re-applying the active value reports it.
        // SAFETY: pseudo-handle for the current thread; the mask is the subset we
        // just bound to.
        let active = unsafe { SetThreadAffinityMask(GetCurrentThread(), want_mask) };
        assert_eq!(active, want_mask);
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use std::collections::HashSet;

    use super::{available_cpus, bind_to_cpu_set};

    #[test]
    fn available_cpus_is_nonempty() {
        let cpus = available_cpus().expect("logical CPU count must be available");
        assert!(!cpus.is_empty());
    }

    #[test]
    fn bind_runs_cleanly() {
        // macOS only exposes an affinity *hint* (and not at all on Apple Silicon),
        // so we can only assert that binding runs without panicking.
        let available = available_cpus().unwrap();
        let want: HashSet<usize> = available.into_iter().take(2).collect();
        bind_to_cpu_set(&want);
    }
}
