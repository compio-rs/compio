#![cfg_attr(feature = "sanitize", feature(cfg_sanitize))]

use std::time::Duration;

use compio_runtime::time::sleep;

#[test]
#[should_panic]
#[cfg_attr(feature = "sanitize", cfg_attr(sanitize = "address", ignore))]
fn simple_drop() {
    compio_runtime::Runtime::new().unwrap().block_on(async {
        let handle = compio_runtime::spawn(async {
            sleep(Duration::from_millis(100)).await;
        });
        // Register the waker for handle here to make sure that when the task gets
        // dropped, `has_waker` check returns true
        compio_runtime::spawn(handle).detach();
        sleep(Duration::from_millis(100)).await;
        panic!("Main future panics");
    })
}
