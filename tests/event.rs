use compio::event::Event;

#[test]
fn event_handle() {
    let event = Event::new().unwrap();

    std::thread::scope(|scope| {
        let handle = event.handle();
        let wait = event.wait();
        scope.spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(1));
            handle.notify().unwrap()
        });
        scope.spawn(move || {
            compio::task::block_on(async {
                wait.await.unwrap();
            })
        });
    });
}
