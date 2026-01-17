//! Simple wrapper around UDP socket with advanced features useful for QUIC,
//! ported from [`quinn-udp`]
//!
//! Differences from [`quinn-udp`]:
//! - [quinn-rs/quinn#1516] is not implemented
//! - `recvmmsg` is not available
//!
//! [`quinn-udp`]: https://docs.rs/quinn-udp
//! [quinn-rs/quinn#1516]: https://github.com/quinn-rs/quinn/pull/1516

use std::{
    future::Future,
    io,
    net::{IpAddr, SocketAddr},
    ops::{Deref, DerefMut},
    sync::atomic::Ordering,
};

use compio_buf::{BufResult, IntoInner, IoBuf, IoBufMut, SetLen, buf_try};
use compio_net::{CMsgBuilder, CMsgIter, UdpSocket};
use quinn_proto::{EcnCodepoint, Transmit};
#[cfg(windows)]
use windows_sys::Win32::Networking::WinSock;

use crate::AtomicBool;

/// Metadata for a single buffer filled with bytes received from the network
///
/// This associated buffer can contain one or more datagrams, see [`stride`].
///
/// [`stride`]: RecvMeta::stride
#[derive(Debug)]
pub(crate) struct RecvMeta {
    /// The source address of the datagram(s) contained in the buffer
    pub remote: SocketAddr,
    /// The number of bytes the associated buffer has
    pub len: usize,
    /// The size of a single datagram in the associated buffer
    ///
    /// When GRO (Generic Receive Offload) is used this indicates the size of a
    /// single datagram inside the buffer. If the buffer is larger, that is
    /// if [`len`] is greater then this value, then the individual datagrams
    /// contained have their boundaries at `stride` increments from the
    /// start. The last datagram could be smaller than `stride`.
    ///
    /// [`len`]: RecvMeta::len
    pub stride: usize,
    /// The Explicit Congestion Notification bits for the datagram(s) in the
    /// buffer
    pub ecn: Option<EcnCodepoint>,
    /// The destination IP address which was encoded in this datagram
    ///
    /// Populated on platforms: Windows, Linux, Android, FreeBSD, OpenBSD,
    /// NetBSD, macOS, and iOS.
    pub local_ip: Option<IpAddr>,
}

const CMSG_LEN: usize = 128;

struct Ancillary<const N: usize> {
    inner: [u8; N],
    len: usize,
    #[cfg(unix)]
    _align: [libc::cmsghdr; 0],
    #[cfg(windows)]
    _align: [WinSock::CMSGHDR; 0],
}

impl<const N: usize> Ancillary<N> {
    fn new() -> Self {
        Self {
            inner: [0u8; N],
            len: 0,
            _align: [],
        }
    }
}

impl<const N: usize> IoBuf for Ancillary<N> {
    fn as_init(&self) -> &[u8] {
        &self.inner[..self.len]
    }
}

impl<const N: usize> SetLen for Ancillary<N> {
    unsafe fn set_len(&mut self, len: usize) {
        debug_assert!(len <= N);
        self.len = len;
    }
}

impl<const N: usize> IoBufMut for Ancillary<N> {
    fn as_uninit(&mut self) -> &mut [std::mem::MaybeUninit<u8>] {
        self.inner.as_uninit()
    }
}

impl<const N: usize> Deref for Ancillary<N> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.inner[0..self.len]
    }
}

impl<const N: usize> DerefMut for Ancillary<N> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner[0..self.len]
    }
}

#[cfg(linux_all)]
#[inline]
fn max_gso_segments(socket: &UdpSocket) -> io::Result<usize> {
    unsafe {
        socket.get_socket_option::<libc::c_int>(libc::SOL_UDP, libc::UDP_SEGMENT)?;
    }
    Ok(32)
}
#[cfg(windows)]
#[inline]
fn max_gso_segments(socket: &UdpSocket) -> io::Result<usize> {
    unsafe {
        socket.get_socket_option::<i32>(WinSock::IPPROTO_UDP, WinSock::UDP_SEND_MSG_SIZE)?;
    }
    Ok(512)
}
#[cfg(not(any(linux_all, windows)))]
#[inline]
fn max_gso_segments(_socket: &UdpSocket) -> io::Result<usize> {
    Err(io::Error::from(io::ErrorKind::Unsupported))
}

