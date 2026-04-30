use std::time::Duration;

use compio_runtime::time::sleep;

#[test]
#[should_panic]
fn panic_spawn() {
    compio_runtime::Runtime::new().unwrap().block_on(async {
        let _handle = compio_runtime::spawn(async {
            sleep(Duration::from_millis(100)).await;
            panic!("test panic in spawn");
        });
        sleep(Duration::from_millis(100)).await;
        panic!("another panic");
    })
}
