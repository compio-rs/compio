use std::{cell::Cell, future::poll_fn, io::Write, rc::Rc, task::Poll};

use compio_driver::ProactorBuilder;
use compio_io::AsyncRead;
use compio_net::TcpStream;
use compio_runtime::Runtime;

const MAX_SPINS: u32 = 100_000;

async fn yield_now() {
    let mut yielded = false;
    poll_fn(|cx| {
        if yielded {
            Poll::Ready(())
        } else {
            yielded = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    })
    .await
}

/// A completion on a DEFER_TASKRUN ring must be reaped while other tasks keep
/// the runtime busy, not only on the next blocking wait. Skipped when io_uring
/// or DEFER_TASKRUN (Linux 6.1) is unavailable.
#[test]
fn defer_taskrun_completes_while_tasks_stay_runnable() {
    let mut proactor = ProactorBuilder::new();
    proactor.single_issuer(true).defer_taskrun(true);
    let Ok(runtime) = Runtime::builder().with_proactor(proactor).build() else {
        return;
    };
    if !runtime.driver_type().is_iouring() {
        return;
    }

    runtime.block_on(async {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let mut client = std::net::TcpStream::connect(addr).unwrap();
        let (server, _) = listener.accept().unwrap();
        let mut stream = TcpStream::from_std(server).unwrap();

        let done = Rc::new(Cell::new(false));
        let busy = compio_runtime::spawn({
            let done = done.clone();
            async move {
                let mut spins = 0;
                while !done.get() && spins < MAX_SPINS {
                    spins += 1;
                    yield_now().await;
                }
                spins
            }
        });
        let reader =
            compio_runtime::spawn(async move { stream.read(Vec::with_capacity(8)).await.0 });

        for _ in 0..10 {
            yield_now().await;
        }
        client.write_all(b"ping").unwrap();

        let read = reader.await.unwrap().unwrap();
        done.set(true);
        assert_eq!(read, 4);
        assert!(busy.await.unwrap() < MAX_SPINS);
    });
}
