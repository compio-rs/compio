use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

#[test]
fn cross_thread_waker_interrupts_poll() {
    let rt = compio_runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let val = Arc::new(AtomicU32::new(0));
        let val2 = val.clone();

        let handle = compio_runtime::spawn(async move {
            loop {
                let v = val2.load(Ordering::Acquire);
                if v != 0 {
                    return v;
                }
                compio_runtime::time::sleep(Duration::from_millis(1)).await;
            }
        });

        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(10));
            val.store(42, Ordering::Release);
        });

        let start = Instant::now();
        use compio_runtime::ResumeUnwind;
        let v = handle.await.resume_unwind().unwrap();
        let elapsed = start.elapsed();

        assert_eq!(v, 42);
        assert!(
            elapsed < Duration::from_millis(500),
            "took {elapsed:?}, expected < 500ms"
        );
    });
}
