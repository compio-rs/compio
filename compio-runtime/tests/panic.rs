use std::{
    cell::Cell,
    panic::{AssertUnwindSafe, catch_unwind},
    rc::Rc,
    time::Duration,
};

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

#[test]
fn timer_construction_does_not_require_runtime() {
    let timer = sleep(Duration::ZERO);
    compio_runtime::Runtime::new().unwrap().block_on(timer);
}

struct DropGuard(Rc<Cell<bool>>);

impl Drop for DropGuard {
    fn drop(&mut self) {
        self.0.set(true);
    }
}

#[test]
fn panic_clears_spawned_tasks() {
    let dropped = Rc::new(Cell::new(false));
    let observed = dropped.clone();
    let runtime = compio_runtime::Runtime::new().unwrap();

    let result = catch_unwind(AssertUnwindSafe(|| {
        runtime.block_on(async move {
            compio_runtime::spawn(async move {
                let _guard = DropGuard(dropped);
                sleep(Duration::from_secs(1)).await;
            })
            .detach();

            sleep(Duration::from_millis(10)).await;
            panic!("panic while a spawned task is pending");
        });
    }));

    let dropped_before_runtime_drop = observed.get();
    drop(runtime);

    assert!(result.is_err());
    assert!(
        dropped_before_runtime_drop,
        "pending task was not dropped before the panic resumed"
    );
}
