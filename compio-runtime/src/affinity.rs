#[cfg(affinity)]
use std::collections::HashSet;

/// Bind current thread to given cpus
#[cfg(affinity)]
pub fn bind_to_cpu_set(cpus: HashSet<usize>) -> Result<(), std::io::Error> {
    use core_affinity::CoreId;
    if cpus.is_empty() {
        return Ok(());
    }

    let ids = core_affinity::get_core_ids()
        .ok_or_else(std::io::Error::last_os_error)?
        .into_iter()
        .map(|core_id| core_id.id)
        .collect::<HashSet<_>>();
    match (ids.iter().max(), cpus.iter().max()) {
        (Some(max_id), Some(max_cpu)) if *max_cpu > *max_id => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("CPU ID: {max_cpu} exceeds maximum available CPU ID: {max_id}"),
            ));
        }
        _ => {}
    }
    let cpu_set = ids.intersection(&cpus);
    for cpu in cpu_set {
        let result = core_affinity::set_for_current(CoreId { id: *cpu });
        if !result {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Bind current thread to given cpus
#[cfg(not(affinity))]
pub fn bind_to_cpu_set(_cpus: std::collections::HashSet<usize>) -> Result<(), std::io::Error> {
    Ok(())
}

#[cfg(all(test, affinity))]
mod tests {
    use super::*;

    #[test]
    fn test_bind_to_cpu_set() {
        assert!(bind_to_cpu_set(HashSet::from([0])).is_ok());
        assert!(bind_to_cpu_set(HashSet::from([10000])).is_err());
    }
}
