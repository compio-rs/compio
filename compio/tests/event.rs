use compio::event::Event;

#[compio::test]
async fn event_handle() {
    let event = Event::new().unwrap();
    let handle = event.handle().unwrap();
    std::thread::spawn(move || {
        handle.notify().unwrap();
    });
    event.wait().await.unwrap();
}
