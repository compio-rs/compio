use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Instant,
};

use compio_buf::bytes::Bytes;
use criterion::{Bencher, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use futures_util::{StreamExt, stream::FuturesUnordered};
use rand::{RngCore, rng};

criterion_group!(quic, echo);
criterion_main!(quic);

fn gen_cert() -> (
    rustls::pki_types::CertificateDer<'static>,
    rustls::pki_types::PrivateKeyDer<'static>,
) {
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let cert = cert.der().clone();
    let key_der = key_pair.serialize_der().try_into().unwrap();
    (cert, key_der)
}

macro_rules! echo_impl {
    ($send:ident, $recv:ident) => {
        loop {
            // These are 32 buffers, for reading approximately 32kB at once
            let mut bufs: [Bytes; 32] = std::array::from_fn(|_| Bytes::new());

            match $recv.read_chunks(&mut bufs).await.unwrap() {
                Some(n) => {
                    $send.write_all_chunks(&mut bufs[..n]).await.unwrap();
                }
                None => break,
            }
        }

        let _ = $send.finish();
    };
}

fn echo_compio_quic(b: &mut Bencher, content: &[u8], streams: usize) {
    use compio_quic::{ClientBuilder, ServerBuilder};

    let runtime = compio_runtime::Runtime::new().unwrap();
    b.to_async(runtime).iter_custom(|iter| async move {
        let (cert, key_der) = gen_cert();
        let server = ServerBuilder::new_with_single_cert(vec![cert.clone()], key_der)
            .unwrap()
            .bind("127.0.0.1:0")
            .await
            .unwrap();
        let client = ClientBuilder::new_with_empty_roots()
            .with_custom_certificate(cert)
            .unwrap()
            .with_no_crls()
            .bind("127.0.0.1:0")
            .await
            .unwrap();
        let addr = server.local_addr().unwrap();

        let (client_conn, server_conn) = futures_util::join!(
            async move {
                client
                    .connect(addr, "localhost", None)
                    .unwrap()
                    .await
                    .unwrap()
            },
            async move { server.wait_incoming().await.unwrap().await.unwrap() }
        );

        let start = Instant::now();
        let handle = compio_runtime::spawn(async move {
            while let Ok((mut send, mut recv)) = server_conn.accept_bi().await {
                compio_runtime::spawn(async move {
                    echo_impl!(send, recv);
                })
                .detach();
            }
        });
        for _i in 0..iter {
            let mut futures = (0..streams)
                .map(|_| async {
                    let (mut send, mut recv) = client_conn.open_bi_wait().await.unwrap();
                    futures_util::join!(
                        async {
                            send.write_all(content).await.unwrap();
                            send.finish().unwrap();
                        },
                        async {
                            let mut buf = vec![];
                            recv.read_to_end(&mut buf).await.unwrap();
                        }
                    );
                })
                .collect::<FuturesUnordered<_>>();
            while futures.next().await.is_some() {}
        }
        drop(handle);
        start.elapsed()
    })
}

fn echo_quinn(b: &mut Bencher, content: &[u8], streams: usize) {
    use quinn::{ClientConfig, Endpoint, ServerConfig};

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    b.to_async(&runtime).iter_custom(|iter| async move {
        let (cert, key_der) = gen_cert();
        let server_config = ServerConfig::with_single_cert(vec![cert.clone()], key_der).unwrap();
        let mut roots = rustls::RootCertStore::empty();
        roots.add(cert).unwrap();
        let client_config = ClientConfig::with_root_certificates(Arc::new(roots)).unwrap();
        let server = Endpoint::server(
            server_config,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        )
        .unwrap();
        let mut client =
            Endpoint::client(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)).unwrap();
        client.set_default_client_config(client_config);
        let addr = server.local_addr().unwrap();

        let (client_conn, server_conn) = tokio::join!(
            async move { client.connect(addr, "localhost").unwrap().await.unwrap() },
            async move { server.accept().await.unwrap().await.unwrap() }
        );

        let start = Instant::now();
        let handle = tokio::spawn(async move {
            while let Ok((mut send, mut recv)) = server_conn.accept_bi().await {
                tokio::spawn(async move {
                    echo_impl!(send, recv);
                });
            }
        });
        for _i in 0..iter {
            let mut futures = (0..streams)
                .map(|_| async {
                    let (mut send, mut recv) = client_conn.open_bi().await.unwrap();
                    tokio::join!(
                        async {
                            send.write_all(content).await.unwrap();
                            send.finish().unwrap();
                        },
                        async {
                            recv.read_to_end(usize::MAX).await.unwrap();
                        }
                    );
                })
                .collect::<FuturesUnordered<_>>();
            while futures.next().await.is_some() {}
        }
        handle.abort();
        start.elapsed()
    });
}

const DATA_SIZES: &[usize] = &[1, 10, 1024, 1200, 1024 * 16, 1024 * 128];
const STREAMS: &[usize] = &[1, 10, 100];

fn echo(c: &mut Criterion) {
    let mut rng = rng();

    let mut data = vec![0u8; *DATA_SIZES.last().unwrap()];
    rng.fill_bytes(&mut data);

    let mut group = c.benchmark_group("echo");
    for &size in DATA_SIZES {
        let data = &data[..size];
        for &streams in STREAMS {
            group.throughput(Throughput::Bytes((data.len() * streams * 2) as u64));

            group.bench_with_input(
                BenchmarkId::new("compio-quic", format!("{}-streams-{}-bytes", streams, size)),
                &(),
                |b, _| echo_compio_quic(b, data, streams),
            );
            group.bench_with_input(
                BenchmarkId::new("quinn", format!("{}-streams-{}-bytes", streams, size)),
                &(),
                |b, _| echo_quinn(b, data, streams),
            );
        }
    }
    group.finish();
}
