use std::time::Duration;

use compio_io::{AsyncRead, AsyncWrite, util::Splittable};

use crate::{
    client::{self, ClientConnection, SendRequest},
    error::H2Error,
    proto::{ping_pong::PingPong, settings::ConnSettings},
    server::ServerConnection,
    state::ConnExtra,
};

/// Shared builder settings wrapping `ConnSettings` and keepalive configuration.
#[derive(Clone)]
struct BuilderSettings {
    settings: ConnSettings,
    keepalive_interval: Option<Duration>,
    keepalive_timeout: Option<Duration>,
    initial_connection_window_size: Option<u32>,
    max_concurrent_reset_streams: Option<usize>,
    reset_stream_duration: Option<Duration>,
    max_send_buffer_size: Option<usize>,
}

impl BuilderSettings {
    fn new() -> Self {
        BuilderSettings {
            settings: ConnSettings::new(),
            keepalive_interval: None,
            keepalive_timeout: None,
            initial_connection_window_size: None,
            max_concurrent_reset_streams: None,
            reset_stream_duration: None,
            max_send_buffer_size: None,
        }
    }

    fn build_ping_pong(&self) -> PingPong {
        match (self.keepalive_interval, self.keepalive_timeout) {
            (Some(interval), Some(timeout)) => PingPong::new(interval, timeout),
            (Some(interval), None) => PingPong::new(interval, Duration::from_secs(10)),
            _ => PingPong::disabled(),
        }
    }

    fn build_extra(&self) -> ConnExtra {
        ConnExtra {
            max_concurrent_reset_streams: self.max_concurrent_reset_streams,
            reset_stream_duration: self.reset_stream_duration,
            max_send_buffer_size: self.max_send_buffer_size,
        }
    }
}

/// Generate shared builder setter methods.
macro_rules! impl_builder_setters {
    ($builder:ident) => {
        impl $builder {
            /// Create a new builder with default HTTP/2 settings.
            pub fn new() -> Self {
                $builder {
                    inner: BuilderSettings::new(),
                }
            }

            /// Set the initial flow control window size for new streams.
            pub fn initial_window_size(mut self, size: u32) -> Self {
                self.inner.settings.set_local_initial_window_size(size);
                self
            }

            /// Set the maximum number of concurrent streams.
            pub fn max_concurrent_streams(mut self, max: u32) -> Self {
                self.inner.settings.set_local_max_concurrent_streams(max);
                self
            }

            /// Set the maximum frame payload size.
            pub fn max_frame_size(mut self, size: u32) -> Self {
                self.inner.settings.set_local_max_frame_size(size);
                self
            }

            /// Set the maximum header list size.
            pub fn max_header_list_size(mut self, size: u32) -> Self {
                self.inner.settings.set_local_max_header_list_size(size);
                self
            }

            /// Set the HPACK dynamic table size.
            pub fn header_table_size(mut self, size: u32) -> Self {
                self.inner.settings.set_local_header_table_size(size);
                self
            }

            /// Enable or disable server push.
            pub fn enable_push(mut self, enabled: bool) -> Self {
                self.inner.settings.set_local_enable_push(enabled);
                self
            }

            /// Set the initial connection-level flow control window size.
            ///
            /// Separate from per-stream window size; a WINDOW_UPDATE is sent
            /// after handshake if larger than the default 65,535.
            pub fn initial_connection_window_size(mut self, size: u32) -> Self {
                self.inner.initial_connection_window_size = Some(size);
                self
            }

            /// Set the keepalive PING interval.
            pub fn keepalive_interval(mut self, interval: Duration) -> Self {
                self.inner.keepalive_interval = Some(interval);
                self
            }

            /// Set the keepalive PING timeout (how long to wait for ACK).
            pub fn keepalive_timeout(mut self, timeout: Duration) -> Self {
                self.inner.keepalive_timeout = Some(timeout);
                self
            }

            /// Set the maximum number of concurrent reset streams before triggering
            /// GOAWAY with ENHANCE_YOUR_CALM (CVE-2023-44487 protection).
            ///
            /// This is a **local policy**, not an HTTP/2 SETTINGS parameter — it is
            /// never sent to the peer.
            ///
            /// Default: 50 resets within the reset stream duration window.
            pub fn max_concurrent_reset_streams(mut self, max: usize) -> Self {
                self.inner.max_concurrent_reset_streams = Some(max);
                self
            }

            /// Set the duration window for counting reset streams.
            ///
            /// This is a **local policy**, not an HTTP/2 SETTINGS parameter — it is
            /// never sent to the peer.
            ///
            /// Default: 1 second.
            pub fn reset_stream_duration(mut self, duration: Duration) -> Self {
                self.inner.reset_stream_duration = Some(duration);
                self
            }

            /// Set the maximum send buffer size in bytes.
            ///
            /// When the total pending data queued for sending exceeds this limit,
            /// further send attempts will return an error instead of queuing.
            ///
            /// This is a **local policy**, not an HTTP/2 SETTINGS parameter — it is
            /// never sent to the peer.
            ///
            /// Default: 409,600 bytes (400 KiB).
            pub fn max_send_buffer_size(mut self, size: usize) -> Self {
                self.inner.max_send_buffer_size = Some(size);
                self
            }
        }

        impl Default for $builder {
            fn default() -> Self {
                Self::new()
            }
        }
    };
}

