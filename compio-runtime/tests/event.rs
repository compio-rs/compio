use compio_runtime::event::Event;

#[test]
fn event_handle() {
    compio_runtime::Runtime::new().unwrap().block_on(async {
        let event = Event::new();
        let handle = event.handle();
        let task = compio_runtime::spawn_blocking(move || {
            handle.notify();
        });
        event.wait().await;
        task.await;
    })
}