#[inline]
fn error_is_unsupported(e: &io::Error) -> bool {
    if matches!(
        e.kind(),
        io::ErrorKind::Unsupported | io::ErrorKind::InvalidInput
    ) {
        return true;
    }
    let Some(raw) = e.raw_os_error() else {
        return false;
    };
    #[cfg(unix)]
    {
        raw == libc::ENOPROTOOPT
    }
    #[cfg(windows)]
    {
        raw == WinSock::WSAENOPROTOOPT
    }
}

macro_rules! set_socket_option {
    ($socket:expr, $level:expr, $name:expr, $value:expr $(,)?) => {
        match unsafe { $socket.set_socket_option($level, $name, $value) } {
            Ok(()) => true,
            Err(e) if error_is_unsupported(&e) => false,
            Err(e) => {
                compio_log::warn!(
                    level = stringify!($level),
                    name = stringify!($name),
                    "failed to set socket option: {}",
                    e
                );
                return Err(e);
            }
        }
    };
}

#[derive(Debug)]
pub(crate) struct Socket {
    inner: UdpSocket,
    max_gro_segments: usize,
    max_gso_segments: usize,
    may_fragment: bool,
    has_gso_error: AtomicBool,
    #[cfg(freebsd)]
    encode_src_ip_v4: bool,
}

impl Socket {
    pub fn new(socket: UdpSocket) -> io::Result<Self> {
        let is_ipv6 = socket.local_addr()?.is_ipv6();
        #[cfg(unix)]
        let only_v6 = unsafe {
            is_ipv6
                && socket.get_socket_option::<libc::c_int>(libc::IPPROTO_IPV6, libc::IPV6_V6ONLY)?
                    != 0
        };
        #[cfg(windows)]
        let only_v6 = unsafe {
            is_ipv6
                && socket.get_socket_option::<u8>(WinSock::IPPROTO_IPV6, WinSock::IPV6_V6ONLY)? != 0
        };
        let is_ipv4 = socket.local_addr()?.is_ipv4() || !only_v6;

        // ECN
        if is_ipv4 {
            #[cfg(all(unix, not(any(non_freebsd, solarish))))]
            set_socket_option!(socket, libc::IPPROTO_IP, libc::IP_RECVTOS, &1);
            #[cfg(windows)]
            set_socket_option!(socket, WinSock::IPPROTO_IP, WinSock::IP_RECVECN, &1);
        }
        if is_ipv6 {
            #[cfg(unix)]
            set_socket_option!(socket, libc::IPPROTO_IPV6, libc::IPV6_RECVTCLASS, &1);
            #[cfg(windows)]
            set_socket_option!(socket, WinSock::IPPROTO_IPV6, WinSock::IPV6_RECVECN, &1);
        }

        // pktinfo / destination address
        if is_ipv4 {
            #[cfg(linux_all)]
            set_socket_option!(socket, libc::IPPROTO_IP, libc::IP_PKTINFO, &1);
            #[cfg(any(bsd, solarish, apple))]
            set_socket_option!(socket, libc::IPPROTO_IP, libc::IP_RECVDSTADDR, &1);
            #[cfg(windows)]
            set_socket_option!(socket, WinSock::IPPROTO_IP, WinSock::IP_PKTINFO, &1);
        }
        if is_ipv6 {
            #[cfg(unix)]
            set_socket_option!(socket, libc::IPPROTO_IPV6, libc::IPV6_RECVPKTINFO, &1);
            #[cfg(windows)]
            set_socket_option!(socket, WinSock::IPPROTO_IPV6, WinSock::IPV6_PKTINFO, &1);
        }

        // disable fragmentation
        #[allow(unused_mut)]
        let mut may_fragment = false;
        if is_ipv4 {
            #[cfg(linux_all)]
            {
                may_fragment |= set_socket_option!(
                    socket,
                    libc::IPPROTO_IP,
                    libc::IP_MTU_DISCOVER,
                    &libc::IP_PMTUDISC_PROBE,
                );
            }
            #[cfg(any(aix, freebsd, apple))]
            {
                may_fragment |= set_socket_option!(socket, libc::IPPROTO_IP, libc::IP_DONTFRAG, &1);
            }
            #[cfg(windows)]
            {
                may_fragment |=
                    set_socket_option!(socket, WinSock::IPPROTO_IP, WinSock::IP_DONTFRAGMENT, &1);
            }
        }
        if is_ipv6 {
            #[cfg(linux_all)]
            {
                may_fragment |= set_socket_option!(
                    socket,
                    libc::IPPROTO_IPV6,
                    libc::IPV6_MTU_DISCOVER,
                    &libc::IPV6_PMTUDISC_PROBE,
                );
            }
            #[cfg(unix)]
            {
                may_fragment |=
                    set_socket_option!(socket, libc::IPPROTO_IPV6, libc::IPV6_DONTFRAG, &1);
            }
            #[cfg(windows)]
            {
                may_fragment |=
                    set_socket_option!(socket, WinSock::IPPROTO_IPV6, WinSock::IPV6_DONTFRAG, &1);
            }
        }

        // GRO
        #[allow(unused_mut)]
        let mut max_gro_segments = 1;
        #[cfg(linux_all)]
        if set_socket_option!(socket, libc::SOL_UDP, libc::UDP_GRO, &1) {
            max_gro_segments = 64;
        }
        #[cfg(all(windows, feature = "windows-gro"))]
        if set_socket_option!(
            socket,
            WinSock::IPPROTO_UDP,
            WinSock::UDP_RECV_MAX_COALESCED_SIZE,
            &(u16::MAX as u32),
        ) {
            max_gro_segments = 64;
        }

        // GSO
        let max_gso_segments = max_gso_segments(&socket).unwrap_or(1);

        #[cfg(freebsd)]
        let encode_src_ip_v4 =
            socket.local_addr().unwrap().ip() == IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED);

