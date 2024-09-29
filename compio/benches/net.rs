use std::{net::Ipv4Addr, rc::Rc, time::Instant};

use criterion::{Bencher, Criterion, Throughput, criterion_group, criterion_main};
use rand::{RngCore, thread_rng};

#[cfg(target_os = "linux")]
mod monoio_wrap;
#[cfg(target_os = "linux")]
use monoio_wrap::MonoioRuntime;

criterion_group!(net, echo);
criterion_main!(net);

const BUFFER_SIZE: usize = 4096;
const BUFFER_COUNT: usize = 1024;

async fn echo_tokio_impl<T, R>(mut tx: T, mut rx: R, content: &[u8; BUFFER_SIZE])
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    R: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

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

async fn echo_compio_impl<T, R>(mut tx: T, mut rx: R, mut content: Rc<[u8; BUFFER_SIZE]>)
where
    T: compio::io::AsyncRead + compio::io::AsyncWrite,
    R: compio::io::AsyncRead + compio::io::AsyncWrite,
{
    use compio::io::{AsyncReadExt, AsyncWriteExt};

    let client = async move {
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

#[cfg(target_os = "linux")]
async fn echo_monoio_impl<T, R>(mut tx: T, mut rx: R, mut content: Box<[u8; BUFFER_SIZE]>)
where
    T: monoio::io::AsyncReadRent + monoio::io::AsyncWriteRent,
    R: monoio::io::AsyncReadRent + monoio::io::AsyncWriteRent,
{
    use monoio::io::{AsyncReadRentExt, AsyncWriteRentExt};

    let client = async {
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

fn echo_tokio_tcp(b: &mut Bencher, content: &[u8; BUFFER_SIZE]) {
    use tokio::net::{TcpListener, TcpStream};

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    b.to_async(&runtime).iter_custom(|iter| async move {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();

        let start = Instant::now();
        for _i in 0..iter {
            let (tx, (rx, _)) =
                tokio::try_join!(TcpStream::connect(addr), listener.accept()).unwrap();
            echo_tokio_impl(tx, rx, content).await;
        }
        start.elapsed()
    })
}

fn echo_compio_tcp(b: &mut Bencher, content: Rc<[u8; BUFFER_SIZE]>) {
    use compio::net::{TcpListener, TcpStream};

    let runtime = compio::runtime::Runtime::new().unwrap();
    b.to_async(&runtime).iter_custom(|iter| {
        let content = content.clone();
        async move {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let addr = listener.local_addr().unwrap();

            let start = Instant::now();
            for _i in 0..iter {
                let (tx, (rx, _)) =
                    futures_util::try_join!(TcpStream::connect(addr), listener.accept()).unwrap();
                echo_compio_impl(tx, rx, content.clone()).await;
            }
            start.elapsed()
        }
    })
}

#[cfg(target_os = "linux")]
fn echo_monoio_tcp(b: &mut Bencher, content: &[u8; BUFFER_SIZE]) {
    use monoio::net::{TcpListener, TcpStream};

    let runtime = MonoioRuntime::new();
    b.to_async(&runtime).iter_custom(|iter| {
        let content = Box::new(*content);
        async move {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            let addr = listener.local_addr().unwrap();

            let start = Instant::now();
            for _i in 0..iter {
                let (tx, (rx, _)) =
                    futures_util::try_join!(TcpStream::connect(addr), listener.accept()).unwrap();
                echo_monoio_impl(tx, rx, content.clone()).await;
            }
            start.elapsed()
        }
    })
}

#[cfg(windows)]
fn echo_tokio_win_named_pipe(b: &mut Bencher, content: &[u8; BUFFER_SIZE]) {
    use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    b.to_async(&runtime).iter_custom(|iter| async move {
        const PIPE_NAME: &str = r"\\.\pipe\tokio-named-pipe";

        let start = Instant::now();
        for _i in 0..iter {
            let rx = ServerOptions::new().create(PIPE_NAME).unwrap();
            let tx = ClientOptions::new().open(PIPE_NAME).unwrap();

            rx.connect().await.unwrap();

            echo_tokio_impl(tx, rx, content).await;
        }
        start.elapsed()
    })
}

#[cfg(windows)]
fn echo_compio_win_named_pipe(b: &mut Bencher, content: Rc<[u8; BUFFER_SIZE]>) {
    use compio::fs::named_pipe::{ClientOptions, ServerOptions};

    let runtime = compio::runtime::Runtime::new().unwrap();
    b.to_async(&runtime).iter_custom(|iter| {
        let content = content.clone();
        async move {
            const PIPE_NAME: &str = r"\\.\pipe\compio-named-pipe";

            let start = Instant::now();
            let options = ClientOptions::new();
            for _i in 0..iter {
                let rx = ServerOptions::new().create(PIPE_NAME).unwrap();
                let (tx, ()) =
                    futures_util::try_join!(options.open(PIPE_NAME), rx.connect()).unwrap();
                echo_compio_impl(tx, rx, content.clone()).await;
            }
            start.elapsed()
        }
    })
}

#[cfg(unix)]
fn echo_tokio_unix(b: &mut Bencher, content: &[u8; BUFFER_SIZE]) {
    use tokio::net::{UnixListener, UnixStream};

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    b.to_async(&runtime).iter_custom(|iter| async move {
        let dir = tempfile::Builder::new()
            .prefix("tokio-uds")
            .tempdir()
            .unwrap();
        let sock_path = dir.path().join("connect.sock");
        let listener = UnixListener::bind(&sock_path).unwrap();

        let start = Instant::now();
        for _i in 0..iter {
            let (tx, (rx, _)) =
                tokio::try_join!(UnixStream::connect(&sock_path), listener.accept()).unwrap();
            echo_tokio_impl(tx, rx, content).await;
        }
        start.elapsed()
    })
}

fn echo_compio_unix(b: &mut Bencher, content: Rc<[u8; BUFFER_SIZE]>) {
    use compio::net::{UnixListener, UnixStream};

    let runtime = compio::runtime::Runtime::new().unwrap();
    b.to_async(&runtime).iter_custom(|iter| {
        let content = content.clone();
        async move {
            let dir = tempfile::Builder::new()
                .prefix("compio-uds")
                .tempdir()
                .unwrap();
            let sock_path = dir.path().join("connect.sock");
            let listener = UnixListener::bind(&sock_path).await.unwrap();

            let start = Instant::now();
            for _i in 0..iter {
                let (tx, (rx, _)) =
                    futures_util::try_join!(UnixStream::connect(&sock_path), listener.accept())
                        .unwrap();
                echo_compio_impl(tx, rx, content.clone()).await;
            }
            start.elapsed()
        }
    })
}

#[cfg(target_os = "linux")]
fn echo_monoio_unix(b: &mut Bencher, content: &[u8; BUFFER_SIZE]) {
    use monoio::net::{UnixListener, UnixStream};

    let runtime = MonoioRuntime::new();
    b.to_async(&runtime).iter_custom(|iter| {
        let content = Box::new(*content);
        async move {
            let dir = tempfile::Builder::new()
                .prefix("monoio-uds")
                .tempdir()
                .unwrap();
            let sock_path = dir.path().join("connect.sock");
            let listener = UnixListener::bind(&sock_path).unwrap();

            let start = Instant::now();
            for _i in 0..iter {
                let (tx, (rx, _)) =
                    futures_util::try_join!(UnixStream::connect(&sock_path), listener.accept())
                        .unwrap();
                echo_monoio_impl(tx, rx, content.clone()).await;
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

    group.bench_function("tokio-tcp", |b| echo_tokio_tcp(b, &content));
    group.bench_function("compio-tcp", |b| echo_compio_tcp(b, content.clone()));
    #[cfg(target_os = "linux")]
    group.bench_function("monoio-tcp", |b| echo_monoio_tcp(b, &content));

    #[cfg(windows)]
    group.bench_function("tokio-pipe", |b| echo_tokio_win_named_pipe(b, &content));
    #[cfg(windows)]
    group.bench_function("compio-pipe", |b| {
        echo_compio_win_named_pipe(b, content.clone())
    });

    #[cfg(unix)]
    group.bench_function("tokio-unix", |b| echo_tokio_unix(b, &content));
    group.bench_function("compio-unix", |b| echo_compio_unix(b, content.clone()));
    #[cfg(target_os = "linux")]
    group.bench_function("monoio-unix", |b| echo_monoio_unix(b, &content));

    group.finish();
}
