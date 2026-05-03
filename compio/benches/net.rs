use std::{net::Ipv4Addr, time::Instant};

use criterion::{Bencher, Criterion, Throughput, criterion_group, criterion_main};
use rand::{Rng, rng};

#[cfg(all(target_os = "linux", target_env = "gnu"))]
mod monoio_wrap;
#[cfg(all(target_os = "linux", target_env = "gnu"))]
use monoio_wrap::MonoioRuntime;

criterion_group!(net, echo);
criterion_main!(net);

const BUFFER_SIZE: usize = 524288;
const BUFFER_COUNT: usize = 8;

async fn echo_tokio_impl<T, R>(
    mut tx: T,
    mut rx: R,
    content: &[u8],
    client_buffer: &mut [u8],
    server_buffer: &mut [u8],
) where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    R: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let client = async move {
        for _i in 0..BUFFER_COUNT {
            tx.write_all(content).await.unwrap();
            tx.read_exact(client_buffer).await.unwrap();
        }
    };
    let server = async move {
        for _i in 0..BUFFER_COUNT {
            rx.read_exact(server_buffer).await.unwrap();
            rx.write_all(server_buffer).await.unwrap();
        }
    };
    tokio::join!(client, server);
}

async fn echo_compio_impl<T, R>(
    mut tx: T,
    mut rx: R,
    mut content: Vec<u8>,
    mut client_buffer: Vec<u8>,
    mut server_buffer: Vec<u8>,
) -> (Vec<u8>, Vec<u8>, Vec<u8>)
where
    T: compio::io::AsyncRead + compio::io::AsyncWrite,
    R: compio::io::AsyncRead + compio::io::AsyncWrite,
{
    use compio::io::{AsyncReadExt, AsyncWriteExt};

    let client = async move {
        for _i in 0..BUFFER_COUNT {
            (_, content) = tx.write_all(content).await.unwrap();
            (_, client_buffer) = tx.read_exact(client_buffer).await.unwrap();
        }
        (content, client_buffer)
    };
    let server = async move {
        for _i in 0..BUFFER_COUNT {
            (_, server_buffer) = rx.read_exact(server_buffer).await.unwrap();
            (_, server_buffer) = rx.write_all(server_buffer).await.unwrap();
        }
        server_buffer
    };
    let ((content, client_buffer), server_buffer) = futures_util::join!(client, server);
    (content, client_buffer, server_buffer)
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
async fn echo_monoio_impl<T, R>(
    mut tx: T,
    mut rx: R,
    mut content: Vec<u8>,
    mut client_buffer: Vec<u8>,
    mut server_buffer: Vec<u8>,
) -> (Vec<u8>, Vec<u8>, Vec<u8>)
where
    T: monoio::io::AsyncReadRent + monoio::io::AsyncWriteRent,
    R: monoio::io::AsyncReadRent + monoio::io::AsyncWriteRent,
{
    use monoio::io::{AsyncReadRentExt, AsyncWriteRentExt};

    let client = async {
        let mut res;
        for _i in 0..BUFFER_COUNT {
            (res, content) = tx.write_all(content).await;
            res.unwrap();
            (res, client_buffer) = tx.read_exact(client_buffer).await;
            res.unwrap();
        }
        (content, client_buffer)
    };
    let server = async move {
        let mut res;
        for _i in 0..BUFFER_COUNT {
            (res, server_buffer) = rx.read_exact(server_buffer).await;
            res.unwrap();
            (res, server_buffer) = rx.write_all(server_buffer).await;
            res.unwrap();
        }
        server_buffer
    };
    let ((content, client_buffer), server_buffer) = futures_util::join!(client, server);
    (content, client_buffer, server_buffer)
}

fn echo_tokio_tcp(b: &mut Bencher, content: &[u8]) {
    use tokio::net::{TcpListener, TcpStream};

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    b.to_async(&runtime).iter_custom(|iter| async move {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();

        let mut client_buffer = vec![0u8; BUFFER_SIZE];
        let mut server_buffer = vec![0u8; BUFFER_SIZE];

        let start = Instant::now();
        for _i in 0..iter {
            let (tx, (rx, _)) =
                tokio::try_join!(TcpStream::connect(addr), listener.accept()).unwrap();
            echo_tokio_impl(tx, rx, content, &mut client_buffer, &mut server_buffer).await;
        }
        start.elapsed()
    })
}

fn echo_compio_tcp(b: &mut Bencher, content: Vec<u8>) {
    use compio::net::{TcpListener, TcpStream};

    let runtime = compio::runtime::Runtime::new().unwrap();
    b.to_async(&runtime).iter_custom(|iter| {
        let mut content = content.clone();
        async move {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            let addr = listener.local_addr().unwrap();

            let mut client_buffer = vec![0u8; BUFFER_SIZE];
            let mut server_buffer = vec![0u8; BUFFER_SIZE];

            let start = Instant::now();
            for _i in 0..iter {
                let (tx, (rx, _)) =
                    futures_util::try_join!(TcpStream::connect(addr), listener.accept()).unwrap();
                (content, client_buffer, server_buffer) =
                    echo_compio_impl(tx, rx, content, client_buffer, server_buffer).await;
            }
            start.elapsed()
        }
    })
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn echo_monoio_tcp(b: &mut Bencher, content: Vec<u8>) {
    use monoio::net::{TcpListener, TcpStream};

    let runtime = MonoioRuntime::new();
    b.to_async(&runtime).iter_custom(|iter| {
        let mut content = content.clone();
        async move {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            let addr = listener.local_addr().unwrap();

            let mut client_buffer = vec![0u8; BUFFER_SIZE];
            let mut server_buffer = vec![0u8; BUFFER_SIZE];

            let start = Instant::now();
            for _i in 0..iter {
                let (tx, (rx, _)) =
                    futures_util::try_join!(TcpStream::connect(addr), listener.accept()).unwrap();
                (content, client_buffer, server_buffer) =
                    echo_monoio_impl(tx, rx, content, client_buffer, server_buffer).await;
            }
            start.elapsed()
        }
    })
}

#[cfg(windows)]
fn echo_tokio_win_named_pipe(b: &mut Bencher, content: &[u8]) {
    use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    b.to_async(&runtime).iter_custom(|iter| async move {
        const PIPE_NAME: &str = r"\\.\pipe\tokio-named-pipe";

        let mut client_buffer = vec![0u8; BUFFER_SIZE];
        let mut server_buffer = vec![0u8; BUFFER_SIZE];

        let start = Instant::now();
        for _i in 0..iter {
            let rx = ServerOptions::new().create(PIPE_NAME).unwrap();
            let tx = ClientOptions::new().open(PIPE_NAME).unwrap();

            rx.connect().await.unwrap();

            echo_tokio_impl(tx, rx, content, &mut client_buffer, &mut server_buffer).await;
        }
        start.elapsed()
    })
}

#[cfg(windows)]
fn echo_compio_win_named_pipe(b: &mut Bencher, content: Vec<u8>) {
    use compio::fs::named_pipe::{ClientOptions, ServerOptions};

    let runtime = compio::runtime::Runtime::new().unwrap();
    b.to_async(&runtime).iter_custom(|iter| {
        let mut content = content.clone();
        async move {
            const PIPE_NAME: &str = r"\\.\pipe\compio-named-pipe";

            let mut client_buffer = vec![0u8; BUFFER_SIZE];
            let mut server_buffer = vec![0u8; BUFFER_SIZE];

            let start = Instant::now();
            let options = ClientOptions::new();
            for _i in 0..iter {
                let rx = ServerOptions::new().create(PIPE_NAME).unwrap();
                let (tx, ()) =
                    futures_util::try_join!(options.open(PIPE_NAME), rx.connect()).unwrap();
                (content, client_buffer, server_buffer) =
                    echo_compio_impl(tx, rx, content, client_buffer, server_buffer).await;
            }
            start.elapsed()
        }
    })
}

#[cfg(unix)]
fn echo_tokio_unix(b: &mut Bencher, content: &[u8]) {
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

        let mut client_buffer = vec![0u8; BUFFER_SIZE];
        let mut server_buffer = vec![0u8; BUFFER_SIZE];

        let start = Instant::now();
        for _i in 0..iter {
            let (tx, (rx, _)) =
                tokio::try_join!(UnixStream::connect(&sock_path), listener.accept()).unwrap();
            echo_tokio_impl(tx, rx, content, &mut client_buffer, &mut server_buffer).await;
        }
        start.elapsed()
    })
}

