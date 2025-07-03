/// Bind current thread to given cpus
#[cfg(affinity)]
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
#[cfg(not(affinity))]
pub fn bind_to_cpu_set(cpus: Vec<usize>) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bind_to_cpu_set() {
        bind_to_cpu_set(vec![0]);
    }
}
