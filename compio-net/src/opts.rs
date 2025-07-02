use std::time::Duration;

use crate::Socket;

/// Options for configuring TCP sockets.
/// By default, SO_REUSEADDR is enabled.
#[derive(Default, Debug, Copy, Clone)]
pub struct TcpOpts {
    recv_buffer_size: Option<usize>,
    send_buffer_size: Option<usize>,
    keepalive: bool,
    linger: Option<Duration>,
    read_timeout: Option<Duration>,
    write_timeout: Option<Duration>,
    reuse_address: bool,
    reuse_port: bool,
    no_delay: bool,
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
        self.keepalive = keepalive;
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
        self.reuse_address = reuse;
        self
    }

    /// Sets whether the TCP socket should reuse the port.
    pub fn reuse_port(mut self, reuse: bool) -> Self {
        self.reuse_port = reuse;
        self
    }

    /// Sets whether the TCP socket should disable Nagle's algorithm (no delay).
    pub fn no_delay(mut self, no_delay: bool) -> Self {
        self.no_delay = no_delay;
        self
    }

    pub(crate) fn setup_socket(&self, socket: &Socket) -> std::io::Result<()> {
        if let Some(size) = self.recv_buffer_size {
            socket.socket.set_recv_buffer_size(size)?;
        }
        if let Some(size) = self.send_buffer_size {
            socket.socket.set_send_buffer_size(size)?;
        }

        socket.socket.set_keepalive(self.keepalive)?;
        socket.socket.set_linger(self.linger)?;
        socket.socket.set_read_timeout(self.read_timeout)?;
        socket.socket.set_write_timeout(self.write_timeout)?;
        socket.socket.set_reuse_address(self.reuse_address)?;
        #[cfg(all(
            unix,
            not(any(target_os = "illumos", target_os = "solaris", target_os = "cygwin"))
        ))]
        socket.socket.set_reuse_port(self.reuse_port)?;
        socket.socket.set_nodelay(self.no_delay)?;
        Ok(())
    }
}
