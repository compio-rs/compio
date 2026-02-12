use std::{
    net::{Ipv4Addr, SocketAddr},
    sync::Arc,
    time::{Duration, Instant},
};

use compio_buf::bytes::Bytes;
use criterion::{BenchmarkId, Criterion, Throughput};
use futures_util::{StreamExt, stream::FuturesUnordered};
use rand::{Rng, rng};

macro_rules! compio_spawn {
    ($fut:expr) => {
        compio_runtime::spawn($fut).detach()
    };
}

macro_rules! tokio_spawn {
    ($fut:expr) => {
        tokio::spawn($fut)
    };
}

macro_rules! echo_server_impl {
    ($spawn:ident, $incoming:expr) => {
        let Ok(conn) = $incoming.await else {
            continue;
        };
        $spawn!(async move {
            while let Ok((mut send, mut recv)) = conn.accept_bi().await {
                $spawn!(async move {
                    loop {
                        // These are 32 buffers, for reading approximately 32kB at once
                        let mut bufs: [Bytes; 32] = std::array::from_fn(|_| Bytes::new());

                        if let Ok(Some(n)) = recv.read_chunks(&mut bufs).await {
                            if send.write_all_chunks(&mut bufs[..n]).await.is_err() {
                                break;
                            }
                        } else {
                            break;
                        }
                    }

                    send.finish().ok();
                });
            }
        });
    };
}

fn start_compio_quic_server(
    cert: rustls::pki_types::CertificateDer<'static>,
    key_der: rustls::pki_types::PrivateKeyDer<'static>,
) -> SocketAddr {
    let (tx, rx) = flume::bounded(0);

    std::thread::spawn(move || {
        compio_runtime::Runtime::new().unwrap().block_on(async {
            let server = compio_quic::ServerBuilder::new_with_single_cert(vec![cert], key_der)
                .unwrap()
                .bind((Ipv4Addr::LOCALHOST, 0))
                .await
                .unwrap();

            tx.send(server.local_addr().unwrap()).unwrap();

            while let Some(incoming) = server.wait_incoming().await {
                echo_server_impl!(compio_spawn, incoming);
            }
        });
    });

    rx.recv().unwrap()
}

fn start_quinn_server(
    cert: rustls::pki_types::CertificateDer<'static>,
    key_der: rustls::pki_types::PrivateKeyDer<'static>,
) -> SocketAddr {
    let (tx, rx) = flume::bounded(0);

    std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let server_config =
                    quinn::ServerConfig::with_single_cert(vec![cert], key_der).unwrap();
                let server =
                    quinn::Endpoint::server(server_config, (Ipv4Addr::LOCALHOST, 0).into())
                        .unwrap();

                tx.send(server.local_addr().unwrap()).unwrap();

                while let Some(incoming) = server.accept().await {
                    echo_server_impl!(tokio_spawn, incoming);
                }
            });
    });

    rx.recv().unwrap()
}

async fn compio_quic_echo_client(
    client: &compio_quic::Endpoint,
    remote: SocketAddr,
    data: &[u8],
    iters: u64,
) -> Duration {
    let conn = client
        .connect(remote, "localhost", None)
        .unwrap()
        .await
        .unwrap();

    let start = Instant::now();
    let mut futures = (0..iters)
        .map(|_| async {
            let (send, mut recv) = conn.open_bi_wait().await.unwrap();
            let mut send = send.into_compat();
            futures_util::join!(
                async {
                    send.write_all(data).await.unwrap();
                    send.finish().unwrap();
                },
                async {
                    recv.read_to_end(vec![]).await.unwrap();
                }
            );
        })
        .collect::<FuturesUnordered<_>>();
    while futures.next().await.is_some() {}

    let elapsed = start.elapsed();

    conn.close(0u32.into(), b"done");
    conn.closed().await;

    elapsed
}

async fn quinn_echo_client(
    client: &quinn::Endpoint,
    remote: SocketAddr,
    data: &[u8],
    iters: u64,
) -> Duration {
    let conn = client.connect(remote, "localhost").unwrap().await.unwrap();

    let start = Instant::now();
    let mut futures = (0..iters)
        .map(|_| async {
            let (mut send, mut recv) = conn.open_bi().await.unwrap();
            futures_util::join!(
                async {
                    send.write_all(data).await.unwrap();
                    send.finish().unwrap();
                },
                async {
                    recv.read_to_end(usize::MAX).await.unwrap();
                }
            );
        })
        .collect::<FuturesUnordered<_>>();
    while futures.next().await.is_some() {}

    let elapsed = start.elapsed();

    conn.close(0u32.into(), b"done");
    conn.closed().await;

    elapsed
}

fn main() {
    let mut c = Criterion::default().configure_from_args();

    let rcgen::CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let cert = cert.der().clone();
    let key_der: rustls::pki_types::PrivateKeyDer = signing_key.serialize_der().try_into().unwrap();

    let compio_quic_server = start_compio_quic_server(cert.clone(), key_der.clone_key());
    let quinn_server = start_quinn_server(cert.clone(), key_der);

    let compio_rt = compio_runtime::Runtime::new().unwrap();
    let compio_quic_client = compio_rt.block_on(async {
        compio_quic::ClientBuilder::new_with_empty_roots()
            .with_custom_certificate(cert.clone())
            .unwrap()
            .with_no_crls()
            .bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap()
    });

    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let quinn_client = tokio_rt.block_on(async {
        let mut roots = rustls::RootCertStore::empty();
        roots.add(cert).unwrap();
        let client_config = quinn::ClientConfig::with_root_certificates(Arc::new(roots)).unwrap();
        let mut client = quinn::Endpoint::client((Ipv4Addr::LOCALHOST, 0).into()).unwrap();
        client.set_default_client_config(client_config);
        client
    });

    const DATA_SIZE: usize = 1024 * 1024;

    let mut rng = rng();
    let mut data = vec![0u8; DATA_SIZE];
    rng.fill_bytes(&mut data);

    let mut g = c.benchmark_group("quic-echo");
    g.throughput(Throughput::Bytes((DATA_SIZE * 2) as u64));

    for (server_name, remote) in [
        ("compio-quic-server", compio_quic_server),
        ("quinn-server", quinn_server),
    ] {
        g.bench_function(BenchmarkId::new("compio-quic", server_name), |b| {
            b.to_async(&compio_rt).iter_custom(|iters| {
                compio_quic_echo_client(&compio_quic_client, remote, &data, iters)
            });
        });

        g.bench_function(BenchmarkId::new("quinn", server_name), |b| {
            b.to_async(&tokio_rt)
                .iter_custom(|iters| quinn_echo_client(&quinn_client, remote, &data, iters));
        });
    }
    g.finish();

    c.final_summary();
}
