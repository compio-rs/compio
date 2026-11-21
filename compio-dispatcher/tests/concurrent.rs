use std::{
    num::NonZeroUsize,
    sync::{Arc, Mutex},
    time::Duration,
};

use compio_dispatcher::Dispatcher;
use compio_runtime::time::timeout;
use futures_channel::oneshot;

#[derive(Debug, Clone, PartialEq)]
enum Step {
    AStart,
    AEnd,
    B,
}

/// Dispatch task `a` and then task `b` to a single worker, and return the
/// order in which they were processed.
/// A waiting period (at most `b_timeout`) is set up before task `a` completes.
/// The purpose of this function is to find out whether `b` starts during it.
async fn execution_order(concurrent: bool, b_timeout: Duration) -> Vec<Step> {
    let dispatcher = Dispatcher::builder()
        .worker_threads(NonZeroUsize::new(1).unwrap())
        .concurrent(concurrent)
        .build()
        .unwrap();
    let order = Arc::new(Mutex::new(Vec::new()));
    let (started_tx, started_rx) = oneshot::channel::<()>();
    let (release_tx, release_rx) = oneshot::channel::<()>();

    let a = dispatcher
        .dispatch({
            let order = order.clone();
            async move || {
                order.lock().unwrap().push(Step::AStart);
                started_tx.send(()).unwrap();
                release_rx.await.unwrap();
                order.lock().unwrap().push(Step::AEnd);
            }
        })
        .unwrap();
    started_rx.await.unwrap();

    let mut b = dispatcher
        .dispatch({
            let order = order.clone();
            async move || {
                order.lock().unwrap().push(Step::B);
            }
        })
        .unwrap();

    // With `a` suspended, wait for either `b` to finish or the timeout.
    let b_finished = timeout(b_timeout, &mut b).await.is_ok();

    // Let `a` proceed.
    release_tx.send(()).unwrap();
    a.await.unwrap();

    if !b_finished {
        b.await.unwrap();
    }
    dispatcher.join().await.unwrap();

    order.lock().unwrap().clone()
}

/// Wait 100 ms and check that `b` does not start before `a` completes.
/// The timeout is expected to elapse.
#[compio_macros::test]
async fn sequential_finishes_a_task_before_starting_the_next() {
    assert_eq!(
        execution_order(false, Duration::from_millis(100)).await,
        [Step::AStart, Step::AEnd, Step::B]
    );
}

/// Check that `b` starts even though `a` has not completed.
/// The timeout is 10 seconds, but `b` is expected to finish before it.
#[compio_macros::test]
async fn concurrent_starts_the_next_task_while_one_is_suspended() {
    assert_eq!(
        execution_order(true, Duration::from_secs(10)).await,
        [Step::AStart, Step::B, Step::AEnd]
    );
}
