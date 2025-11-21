use std::time::Duration;

use crate::Socket;

/// Options for configuring TCP sockets.
/// By default, SO_REUSEADDR is enabled.
#[derive(Default, Debug, Copy, Clone)]
pub struct TcpOpts {
    recv_buffer_size: Option<usize>,
    send_buffer_size: Option<usize>,
    keepalive: Option<bool>,
    linger: Option<Duration>,
    read_timeout: Option<Duration>,
    write_timeout: Option<Duration>,
    reuse_address: Option<bool>,
    reuse_port: Option<bool>,
    nodelay: Option<bool>,
}

impl TcpOpts {
    /// Creates a new `TcpOpts` with default settings.
    pub fn new() -> Self {
        TcpOpts::default()
    }

    /// Sets the receive buffer size for the TCP socket.
    pub fn recv_buffer_size(mut self, size: usize) -> Self {
        self.recv_buffer_size = Some(size);
        self
    }

    /// Sets the send buffer size for the TCP socket.
    pub fn send_buffer_size(mut self, size: usize) -> Self {
        self.send_buffer_size = Some(size);
        self
    }

    /// Enables or disables the TCP keepalive option.
    pub fn keepalive(mut self, keepalive: bool) -> Self {
        self.keepalive = Some(keepalive);
        self
    }

    /// Sets the linger duration for the TCP socket.
    pub fn linger(mut self, duration: Duration) -> Self {
        self.linger = Some(duration);
        self
    }

    /// Sets the read timeout for the TCP socket.
    pub fn read_timeout(mut self, duration: Duration) -> Self {
        self.read_timeout = Some(duration);
        self
    }

    /// Sets the write timeout for the TCP socket.
    pub fn write_timeout(mut self, duration: Duration) -> Self {
        self.write_timeout = Some(duration);
        self
    }

    /// Sets whether the TCP socket should reuse the address.
    pub fn reuse_address(mut self, reuse: bool) -> Self {
        self.reuse_address = Some(reuse);
        self
    }

    /// Sets whether the TCP socket should reuse the port.
    pub fn reuse_port(mut self, reuse: bool) -> Self {
        self.reuse_port = Some(reuse);
        self
    }

    /// Sets whether the TCP socket should disable Nagle's algorithm (no delay).
    pub fn nodelay(mut self, nodelay: bool) -> Self {
        self.nodelay = Some(nodelay);
        self
    }

    pub(crate) fn setup_socket(&self, socket: &Socket) -> std::io::Result<()> {
        if let Some(size) = self.recv_buffer_size {
            socket.socket.set_recv_buffer_size(size)?;
        }
        if let Some(size) = self.send_buffer_size {
            socket.socket.set_send_buffer_size(size)?;
        }
        if let Some(keepalive) = self.keepalive {
            socket.socket.set_keepalive(keepalive)?;
        }
        if let Some(linger) = self.linger {
            socket.socket.set_linger(Some(linger))?;
        }
        if let Some(read_timeout) = self.read_timeout {
            socket.socket.set_read_timeout(Some(read_timeout))?;
        }
        if let Some(write_timeout) = self.write_timeout {
            socket.socket.set_write_timeout(Some(write_timeout))?;
        }
        if let Some(reuse_address) = self.reuse_address {
            socket.socket.set_reuse_address(reuse_address)?;
        }
        #[cfg(all(
            unix,
            not(any(target_os = "illumos", target_os = "solaris", target_os = "cygwin"))
        ))]
        if let Some(reuse_port) = self.reuse_port {
            socket.socket.set_reuse_port(reuse_port)?;
        }
        if let Some(nodelay) = self.nodelay {
            socket.socket.set_tcp_nodelay(nodelay)?;
        }
        Ok(())
    }
}
