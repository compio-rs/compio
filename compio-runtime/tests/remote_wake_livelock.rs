use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use compio_runtime::Runtime;

/// Number of concurrent tasks to be sent between the two threads
const CALLERS: usize = 128;
/// Sync queue size
const SYNC_QUEUE_SIZE: usize = 32;

/// Requests each caller task issues.
const ROUNDS: usize = 200;
/// If both peers haven't finished by now, they're deadlocked.
const WATCHDOG: Duration = Duration::from_secs(60);

struct Request {
    reply: oneshot::Sender<()>,
}

fn spawn_peer(
    name: &'static str,
    inbox: flume::Receiver<Request>,
    outbox: flume::Sender<Request>,
    done: Arc<AtomicUsize>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            let rt = Runtime::builder()
                .sync_queue_size(SYNC_QUEUE_SIZE)
                .build()
                .unwrap();

            rt.block_on(async move {
                // Handle requests from the peer
                let worker = compio_runtime::spawn(async move {
                    while let Ok(req) = inbox.recv_async().await {
                        compio_runtime::spawn(async move {
                            let _ = req.reply.send(());
                        })
                        .detach();
                    }
                });

                // Send requests to the peer
                let mut callers = Vec::with_capacity(CALLERS);
                for _ in 0..CALLERS {
                    let outbox = outbox.clone();
                    callers.push(compio_runtime::spawn(async move {
                        for _ in 0..ROUNDS {
                            let (reply, wait) = oneshot::channel();
                            if outbox.send(Request { reply }).is_err() {
                                break;
                            }
                            let _ = wait.await;
                        }
                    }));
                }

                for c in callers {
                    let _ = c.await;
                }

                // Done sending: drop our outbound sender so the peer's worker
                // sees the channel disconnect and exits, then wait for our own
                // worker to drain the peer's final requests.
                drop(outbox);
                let _ = worker.await;
            });
            done.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap()
}

#[test]
fn cross_thread_channel_rpc_does_not_livelock() {
    // chan_a: peer A is the worker (a_rx); peer B sends into it (a_tx).
    // chan_b: peer B is the worker (b_rx); peer A sends into it (b_tx).
    let (a_tx, a_rx) = flume::unbounded::<Request>();
    let (b_tx, b_rx) = flume::unbounded::<Request>();

    let done = Arc::new(AtomicUsize::new(0));

    // Peer A: worker on a_rx, caller into b_tx.
    let thread_a = spawn_peer("peer-a", a_rx, b_tx, done.clone());
    // Peer B: worker on b_rx, caller into a_tx.
    let thread_b = spawn_peer("peer-b", b_rx, a_tx, done.clone());

    let start = Instant::now();
    while done.load(Ordering::SeqCst) < 2 {
        assert!(
            start.elapsed() < WATCHDOG,
            "cross-thread channel-RPC livelock: both runtimes stuck spinning in \
             Remote::schedule with full sync queues ({} of 2 finished)",
            done.load(Ordering::SeqCst)
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    thread_a.join().unwrap();
    thread_b.join().unwrap();
}
