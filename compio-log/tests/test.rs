use compio_log::Level;

#[test]
fn test_log() {
    compio_log::subscriber::fmt()
        .with_max_level(Level::TRACE)
        .init();

    compio_log::debug!("debug");
    compio_log::error!("error");
    compio_log::event!(Level::DEBUG, "event");
    compio_log::info!("info");
    compio_log::warn!("warn");
    compio_log::trace!("trace");
}