        Ok(Self {
            inner: socket,
            max_gro_segments,
            max_gso_segments,
            may_fragment,
            has_gso_error: AtomicBool::new(false),
            #[cfg(freebsd)]
            encode_src_ip_v4,
        })
    }

    #[inline]
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }

    #[inline]
    pub fn may_fragment(&self) -> bool {
        self.may_fragment
    }

    #[inline]
    pub fn max_gro_segments(&self) -> usize {
        self.max_gro_segments
    }

    #[inline]
    pub fn max_gso_segments(&self) -> usize {
        if self.has_gso_error.load(Ordering::Relaxed) {
            1
        } else {
            self.max_gso_segments
        }
    }

    pub async fn recv<T: IoBufMut>(&self, buffer: T) -> BufResult<RecvMeta, T> {
        let control = Ancillary::<CMSG_LEN>::new();

        let BufResult(res, (buffer, control)) = self.inner.recv_msg(buffer, control).await;
        let ((len, _, remote), buffer) = buf_try!(res, buffer);

        let mut ecn_bits = 0u8;
        let mut local_ip = None;
        #[allow(unused_mut)]
        let mut stride = len;

        // SAFETY: `control` contains valid data
        unsafe {
            for cmsg in CMsgIter::new(&control) {
                #[cfg(windows)]
                const UDP_COALESCED_INFO: i32 = WinSock::UDP_COALESCED_INFO as i32;

                match (cmsg.level(), cmsg.ty()) {
                    // ECN
                    #[cfg(unix)]
                    (libc::IPPROTO_IP, libc::IP_TOS) => ecn_bits = *cmsg.data::<u8>(),
                    #[cfg(all(unix, not(any(non_freebsd, solarish))))]
                    (libc::IPPROTO_IP, libc::IP_RECVTOS) => ecn_bits = *cmsg.data::<u8>(),
                    #[cfg(unix)]
                    (libc::IPPROTO_IPV6, libc::IPV6_TCLASS) => {
                        // NOTE: It's OK to use `c_int` instead of `u8` on Apple systems
                        ecn_bits = *cmsg.data::<libc::c_int>() as u8
                    }
                    #[cfg(windows)]
                    (WinSock::IPPROTO_IP, WinSock::IP_ECN)
                    | (WinSock::IPPROTO_IPV6, WinSock::IPV6_ECN) => {
                        ecn_bits = *cmsg.data::<i32>() as u8
                    }

                    // pktinfo / destination address
                    #[cfg(linux_all)]
                    (libc::IPPROTO_IP, libc::IP_PKTINFO) => {
                        let pktinfo = cmsg.data::<libc::in_pktinfo>();
                        local_ip = Some(IpAddr::from(pktinfo.ipi_addr.s_addr.to_ne_bytes()));
                    }
                    #[cfg(any(bsd, solarish, apple))]
                    (libc::IPPROTO_IP, libc::IP_RECVDSTADDR) => {
                        let in_addr = cmsg.data::<libc::in_addr>();
                        local_ip = Some(IpAddr::from(in_addr.s_addr.to_ne_bytes()));
                    }
                    #[cfg(windows)]
                    (WinSock::IPPROTO_IP, WinSock::IP_PKTINFO) => {
                        let pktinfo = cmsg.data::<WinSock::IN_PKTINFO>();
                        local_ip = Some(IpAddr::from(pktinfo.ipi_addr.S_un.S_addr.to_ne_bytes()));
                    }
                    #[cfg(unix)]
                    (libc::IPPROTO_IPV6, libc::IPV6_PKTINFO) => {
                        let pktinfo = cmsg.data::<libc::in6_pktinfo>();
                        local_ip = Some(IpAddr::from(pktinfo.ipi6_addr.s6_addr));
                    }
                    #[cfg(windows)]
                    (WinSock::IPPROTO_IPV6, WinSock::IPV6_PKTINFO) => {
                        let pktinfo = cmsg.data::<WinSock::IN6_PKTINFO>();
                        local_ip = Some(IpAddr::from(pktinfo.ipi6_addr.u.Byte));
                    }

                    // GRO
                    #[cfg(linux_all)]
                    (libc::SOL_UDP, libc::UDP_GRO) => stride = *cmsg.data::<libc::c_int>() as usize,
                    #[cfg(windows)]
                    (WinSock::IPPROTO_UDP, UDP_COALESCED_INFO) => {
                        stride = *cmsg.data::<u32>() as usize
                    }

                    _ => {}
                }
            }
        }

        let meta = RecvMeta {
            remote,
            len,
            stride,
            ecn: EcnCodepoint::from_bits(ecn_bits),
            local_ip,
        };
        BufResult(Ok(meta), buffer)
    }

    pub async fn send<T: IoBuf>(&self, buffer: T, transmit: &Transmit) -> T {
        let is_ipv4 = transmit.destination.ip().to_canonical().is_ipv4();
        let ecn = transmit.ecn.map_or(0, |x| x as u8);

        let mut control = Ancillary::<CMSG_LEN>::new();
        let mut builder = CMsgBuilder::new(control.as_uninit());

        // ECN
        if is_ipv4 {
            #[cfg(all(unix, not(any(freebsd, netbsd))))]
            builder.try_push(libc::IPPROTO_IP, libc::IP_TOS, ecn as libc::c_int);
            #[cfg(freebsd)]
            builder.try_push(libc::IPPROTO_IP, libc::IP_TOS, ecn as libc::c_uchar);
            #[cfg(windows)]
            builder.try_push(WinSock::IPPROTO_IP, WinSock::IP_ECN, ecn as i32);
        } else {
            #[cfg(unix)]
            builder.try_push(libc::IPPROTO_IPV6, libc::IPV6_TCLASS, ecn as libc::c_int);
            #[cfg(windows)]
            builder.try_push(WinSock::IPPROTO_IPV6, WinSock::IPV6_ECN, ecn as i32);
        }

        // pktinfo / destination address
        match transmit.src_ip {
            Some(IpAddr::V4(ip)) => {
                let addr = u32::from_ne_bytes(ip.octets());
                #[cfg(linux_all)]
                {
                    let pktinfo = libc::in_pktinfo {
                        ipi_ifindex: 0,
                        ipi_spec_dst: libc::in_addr { s_addr: addr },
                        ipi_addr: libc::in_addr { s_addr: 0 },
                    };
                    builder.try_push(libc::IPPROTO_IP, libc::IP_PKTINFO, pktinfo);
                }
                #[cfg(any(bsd, solarish, apple))]
                {
                    #[cfg(freebsd)]
                    let encode_src_ip_v4 = self.encode_src_ip_v4;
                    #[cfg(any(non_freebsd, solarish, apple))]
                    let encode_src_ip_v4 = true;

                    if encode_src_ip_v4 {
                        let addr = libc::in_addr { s_addr: addr };
                        builder.try_push(libc::IPPROTO_IP, libc::IP_RECVDSTADDR, addr);
                    }
                }
                #[cfg(windows)]
                {
                    let pktinfo = WinSock::IN_PKTINFO {
                        ipi_addr: WinSock::IN_ADDR {
                            S_un: WinSock::IN_ADDR_0 { S_addr: addr },
                        },
                        ipi_ifindex: 0,
                    };
                    builder.try_push(WinSock::IPPROTO_IP, WinSock::IP_PKTINFO, pktinfo);
                }
            }
            Some(IpAddr::V6(ip)) => {
                #[cfg(unix)]
                {
                    let pktinfo = libc::in6_pktinfo {
                        ipi6_ifindex: 0,
                        ipi6_addr: libc::in6_addr {
                            s6_addr: ip.octets(),
                        },
                    };
                    builder.try_push(libc::IPPROTO_IPV6, libc::IPV6_PKTINFO, pktinfo);
                }
                #[cfg(windows)]
                {
                    let pktinfo = WinSock::IN6_PKTINFO {
                        ipi6_addr: WinSock::IN6_ADDR {
                            u: WinSock::IN6_ADDR_0 { Byte: ip.octets() },
                        },
                        ipi6_ifindex: 0,
                    };
                    builder.try_push(WinSock::IPPROTO_IPV6, WinSock::IPV6_PKTINFO, pktinfo);
                }
            }
            None => {}
        }

        // GSO
        if let Some(segment_size) = transmit.segment_size
            && segment_size < transmit.size
        {
            #[cfg(linux_all)]
            builder.try_push(libc::SOL_UDP, libc::UDP_SEGMENT, segment_size as u16);
            #[cfg(windows)]
            builder.try_push(
                WinSock::IPPROTO_UDP,
                WinSock::UDP_SEND_MSG_SIZE,
                segment_size as u32,
            );
            #[cfg(not(any(linux_all, windows)))]
            let _ = segment_size;
        }

        let len = builder.finish();
        control.len = len;

        let mut buffer = buffer.slice(0..transmit.size);

        loop {
            let res;
            BufResult(res, (buffer, control)) = self
                .inner
                .send_msg(buffer, control, transmit.destination)
                .await;

            match res {
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => match e.raw_os_error() {
                    #[cfg(unix)]
                    Some(libc::EMSGSIZE) => {}
                    _ => {
                        #[cfg(linux_all)]
                        if matches!(e.raw_os_error(), Some(libc::EIO) | Some(libc::EINVAL))
                            && self.max_gso_segments() > 1
                        {
                            self.has_gso_error.store(true, Ordering::Relaxed);
                        }
                        compio_log::info!("failed to send UDP datagram: {e:?}, {transmit:?}");
                    }
                },
            }
            break;
        }

        buffer.into_inner()
    }

    pub fn close(self) -> impl Future<Output = io::Result<()>> {
        self.inner.close()
    }
}

