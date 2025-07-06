use compio_buf::BufResult;
use compio_driver::{Proactor, PushEntry, op::Asyncify};

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
