use std::{net::Ipv4Addr, rc::Rc, time::Instant};

use criterion::{criterion_group, criterion_main, Bencher, Criterion, Throughput};
use rand::{thread_rng, RngCore};

#[cfg(target_os = "linux")]
mod monoio_wrap;
#[cfg(target_os = "linux")]
use monoio_wrap::MonoioRuntime;

criterion_group!(net, echo);
criterion_main!(net);

const BUFFER_SIZE: usize = 4096;
const BUFFER_COUNT: usize = 1024;

fn echo_tokio(b: &mut Bencher, content: &[u8; BUFFER_SIZE]) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    b.to_async(&runtime).iter_custom(|iter| async move {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();

        let start = Instant::now();
        for _i in 0..iter {
            let (mut tx, (mut rx, _)) =
                tokio::try_join!(tokio::net::TcpStream::connect(addr), listener.accept()).unwrap();

            let client = async move {
                let mut buffer = [0u8; BUFFER_SIZE];
                for _i in 0..BUFFER_COUNT {
                    tx.write_all(content).await.unwrap();
                    tx.read_exact(&mut buffer).await.unwrap();
                }
            };
            let server = async move {
                let mut buffer = [0u8; BUFFER_SIZE];
                for _i in 0..BUFFER_COUNT {
                    rx.read_exact(&mut buffer).await.unwrap();
                    rx.write_all(&buffer).await.unwrap();
                }
            };
            tokio::join!(client, server);
        }
        start.elapsed()
    })
}

fn echo_compio(b: &mut Bencher, content: Rc<[u8; BUFFER_SIZE]>) {
    use compio_io::{AsyncReadExt, AsyncWriteExt};

    let runtime = compio::runtime::Runtime::new().unwrap();
    b.to_async(&runtime).iter_custom(|iter| {
        let content = content.clone();
        async move {
            let listener = compio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
                .await
                .unwrap();
            let addr = listener.local_addr().unwrap();

            let start = Instant::now();
            for _i in 0..iter {
                let (mut tx, (mut rx, _)) = futures_util::try_join!(
                    compio::net::TcpStream::connect(addr),
                    listener.accept()
                )
                .unwrap();

                let client = async {
                    let mut content = content.clone();
                    let mut buffer = Box::new([0u8; BUFFER_SIZE]);
                    for _i in 0..BUFFER_COUNT {
                        (_, content) = tx.write_all(content).await.unwrap();
                        (_, buffer) = tx.read_exact(buffer).await.unwrap();
                    }
                };
                let server = async move {
                    let mut buffer = Box::new([0u8; BUFFER_SIZE]);
                    for _i in 0..BUFFER_COUNT {
                        (_, buffer) = rx.read_exact(buffer).await.unwrap();
                        (_, buffer) = rx.write_all(buffer).await.unwrap();
                    }
                };
                futures_util::join!(client, server);
            }
            start.elapsed()
        }
    })
}

#[cfg(target_os = "linux")]
fn echo_monoio(b: &mut Bencher, content: &[u8; BUFFER_SIZE]) {
    use monoio::io::{AsyncReadRentExt, AsyncWriteRentExt};

    let runtime = MonoioRuntime::new();
    b.to_async(&runtime).iter_custom(|iter| {
        let content = Box::new(*content);
        async move {
            let listener = monoio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            let addr = listener.local_addr().unwrap();

            let start = Instant::now();
            for _i in 0..iter {
                let (mut tx, (mut rx, _)) = futures_util::try_join!(
                    monoio::net::TcpStream::connect(addr),
                    listener.accept()
                )
                .unwrap();

                let client = async {
                    let mut content = content.clone();
                    let mut buffer = Box::new([0u8; BUFFER_SIZE]);
                    let mut res;
                    for _i in 0..BUFFER_COUNT {
                        (res, content) = tx.write_all(content).await;
                        res.unwrap();
                        (res, buffer) = tx.read_exact(buffer).await;
                        res.unwrap();
                    }
                };
                let server = async move {
                    let mut buffer = Box::new([0u8; BUFFER_SIZE]);
                    let mut res;
                    for _i in 0..BUFFER_COUNT {
                        (res, buffer) = rx.read_exact(buffer).await;
                        res.unwrap();
                        (res, buffer) = rx.write_all(buffer).await;
                        res.unwrap();
                    }
                };
                futures_util::join!(client, server);
            }
            start.elapsed()
        }
    })
}

fn echo(c: &mut Criterion) {
    let mut rng = thread_rng();

    let mut content = [0u8; BUFFER_SIZE];
    rng.fill_bytes(&mut content);
    let content = Rc::new(content);

    let mut group = c.benchmark_group("echo");
    group.throughput(Throughput::Bytes((BUFFER_SIZE * BUFFER_COUNT * 2) as u64));

    group.bench_function("tokio", |b| echo_tokio(b, &content));
    group.bench_function("compio", |b| echo_compio(b, content.clone()));
    #[cfg(target_os = "linux")]
    group.bench_function("monoio", |b| echo_monoio(b, &content));

    group.finish();
}