impl Clone for Socket {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            may_fragment: self.may_fragment,
            max_gro_segments: self.max_gro_segments,
            max_gso_segments: self.max_gso_segments,
            has_gso_error: AtomicBool::new(self.has_gso_error.load(Ordering::Relaxed)),
            #[cfg(freebsd)]
            encode_src_ip_v4: self.encode_src_ip_v4,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr};

    use compio_driver::AsRawFd;
    use socket2::{Domain, Protocol, Socket as Socket2, Type};

    use super::*;

    async fn test_send_recv<T: IoBuf>(
        passive: Socket,
        active: Socket,
        content: T,
        transmit: Transmit,
    ) {
        let passive_addr = passive.local_addr().unwrap();
        let active_addr = active.local_addr().unwrap();

        let content = active.send(content, &transmit).await;

        let segment_size = transmit.segment_size.unwrap_or(transmit.size);
        let expected_datagrams = transmit.size / segment_size;
        let mut datagrams = 0;
        while datagrams < expected_datagrams {
            let (meta, buf) = passive
                .recv(Vec::with_capacity(u16::MAX as usize))
                .await
                .unwrap();
            let segments = meta.len / meta.stride;
            for i in 0..segments {
                assert_eq!(
                    &content.as_init()
                        [(datagrams + i) * segment_size..(datagrams + i + 1) * segment_size],
                    &buf[(i * meta.stride)..((i + 1) * meta.stride)]
                );
            }
            datagrams += segments;

            assert_eq!(meta.ecn, transmit.ecn);

            assert_eq!(meta.remote.port(), active_addr.port());
            for addr in [meta.remote.ip(), meta.local_ip.unwrap()] {
                match (active_addr.is_ipv6(), passive_addr.is_ipv6()) {
                    (_, false) => assert_eq!(addr, Ipv4Addr::LOCALHOST),
                    (false, true) => assert!(
                        addr == Ipv4Addr::LOCALHOST || addr == Ipv4Addr::LOCALHOST.to_ipv6_mapped()
                    ),
                    (true, true) => assert!(
                        addr == Ipv6Addr::LOCALHOST || addr == Ipv4Addr::LOCALHOST.to_ipv6_mapped()
                    ),
                }
            }
        }
        assert_eq!(datagrams, expected_datagrams);
    }

    /// Helper function to create dualstack udp socket.
    /// This is only used for testing.
    fn bind_udp_dualstack() -> io::Result<UdpSocket> {
        #[cfg(unix)]
        use std::os::fd::{FromRawFd, IntoRawFd};
        #[cfg(windows)]
        use std::os::windows::io::{FromRawSocket, IntoRawSocket};

        let socket = Socket2::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?;
        socket.set_only_v6(false)?;
        socket.bind(&SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0).into())?;

        compio_runtime::Runtime::with_current(|r| r.attach(socket.as_raw_fd()))?;
        #[cfg(unix)]
        unsafe {
            Ok(UdpSocket::from_raw_fd(socket.into_raw_fd()))
        }
        #[cfg(windows)]
        unsafe {
            Ok(UdpSocket::from_raw_socket(socket.into_raw_socket()))
        }
    }

    #[compio_macros::test]
    async fn basic() {
        let passive = Socket::new(UdpSocket::bind("[::1]:0").await.unwrap()).unwrap();
        let active = Socket::new(UdpSocket::bind("[::1]:0").await.unwrap()).unwrap();
        let content = b"hello";
        let transmit = Transmit {
            destination: passive.local_addr().unwrap(),
            ecn: None,
            size: content.len(),
            segment_size: None,
            src_ip: None,
        };
        test_send_recv(passive, active, content, transmit).await;
    }

    #[compio_macros::test]
    #[cfg_attr(any(non_freebsd, solarish), ignore)]
    async fn ecn_v4() {
        let passive = Socket::new(UdpSocket::bind("127.0.0.1:0").await.unwrap()).unwrap();
        let active = Socket::new(UdpSocket::bind("127.0.0.1:0").await.unwrap()).unwrap();
        for ecn in [EcnCodepoint::Ect0, EcnCodepoint::Ect1] {
            let content = b"hello";
            let transmit = Transmit {
                destination: passive.local_addr().unwrap(),
                ecn: Some(ecn),
                size: content.len(),
                segment_size: None,
                src_ip: None,
            };
            test_send_recv(passive.clone(), active.clone(), content, transmit).await;
        }
    }

    #[compio_macros::test]
    async fn ecn_v6() {
        let passive = Socket::new(UdpSocket::bind("[::1]:0").await.unwrap()).unwrap();
        let active = Socket::new(UdpSocket::bind("[::1]:0").await.unwrap()).unwrap();
        for ecn in [EcnCodepoint::Ect0, EcnCodepoint::Ect1] {
            let content = b"hello";
            let transmit = Transmit {
                destination: passive.local_addr().unwrap(),
                ecn: Some(ecn),
                size: content.len(),
                segment_size: None,
                src_ip: None,
            };
            test_send_recv(passive.clone(), active.clone(), content, transmit).await;
        }
    }

    #[compio_macros::test]
    #[cfg_attr(non_freebsd, ignore)]
    async fn ecn_dualstack() {
        let passive = Socket::new(bind_udp_dualstack().unwrap()).unwrap();

        let mut dst_v4 = passive.local_addr().unwrap();
        dst_v4.set_ip(IpAddr::V4(Ipv4Addr::LOCALHOST));
        let mut dst_v6 = dst_v4;
        dst_v6.set_ip(IpAddr::V6(Ipv6Addr::LOCALHOST));

        for (src, dst) in [("[::1]:0", dst_v6), ("127.0.0.1:0", dst_v4)] {
            let active = Socket::new(UdpSocket::bind(src).await.unwrap()).unwrap();

            for ecn in [EcnCodepoint::Ect0, EcnCodepoint::Ect1] {
                let content = b"hello";
                let transmit = Transmit {
                    destination: dst,
                    ecn: Some(ecn),
                    size: content.len(),
                    segment_size: None,
                    src_ip: None,
                };
                test_send_recv(passive.clone(), active.clone(), content, transmit).await;
            }
        }
    }

    #[compio_macros::test]
    #[cfg_attr(any(non_freebsd, solarish), ignore)]
    async fn ecn_v4_mapped_v6() {
        let passive = Socket::new(UdpSocket::bind("127.0.0.1:0").await.unwrap()).unwrap();
        let active = Socket::new(bind_udp_dualstack().unwrap()).unwrap();

        let mut dst_addr = passive.local_addr().unwrap();
        dst_addr.set_ip(IpAddr::V6(Ipv4Addr::LOCALHOST.to_ipv6_mapped()));

        for ecn in [EcnCodepoint::Ect0, EcnCodepoint::Ect1] {
            let content = b"hello";
            let transmit = Transmit {
                destination: dst_addr,
                ecn: Some(ecn),
                size: content.len(),
                segment_size: None,
                src_ip: None,
            };
            test_send_recv(passive.clone(), active.clone(), content, transmit).await;
        }
    }

    #[compio_macros::test]
    #[cfg_attr(not(any(linux, windows)), ignore)]
    async fn gso() {
        let passive = Socket::new(UdpSocket::bind("[::1]:0").await.unwrap()).unwrap();
        let active = Socket::new(UdpSocket::bind("[::1]:0").await.unwrap()).unwrap();

        let max_segments = active.max_gso_segments();
        const SEGMENT_SIZE: usize = 128;
        let content = vec![0xAB; SEGMENT_SIZE * max_segments];

        let transmit = Transmit {
            destination: passive.local_addr().unwrap(),
            ecn: None,
            size: content.len(),
            segment_size: Some(SEGMENT_SIZE),
            src_ip: None,
        };
        test_send_recv(passive, active, content, transmit).await;
    }
}
