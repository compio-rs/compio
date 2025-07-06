use std::collections::HashSet;

use compio_log::*;
use core_affinity::CoreId;

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
    let cpu_set = ids.intersection(&cpus);
    for cpu in cpu_set {
        let result = core_affinity::set_for_current(CoreId { id: *cpu });
        if !result {
            warn!("cannot set CPU {cpu} for current thread");
        }
    }
}
