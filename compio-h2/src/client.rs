//! HTTP/2 client API.

use std::future::poll_fn;
use std::task::Poll;

use bytes::Bytes;
use compio_io::{AsyncRead, AsyncWrite, util::Splittable};

use crate::{
    error::H2Error,
    frame::StreamId,
    proto::{ping_pong::PingPong, settings::ConnSettings},
    share::{RecvStream, SendStream},
    state::{ConnExtra, SharedState, new_shared_state},
};

/// Create a new client builder for configuring HTTP/2 connection settings.
pub fn builder() -> crate::builder::ClientBuilder {
    crate::builder::ClientBuilder::new()
}

/// Perform HTTP/2 client handshake with default settings.
///
/// The connection handle must be spawned as a background task for the client to
/// function:
///
/// ```no_run
/// # compio_runtime::Runtime::new().unwrap().block_on(async {
/// use compio_h2::client;
/// use compio_net::TcpStream;
///
/// let tcp = TcpStream::connect("127.0.0.1:8080").await.unwrap();
/// let (send_request, connection) = client::handshake(tcp).await.unwrap();
/// compio_runtime::spawn(connection.run()).detach();
/// # });
/// ```
pub async fn handshake<IO>(
    io: IO,
) -> Result<(SendRequest, ClientConnection<IO::ReadHalf, IO::WriteHalf>), H2Error>
where
    IO: Splittable + 'static,
    IO::ReadHalf: AsyncRead + 'static,
    IO::WriteHalf: AsyncWrite + 'static,
{
    handshake_with_settings(
        io,
        ConnSettings::new(),
        PingPong::disabled(),
        None,
        ConnExtra::default(),
    )
    .await
}

/// Perform HTTP/2 client handshake with explicit settings and keepalive.
pub async fn handshake_with_settings<IO>(
    io: IO,
    settings: ConnSettings,
    ping_pong: PingPong,
    initial_connection_window_size: Option<u32>,
    extra: ConnExtra,
) -> Result<(SendRequest, ClientConnection<IO::ReadHalf, IO::WriteHalf>), H2Error>
where
    IO: Splittable + 'static,
    IO::ReadHalf: AsyncRead + 'static,
    IO::WriteHalf: AsyncWrite + 'static,
{
    let (read_half, write_half) = io.split();

    let state = new_shared_state(true, settings, ping_pong, initial_connection_window_size, extra);

    let send_request = SendRequest {
        state: state.clone(),
    };

    let conn = ClientConnection {
        state,
        read_half,
        write_half,
    };

    Ok((send_request, conn))
}

/// Handle for sending requests on a client connection.
#[derive(Clone)]
pub struct SendRequest {
    state: SharedState,
}

impl SendRequest {
    /// Wait until the connection can accept a new stream.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe.
    pub async fn ready(&mut self) -> Result<(), H2Error> {
        poll_fn(|cx| {
            let mut s = self.state.borrow_mut();
            s.check_error()?;
            if s.going_away {
                return Poll::Ready(Err(H2Error::connection(
                    crate::error::Reason::RefusedStream,
                )));
            }
            if s.streams.can_accept_stream() {
                return Poll::Ready(Ok(()));
            }
            s.ready_waiters.push_back(cx.waker().clone());
            Poll::Pending
        })
        .await
    }

    /// Initiate a graceful shutdown by sending a GOAWAY frame.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe.
    pub async fn shutdown(&self) -> Result<(), H2Error> {
        let mut s = self.state.borrow_mut();
        s.check_error()?;
        let last_stream_id = s.last_peer_stream_id;
        s.encode_goaway(last_stream_id, crate::error::Reason::NoError);
        s.going_away = true;
        s.wake_io();
        Ok(())
    }

    /// Set the target connection-level receive window size at runtime.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe.
    pub async fn set_target_window_size(&self, size: u32) -> Result<(), H2Error> {
        let mut s = self.state.borrow_mut();
        s.check_error()?;
        if size > 0x7FFF_FFFF {
            return Err(H2Error::connection(crate::error::Reason::FlowControlError));
        }
        let current = s.conn_recv_flow.window_size();
        let target = size as i32;
        if target > current {
            let increment = (target - current) as u32;
            s.encode_window_update(StreamId::ZERO, increment);
            s.conn_recv_flow
                .release(increment)
                .map_err(|_| H2Error::connection(crate::error::Reason::FlowControlError))?;
        }
        s.wake_io();
        Ok(())
    }

    /// Set the initial stream-level window size via a SETTINGS frame.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe.
    pub async fn set_initial_window_size(&self, size: u32) -> Result<(), H2Error> {
        let mut s = self.state.borrow_mut();
        s.check_error()?;
        if size > 0x7FFF_FFFF {
            return Err(H2Error::connection(crate::error::Reason::FlowControlError));
        }
        let old_size = s.settings.local().initial_window_size as i32;
        let new_size = size as i32;
        s.settings.set_local_initial_window_size(size);
        if let Some(frame) = s.settings.build_local_settings() {
            s.encode_settings(&frame);
        }
        let delta = new_size - old_size;
        if delta != 0 {
            let stream_ids: Vec<StreamId> = s.streams.iter_ids().collect();
            for id in stream_ids {
                if let Some(stream) = s.streams.get_mut(&id) {
                    if stream.state.is_closed() {
                        continue;
                    }
                    stream
                        .recv_flow
                        .update_initial_window_size(new_size)
                        .map_err(|_| {
                            H2Error::connection(crate::error::Reason::FlowControlError)
                        })?;
                }
            }
        }
        s.wake_io();
        Ok(())
    }

