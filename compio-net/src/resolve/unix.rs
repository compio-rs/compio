use std::{
    io,
    net::{SocketAddr, ToSocketAddrs},
    panic::resume_unwind,
    task::Poll,
    thread::JoinHandle,
};

use compio_runtime::event::EventHandle;
pub use libc::{addrinfo, sockaddr_storage, AF_UNSPEC, IPPROTO_TCP, SOCK_STREAM};

pub struct AsyncResolver<'a> {
    name: &'a str,
    port: u16,
    thread: Option<JoinHandle<io::Result<std::vec::IntoIter<SocketAddr>>>>,
}

impl<'a> AsyncResolver<'a> {
    pub fn new(name: &'a str, port: u16) -> io::Result<Self> {
        Ok(Self {
            name,
            port,
            thread: None,
        })
    }

    pub unsafe fn call(
        &mut self,
        _hints: &libc::addrinfo,
        handle: EventHandle,
    ) -> Poll<io::Result<()>> {
        let name = self.name.to_string();
        let port = self.port;
        self.thread = Some(std::thread::spawn(move || {
            let iter = (name, port).to_socket_addrs()?;
            handle.notify()?;
            Ok(iter)
        }));
        Poll::Pending
    }

    pub unsafe fn addrs(&mut self) -> io::Result<std::vec::IntoIter<SocketAddr>> {
        self.thread
            .take()
            .expect("the thread should have been spawned")
            .join()
            .unwrap_or_else(|e| resume_unwind(e))
    }
}
