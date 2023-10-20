cfg_if::cfg_if! {
    if #[cfg(windows)] {
        #[path = "windows.rs"]
        mod sys;
    } else if #[cfg(all(target_os = "linux", target_env = "gnu"))] {
        #[path = "glibc.rs"]
        mod sys;
    } else if #[cfg(unix)] {
        #[path = "unix.rs"]
        mod sys;
    }
}

use std::{io, net::SocketAddr};

pub async fn resolve_sock_addrs(
    host: &str,
    port: u16,
) -> io::Result<std::vec::IntoIter<SocketAddr>> {
    use std::task::Poll;

    use compio_runtime::event::Event;

    let mut resolver = sys::AsyncResolver::new(host, port)?;
    let mut hints: sys::addrinfo = unsafe { std::mem::zeroed() };
    hints.ai_family = sys::AF_UNSPEC as _;
    hints.ai_socktype = sys::SOCK_STREAM;
    hints.ai_protocol = sys::IPPROTO_TCP;

    let event = Event::new()?;
    let handle = event.handle()?;
    match unsafe { resolver.call(&hints, handle) } {
        Poll::Ready(res) => {
            res?;
        }
        Poll::Pending => {
            event.wait().await?;
        }
    }

    unsafe { resolver.addrs() }
}

#[allow(unused)]
fn to_addrs(mut result: *mut sys::addrinfo, port: u16) -> std::vec::IntoIter<SocketAddr> {
    use socket2::SockAddr;

    let mut addrs = vec![];
    while let Some(info) = unsafe { result.as_ref() } {
        let addr = unsafe {
            SockAddr::try_init(|buffer, len| {
                std::slice::from_raw_parts_mut::<u8>(buffer.cast(), info.ai_addrlen as _)
                    .copy_from_slice(std::slice::from_raw_parts::<u8>(
                        info.ai_addr.cast(),
                        info.ai_addrlen as _,
                    ));
                *len = info.ai_addrlen as _;
                Ok(())
            })
        }
        // it is always Ok
        .unwrap()
        .1;
        if let Some(mut addr) = addr.as_socket() {
            addr.set_port(port);
            addrs.push(addr)
        }
        result = info.ai_next;
    }
    addrs.into_iter()
}
