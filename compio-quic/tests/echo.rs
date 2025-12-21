use std::{
    array,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
};

use compio_buf::bytes::Bytes;
use compio_io::AsyncWriteExt;
use compio_quic::{Endpoint, RecvStream, SendStream, TransportConfig};

mod common;
use common::{config_pair, subscribe};
use futures_util::join;
use rand::{RngCore, SeedableRng, rngs::StdRng};

struct EchoArgs {
    client_addr: SocketAddr,
    server_addr: SocketAddr,
    nr_streams: usize,
    stream_size: usize,
    receive_window: Option<u64>,
    stream_receive_window: Option<u64>,
}

async fn echo((mut send, mut recv): (SendStream, RecvStream)) {
    loop {
        // These are 32 buffers, for reading approximately 32kB at once
        let mut bufs: [Bytes; 32] = array::from_fn(|_| Bytes::new());

        match recv.read_chunks(&mut bufs).await.unwrap() {
            Some(n) => {
                send.write_all_chunks(&mut bufs[..n]).await.unwrap();
            }
            None => break,
        }
    }

    let _ = send.finish();
}

/// This is just an arbitrary number to generate deterministic test data
const SEED: u64 = 0x12345678;

fn gen_data(size: usize) -> Vec<u8> {
    let mut rng = StdRng::seed_from_u64(SEED);
    let mut buf = vec![0; size];
    rng.fill_bytes(&mut buf);
    buf
}

async fn run_echo(args: EchoArgs) {
    // Use small receive windows
    let mut transport_config = TransportConfig::default();
    if let Some(receive_window) = args.receive_window {
        transport_config.receive_window(receive_window.try_into().unwrap());
    }
    if let Some(stream_receive_window) = args.stream_receive_window {
        transport_config.stream_receive_window(stream_receive_window.try_into().unwrap());
    }
    transport_config.max_concurrent_bidi_streams(1_u8.into());
    transport_config.max_concurrent_uni_streams(1_u8.into());

    let (server_config, client_config) = config_pair(Some(transport_config));

    let server = Endpoint::server(args.server_addr, server_config)
        .await
        .unwrap();
    let client = Endpoint::client(args.client_addr).await.unwrap();

    join!(
        async {
            let conn = server.wait_incoming().await.unwrap().await.unwrap();

            while let Ok(stream) = conn.accept_bi().await {
                compio_runtime::spawn(echo(stream)).detach();
            }
        },
        async {
            let conn = client
                .connect(
                    server.local_addr().unwrap(),
                    "localhost",
                    Some(client_config),
                )
                .unwrap()
                .await
                .unwrap();

            for _ in 0..args.nr_streams {
                let (mut send, mut recv) = conn.open_bi_wait().await.unwrap();
                let msg = gen_data(args.stream_size);

                let (msg, data) = join!(
                    async {
                        let (_, msg) = send.write_all(msg).await.unwrap();
                        send.finish().unwrap();
                        msg
                    },
                    async { recv.read_to_end(usize::MAX).await.unwrap() }
                );

                assert_eq!(data, msg);
            }
        }
    );
}

#[compio_macros::test]
async fn echo_v6() {
    let _guard = subscribe();
    run_echo(EchoArgs {
        client_addr: SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0),
        server_addr: SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 0),
        nr_streams: 1,
        stream_size: 10 * 1024,
        receive_window: None,
        stream_receive_window: None,
    })
    .await;
}

#[compio_macros::test]
async fn echo_v4() {
    let _guard = subscribe();
    run_echo(EchoArgs {
        client_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
        server_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        nr_streams: 1,
        stream_size: 10 * 1024,
        receive_window: None,
        stream_receive_window: None,
    })
    .await;
}

#[compio_macros::test]
#[cfg_attr(any(bsd, solarish, windows), ignore)]
async fn echo_dualstack() {
    let _guard = subscribe();
    run_echo(EchoArgs {
        client_addr: SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0),
        server_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        nr_streams: 1,
        stream_size: 10 * 1024,
        receive_window: None,
        stream_receive_window: None,
    })
    .await;
}

#[compio_macros::test]
async fn stress_receive_window() {
    run_echo(EchoArgs {
        client_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
        server_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        nr_streams: 50,
        stream_size: 25 * 1024 + 11,
        receive_window: Some(37),
        stream_receive_window: Some(100 * 1024 * 1024),
    })
    .await;
}

#[compio_macros::test]
async fn stress_stream_receive_window() {
    // Note that there is no point in running this with too many streams,
    // since the window is only active within a stream.
    run_echo(EchoArgs {
        client_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
        server_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        nr_streams: 2,
        stream_size: 250 * 1024 + 11,
        receive_window: Some(100 * 1024 * 1024),
        stream_receive_window: Some(37),
    })
    .await;
}

#[compio_macros::test]
async fn stress_both_windows() {
    run_echo(EchoArgs {
        client_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
        server_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        nr_streams: 50,
        stream_size: 25 * 1024 + 11,
        receive_window: Some(37),
        stream_receive_window: Some(37),
    })
    .await;
}
