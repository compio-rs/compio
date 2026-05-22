use std::time::{Duration, Instant};

use compio_runtime::ResumeUnwind;

#[test]
fn cross_thread_waker_interrupts_poll() {
    let rt = compio_runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let (tx, rx) = flume::bounded::<u32>(1);

        let handle = compio_runtime::spawn(async move { rx.recv_async().await.unwrap() });

        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(10));
            tx.send(42).unwrap();
        });

        let start = Instant::now();
        let val = handle.await.resume_unwind().unwrap();
        let elapsed = start.elapsed();

        assert_eq!(val, 42);
        assert!(
            elapsed < Duration::from_millis(200),
            "took {elapsed:?}, expected < 200ms"
        );
    });
}

#[test]
fn same_thread_waker_schedules_promptly() {
    let rt = compio_runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let (tx, rx) = flume::bounded::<u32>(1);

        compio_runtime::spawn(async move {
            let val = rx.recv_async().await.unwrap();
            assert_eq!(val, 42);
        })
        .detach();

        compio_runtime::time::sleep(Duration::from_millis(10)).await;

        let start = Instant::now();
        tx.send(42).unwrap();
        compio_runtime::time::sleep(Duration::from_millis(200)).await;
        let elapsed = start.elapsed();

        assert!(
            elapsed < Duration::from_millis(400),
            "took {elapsed:?}, expected < 400ms"
        );
    });
}
