use compio_net::{CMsgBuilder, CMsgIter, UdpSocket};

#[compio_macros::test]
async fn connect() {
    const MSG: &str = "foo bar baz";

    let passive = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let passive_addr = passive.local_addr().unwrap();

    let active = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let active_addr = active.local_addr().unwrap();

    active.connect(passive_addr).await.unwrap();
    active.send(MSG).await.0.unwrap();

    let (_, buffer) = passive.recv(Vec::with_capacity(20)).await.unwrap();
    assert_eq!(MSG.as_bytes(), &buffer);
    assert_eq!(active.local_addr().unwrap(), active_addr);
    assert_eq!(active.peer_addr().unwrap(), passive_addr);
}

#[compio_macros::test]
async fn send_to() {
    const MSG: &str = "foo bar baz";

    macro_rules! must_success {
        ($r:expr, $expect_addr:expr) => {
            let res = $r;
            assert_eq!(res.0.unwrap().1, $expect_addr);
            assert_eq!(res.1, MSG.as_bytes());
        };
    }

    let passive1 = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let passive1_addr = passive1.local_addr().unwrap();

    let passive01 = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let passive01_addr = passive01.local_addr().unwrap();

    let passive2 = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let passive2_addr = passive2.local_addr().unwrap();

    let passive3 = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let passive3_addr = passive3.local_addr().unwrap();

    let active = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let active_addr = active.local_addr().unwrap();

    active.send_to(MSG, &passive01_addr).await.0.unwrap();
    active.send_to(MSG, &passive1_addr).await.0.unwrap();
    active.send_to(MSG, &passive2_addr).await.0.unwrap();
    active.send_to(MSG, &passive3_addr).await.0.unwrap();

    must_success!(
        passive1.recv_from(Vec::with_capacity(20)).await,
        active_addr
    );
    must_success!(
        passive2.recv_from(Vec::with_capacity(20)).await,
        active_addr
    );
    must_success!(
        passive3.recv_from(Vec::with_capacity(20)).await,
        active_addr
    );
}

#[compio_macros::test]
async fn send_msg_with_ipv6_ecn() {
    #[cfg(unix)]
    use libc::{IPPROTO_IPV6, IPV6_RECVTCLASS, IPV6_TCLASS};
    #[cfg(windows)]
    use windows_sys::Win32::Networking::WinSock::{
        IPPROTO_IPV6, IPV6_ECN, IPV6_RECVTCLASS, IPV6_TCLASS,
    };

    const MSG: &str = "foo bar baz";

    let passive = UdpSocket::bind("[::1]:0").await.unwrap();
    let passive_addr = passive.local_addr().unwrap();

    passive
        .set_socket_option(IPPROTO_IPV6, IPV6_RECVTCLASS, &1)
        .unwrap();

    let active = UdpSocket::bind("[::1]:0").await.unwrap();
    let active_addr = active.local_addr().unwrap();

    let mut control = vec![0u8; 32];
    let mut builder = CMsgBuilder::new(&mut control);

    const ECN_BITS: i32 = 0b11;

    #[cfg(unix)]
    builder
        .try_push(IPPROTO_IPV6, IPV6_TCLASS, ECN_BITS)
        .unwrap();
    #[cfg(windows)]
    builder.try_push(IPPROTO_IPV6, IPV6_ECN, ECN_BITS).unwrap();

    let len = builder.finish();
    control.truncate(len);

    active.send_msg(MSG, control, passive_addr).await.unwrap();

    let res = passive.recv_msg(Vec::with_capacity(20), [0u8; 32]).await;
    assert_eq!(res.0.unwrap().1, active_addr);
    assert_eq!(res.1.0, MSG.as_bytes());
    unsafe {
        let mut iter = CMsgIter::new(&res.1.1);
        let cmsg = iter.next().unwrap();
        assert_eq!(cmsg.level(), IPPROTO_IPV6);
        assert_eq!(cmsg.ty(), IPV6_TCLASS);
        assert_eq!(cmsg.data::<i32>(), &ECN_BITS);
    }
}
