use compio_buf::BufResult;
use compio_driver::{Key, OpCode, Proactor, PushEntry, op::Asyncify};

fn take_key<T: OpCode, R>(res: PushEntry<Key<T>, R>) -> Key<T> {
    match res {
        PushEntry::Pending(key) => key,
        PushEntry::Ready(_) => {
            unreachable!()
        }
    }
}

fn wait_for<T: OpCode>(driver: &mut Proactor, mut key: Key<T>) -> BufResult<usize, T> {
    loop {
        driver.poll(None).unwrap();
        match driver.pop(key) {
            PushEntry::Pending(k) => key = k,
            PushEntry::Ready(res) => break res,
        }
    }
}

#[test]
#[should_panic(expected = "the thread pool is needed but no worker thread is running")]
fn disable() {
    let mut driver = Proactor::builder().thread_pool_limit(0).build().unwrap();

    match driver.push(Asyncify::new(|| -> BufResult<usize, ()> {
        panic!("the asyncify operation should not be executed when the thread pool is disabled");
    })) {
        PushEntry::Pending(_) => {}
        PushEntry::Ready(_) => {
            unreachable!()
        }
    }

    driver.poll(None).unwrap();
}

#[test]
fn small_pool() {
    let mut driver = Proactor::builder().thread_pool_limit(1).build().unwrap();
    let clo = || {
        std::thread::sleep(std::time::Duration::from_secs(1));
        BufResult(Ok(0), ())
    };

    let a = take_key(driver.push(Asyncify::new(clo)));
    let b = take_key(driver.push(Asyncify::new(clo)));

    wait_for(&mut driver, a).unwrap();
    wait_for(&mut driver, b).unwrap();
}
