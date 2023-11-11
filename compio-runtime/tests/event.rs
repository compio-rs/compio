use compio_runtime::event::Event;

#[test]
fn event_handle() {
    compio_runtime::Runtime::new().unwrap().block_on(async {
        let event = Event::new().unwrap();
        let handle = event.handle().unwrap();
        std::thread::spawn(move || {
            handle.notify().unwrap();
        });
        event.wait().await.unwrap();
    })
}
