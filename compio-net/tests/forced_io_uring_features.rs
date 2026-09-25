use std::net::Ipv6Addr;

use compio_driver::{IoUringFeatures, force_io_uring_features};
use compio_io::ancillary::ReturnFlags;
use compio_net::UdpSocket;
use futures_util::StreamExt;

#[compio_macros::test(with_proactor(buffer_pool_buffer_len = 256))]
async fn test_forced_multishot_recvmsg() {
    // Only run when explicitly requested on enterprise kernels with backported
    // features (e.g. RHEL 9 / CentOS Stream 9 running on Linux 5.14).
    if std::env::var_os("COMPIO_TEST_BACKPORTED_IO_URING").is_none() {
        return;
    }

    force_io_uring_features(IoUringFeatures::MULTISHOT_RECVMSG);

    // 1. Test recv_from_multi with forced capability
    {
        let listener = UdpSocket::bind((Ipv6Addr::LOCALHOST, 0)).await.unwrap();
        let server_addr = listener.local_addr().unwrap();
        let connected = UdpSocket::bind((Ipv6Addr::LOCALHOST, 0)).await.unwrap();
        let addr = connected.local_addr().unwrap();

        compio_runtime::spawn(async move {
            listener.send_to(b"test", addr).await.unwrap();
        })
        .detach();

        let result = connected.recv_from_multi().next().await.unwrap().unwrap();
        assert_eq!(result.data(), b"test");
        assert_eq!(result.addr().and_then(|a| a.as_socket()), Some(server_addr));
    }

    // 2. Test recv_msg_multi with forced capability
    {
        let listener = UdpSocket::bind((Ipv6Addr::LOCALHOST, 0)).await.unwrap();
        let server_addr = listener.local_addr().unwrap();
        let connected = UdpSocket::bind((Ipv6Addr::LOCALHOST, 0)).await.unwrap();
        let addr = connected.local_addr().unwrap();

        compio_runtime::spawn(async move {
            listener.send_to(b"test", addr).await.unwrap();
        })
        .detach();

        let result = connected.recv_msg_multi(64).next().await.unwrap().unwrap();
        assert_eq!(result.data(), b"test");
        assert_eq!(result.addr().and_then(|a| a.as_socket()), Some(server_addr));
        assert_eq!(result.flags(), ReturnFlags::empty());
    }

    // 3. Test recv_msg_multi truncated datagram with forced capability
    {
        let listener = UdpSocket::bind((Ipv6Addr::LOCALHOST, 0)).await.unwrap();
        let server_addr = listener.local_addr().unwrap();
        let connected = UdpSocket::bind((Ipv6Addr::LOCALHOST, 0)).await.unwrap();
        let addr = connected.local_addr().unwrap();

        compio_runtime::spawn(async move {
            listener.send_to(vec![0; 1024], addr).await.unwrap();
        })
        .detach();

        let result = connected.recv_msg_multi(64).next().await.unwrap().unwrap();
        assert_eq!(result.addr().and_then(|a| a.as_socket()), Some(server_addr));
        assert!(result.flags().contains(ReturnFlags::TRUNC));
    }
}