    /// Send an HTTP/2 request.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe.
    pub async fn send_request(
        &mut self,
        request: http::Request<()>,
        end_of_stream: bool,
    ) -> Result<(ResponseFuture, Option<SendStream>), H2Error> {
        let mut headers = Vec::new();
        headers.push((
            Bytes::from_static(b":method"),
            Bytes::copy_from_slice(request.method().as_str().as_bytes()),
        ));
        headers.push((
            Bytes::from_static(b":scheme"),
            Bytes::copy_from_slice(request.uri().scheme_str().unwrap_or("https").as_bytes()),
        ));
        headers.push((
            Bytes::from_static(b":path"),
            Bytes::copy_from_slice(
                request
                    .uri()
                    .path_and_query()
                    .map(|pq| pq.as_str())
                    .unwrap_or("/")
                    .as_bytes(),
            ),
        ));
        if let Some(authority) = request.uri().authority() {
            headers.push((
                Bytes::from_static(b":authority"),
                Bytes::copy_from_slice(authority.as_str().as_bytes()),
            ));
        }
        for (name, value) in request.headers() {
            headers.push((
                Bytes::copy_from_slice(name.as_str().as_bytes()),
                Bytes::copy_from_slice(value.as_bytes()),
            ));
        }

        let stream_id = {
            let mut s = self.state.borrow_mut();
            s.check_error()?;
            if s.going_away {
                return Err(H2Error::connection(crate::error::Reason::RefusedStream));
            }
            if !s.streams.can_accept_stream() {
                return Err(H2Error::Protocol("max concurrent streams exceeded".into()));
            }
            let stream_id = s.streams.next_stream_id()?;
            let initial_send_window = s.settings.remote().initial_window_size as i32;
            let initial_recv_window = s.settings.local().initial_window_size as i32;
            s.streams
                .insert(stream_id, initial_send_window, initial_recv_window);

            s.encode_headers(stream_id, &headers, end_of_stream)?;

            if let Some(stream) = s.streams.get_mut(&stream_id) {
                stream.state = stream.state.send_headers(end_of_stream)?;
            }

            s.wake_io();
            stream_id
        };

        let response_future = ResponseFuture {
            stream_id,
            state: self.state.clone(),
        };

        let send_stream = if !end_of_stream {
            Some(SendStream::new(stream_id, self.state.clone()))
        } else {
            None
        };

        Ok((response_future, send_stream))
    }
}

/// Future that resolves to an HTTP response.
pub struct ResponseFuture {
    stream_id: StreamId,
    state: SharedState,
}

impl ResponseFuture {
    /// Wait for the response headers.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe.
    pub async fn await_response(self) -> Result<http::Response<RecvStream>, H2Error> {
        let (status, headers) = poll_fn(|cx| {
            let mut s = self.state.borrow_mut();

            if let Some(stream) = s.streams.get_mut(&self.stream_id) {
                if let Some(result) = stream.response_headers.take() {
                    return Poll::Ready(result);
                }
                // Stream was reset by peer — no response headers coming
                if let Some(reason) = stream.reset_reason {
                    return Poll::Ready(Err(H2Error::stream_remote(
                        self.stream_id.value(),
                        reason,
                    )));
                }
                // Stream recv closed without headers (e.g., RST_STREAM)
                if stream.recv_closed {
                    return Poll::Ready(Err(H2Error::Protocol(
                        "stream closed before response headers".into(),
                    )));
                }
            } else {
                return Poll::Ready(Err(H2Error::Protocol("stream not found".into())));
            }

            if let Some(ref e) = s.error {
                return Poll::Ready(Err(e.clone()));
            }

            s.readable.insert(self.stream_id, cx.waker().clone());
            Poll::Pending
        })
        .await?;

        let recv_stream = RecvStream::new(self.stream_id, self.state.clone());

        let mut response = http::Response::builder()
            .status(status)
            .body(recv_stream)
            .map_err(|e| H2Error::Protocol(format!("failed to build response: {}", e)))?;

        *response.headers_mut() = headers;

        Ok(response)
    }
}

/// Client connection handle. Must be spawned as a background task.
pub struct ClientConnection<R, W> {
    state: SharedState,
    read_half: R,
    write_half: W,
}

impl<R: AsyncRead + 'static, W: AsyncWrite + 'static> ClientConnection<R, W> {
    /// Run the client connection IO driver.
    pub async fn run(self) -> Result<(), H2Error> {
        crate::proto::connection::run_client_io(self.state, self.read_half, self.write_half).await
    }
}