/// Builder for HTTP/2 client connections.
pub struct ClientBuilder {
    inner: BuilderSettings,
}

impl_builder_setters!(ClientBuilder);

impl ClientBuilder {
    /// Perform the HTTP/2 client handshake.
    pub async fn handshake<IO>(
        self,
        io: IO,
    ) -> Result<(SendRequest, ClientConnection<IO::ReadHalf, IO::WriteHalf>), H2Error>
    where
        IO: Splittable + 'static,
        IO::ReadHalf: AsyncRead + 'static,
        IO::WriteHalf: AsyncWrite + 'static,
    {
        let ping_pong = self.inner.build_ping_pong();
        let extra = self.inner.build_extra();
        client::handshake_with_settings(
            io,
            self.inner.settings,
            ping_pong,
            self.inner.initial_connection_window_size,
            extra,
        )
        .await
    }
}

/// Builder for HTTP/2 server connections.
pub struct ServerBuilder {
    inner: BuilderSettings,
}

impl_builder_setters!(ServerBuilder);

impl ServerBuilder {
    /// Perform the HTTP/2 server handshake.
    pub async fn handshake<IO>(self, io: IO) -> Result<ServerConnection, H2Error>
    where
        IO: Splittable + 'static,
        IO::ReadHalf: AsyncRead + 'static,
        IO::WriteHalf: AsyncWrite + 'static,
    {
        let ping_pong = self.inner.build_ping_pong();
        let extra = self.inner.build_extra();
        crate::server::handshake_with_settings(
            io,
            self.inner.settings,
            ping_pong,
            self.inner.initial_connection_window_size,
            extra,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_builder_defaults() {
        let builder = ClientBuilder::new();
        // Should have disabled keepalive by default
        let pp = builder.inner.build_ping_pong();
        assert!(!pp.is_enabled());
    }

    #[test]
    fn test_client_builder_keepalive() {
        let builder = ClientBuilder::new()
            .keepalive_interval(Duration::from_secs(30))
            .keepalive_timeout(Duration::from_secs(5));
        let pp = builder.inner.build_ping_pong();
        assert!(pp.is_enabled());
    }

    #[test]
    fn test_server_builder_defaults() {
        let builder = ServerBuilder::new();
        let pp = builder.inner.build_ping_pong();
        assert!(!pp.is_enabled());
    }

    #[test]
    fn test_builder_chaining() {
        let _builder = ClientBuilder::new()
            .initial_window_size(1 << 20)
            .initial_connection_window_size(1 << 24)
            .max_concurrent_streams(100)
            .max_frame_size(32768)
            .max_header_list_size(8192)
            .header_table_size(8192)
            .enable_push(false)
            .keepalive_interval(Duration::from_secs(20))
            .keepalive_timeout(Duration::from_secs(10));
    }

    #[test]
    fn test_connection_window_size_default_none() {
        let builder = ClientBuilder::new();
        assert!(builder.inner.initial_connection_window_size.is_none());
    }

    #[test]
    fn test_connection_window_size_set() {
        let builder = ClientBuilder::new().initial_connection_window_size(1 << 24);
        assert_eq!(builder.inner.initial_connection_window_size, Some(1 << 24));
    }

    #[test]
    fn test_max_concurrent_reset_streams() {
        let builder = ServerBuilder::new().max_concurrent_reset_streams(100);
        assert_eq!(builder.inner.max_concurrent_reset_streams, Some(100));
    }

    #[test]
    fn test_reset_stream_duration() {
        let builder = ServerBuilder::new().reset_stream_duration(Duration::from_secs(2));
        assert_eq!(
            builder.inner.reset_stream_duration,
            Some(Duration::from_secs(2))
        );
    }

    #[test]
    fn test_max_send_buffer_size() {
        let builder = ClientBuilder::new().max_send_buffer_size(1024 * 1024);
        assert_eq!(builder.inner.max_send_buffer_size, Some(1024 * 1024));
    }

    #[test]
    fn test_build_extra() {
        let builder = ServerBuilder::new()
            .max_concurrent_reset_streams(30)
            .reset_stream_duration(Duration::from_millis(500))
            .max_send_buffer_size(200_000);
        let extra = builder.inner.build_extra();
        assert_eq!(extra.max_concurrent_reset_streams, Some(30));
        assert_eq!(
            extra.reset_stream_duration,
            Some(Duration::from_millis(500))
        );
        assert_eq!(extra.max_send_buffer_size, Some(200_000));
    }
}
