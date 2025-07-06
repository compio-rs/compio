#[cfg(affinity)]
use std::collections::HashSet;

#[cfg(affinity)]
use compio_log::*;

/// Bind current thread to given cpus
#[cfg(affinity)]
pub fn bind_to_cpu_set(cpus: &HashSet<usize>) {
    use core_affinity::CoreId;
    if cpus.is_empty() {
        return;
    }

    let ids = core_affinity::get_core_ids()
        .unwrap_or_default()
        .into_iter()
        .map(|core_id| core_id.id)
        .collect::<HashSet<_>>();
    match (ids.iter().max(), cpus.iter().max()) {
        (Some(max_id), Some(max_cpu)) if *max_cpu > *max_id => {
            error!("CPU ID: {max_cpu} exceeds maximum available CPU ID: {max_id}");
        }
        _ => {}
    }
    let cpu_set = ids.intersection(&cpus);
    for cpu in cpu_set {
        let result = core_affinity::set_for_current(CoreId { id: *cpu });
        if !result {
            warn!("cannot set CPU {cpu} for current thread");
        }
    }
}

/// Bind current thread to given cpus
#[cfg(not(affinity))]
pub fn bind_to_cpu_set(_cpus: std::collections::HashSet<usize>) -> Result<(), std::io::Error> {
    Ok(())
}
