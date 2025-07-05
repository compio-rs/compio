/// Bind current thread to given cpus
#[cfg(affinity)]
pub fn bind_to_cpu_set(cpus: Vec<usize>) -> Result<(), std::io::Error> {
    use core_affinity::CoreId;

    let ids = core_affinity::get_core_ids().ok_or_else(|| {
        std::io::Error::last_os_error()
    })?;
    ids.into_iter()
        .zip(cpus)
        .filter_map(
            |(CoreId { id }, cpu)| {
                if cpu == id { Some(cpu) } else { None }
            },
        )
        .try_for_each(|id| {
            let result = core_affinity::set_for_current(CoreId { id });
            if !result {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        })
}

/// Bind current thread to given cpus
#[cfg(not(affinity))]
pub fn bind_to_cpu_set(cpus: Vec<usize>) -> Result<(), std::io::Error> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bind_to_cpu_set() {
        assert!(bind_to_cpu_set(vec![0]).is_ok());
        assert!(bind_to_cpu_set(vec![2]).is_ok());
    }
}