fn echo_compio_unix(b: &mut Bencher, content: Vec<u8>) {
    use compio::net::{UnixListener, UnixStream};

    let runtime = compio::runtime::Runtime::new().unwrap();
    b.to_async(&runtime).iter_custom(|iter| {
        let mut content = content.clone();
        async move {
            let dir = tempfile::Builder::new()
                .prefix("compio-uds")
                .tempdir()
                .unwrap();
            let sock_path = dir.path().join("connect.sock");
            let listener = UnixListener::bind(&sock_path).await.unwrap();

            let mut client_buffer = vec![0u8; BUFFER_SIZE];
            let mut server_buffer = vec![0u8; BUFFER_SIZE];

            let start = Instant::now();
            for _i in 0..iter {
                let (tx, (rx, _)) =
                    futures_util::try_join!(UnixStream::connect(&sock_path), listener.accept())
                        .unwrap();
                (content, client_buffer, server_buffer) =
                    echo_compio_impl(tx, rx, content, client_buffer, server_buffer).await;
            }
            start.elapsed()
        }
    })
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn echo_monoio_unix(b: &mut Bencher, content: Vec<u8>) {
    use monoio::net::{ListenerOpts, UnixListener, UnixStream};

    let runtime = MonoioRuntime::new();
    b.to_async(&runtime).iter_custom(|iter| {
        let mut content = content.clone();
        async move {
            let dir = tempfile::Builder::new()
                .prefix("monoio-uds")
                .tempdir()
                .unwrap();
            let sock_path = dir.path().join("connect.sock");
            let listener = UnixListener::bind_with_config(
                &sock_path,
                &ListenerOpts::default().reuse_addr(false).reuse_port(false),
            )
            .unwrap();

            let mut client_buffer = vec![0u8; BUFFER_SIZE];
            let mut server_buffer = vec![0u8; BUFFER_SIZE];

            let start = Instant::now();
            for _i in 0..iter {
                let (tx, (rx, _)) =
                    futures_util::try_join!(UnixStream::connect(&sock_path), listener.accept())
                        .unwrap();
                (content, client_buffer, server_buffer) =
                    echo_monoio_impl(tx, rx, content, client_buffer, server_buffer).await;
            }
            start.elapsed()
        }
    })
}

fn echo(c: &mut Criterion) {
    let mut rng = rng();

    let mut content = vec![0u8; BUFFER_SIZE];
    rng.fill_bytes(&mut content);

    let mut group = c.benchmark_group("echo");
    group.throughput(Throughput::Bytes((BUFFER_SIZE * BUFFER_COUNT * 2) as u64));

    group.bench_function("tokio-tcp", |b| echo_tokio_tcp(b, &content));
    group.bench_function("compio-tcp", |b| echo_compio_tcp(b, content.clone()));
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    group.bench_function("monoio-tcp", |b| echo_monoio_tcp(b, content.clone()));

    #[cfg(windows)]
    group.bench_function("tokio-pipe", |b| echo_tokio_win_named_pipe(b, &content));
    #[cfg(windows)]
    group.bench_function("compio-pipe", |b| {
        echo_compio_win_named_pipe(b, content.clone())
    });

    #[cfg(unix)]
    group.bench_function("tokio-unix", |b| echo_tokio_unix(b, &content));
    group.bench_function("compio-unix", |b| echo_compio_unix(b, content.clone()));
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    group.bench_function("monoio-unix", |b| echo_monoio_unix(b, content.clone()));

    group.finish();
}
