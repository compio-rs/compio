use std::collections::HashSet;

use compio_log::*;

/// Bind current thread to given cpus
pub fn bind_to_cpu_set(cpus: &HashSet<usize>) {
    if cpus.is_empty() {
        return;
    }

    let Some(ids) = core_affinity::get_core_ids() else {
        return;
    };

    let ids = ids
        .into_iter()
        .map(|core_id| core_id.id)
        .collect::<HashSet<_>>();
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

#[cfg(windows)]
fn set_affinity(cpus: impl Iterator<Item = usize>) {
    use std::io;

    use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadAffinityMask};

    // A thread affinity mask addresses at most one processor group (64 CPUs).
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

// macOS/iOS, illumos/Solaris and others do not expose an API to pin a thread to
// a set of CPUs, so fall back to best-effort single-CPU binding.
#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_os = "freebsd",
    windows,
)))]
fn set_affinity(cpus: impl Iterator<Item = usize>) {
    use core_affinity::CoreId;

    for cpu in cpus {
        if !core_affinity::set_for_current(CoreId { id: cpu }) {
            warn!("cannot set CPU {cpu} for current thread");
        }
    }
}
