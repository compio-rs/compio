pub fn is_op_supported(code: u8) -> bool {
    #[cfg(feature = "once_cell_try")]
    use std::sync::OnceLock;

    #[cfg(not(feature = "once_cell_try"))]
    use once_cell::sync::OnceCell as OnceLock;

    static PROBE: OnceLock<io_uring::Probe> = OnceLock::new();

    PROBE
        .get_or_try_init(|| {
            let mut probe = io_uring::Probe::new();

            io_uring::IoUring::new(2)?
                .submitter()
                .register_probe(&mut probe)?;

            std::io::Result::Ok(probe)
        })
        .map(|probe| probe.is_supported(code))
        .unwrap_or_default()
}
