//! HTTP/2 server API.

use std::{future::poll_fn, task::Poll};

use bytes::Bytes;
use compio_io::{AsyncRead, AsyncWrite, util::Splittable};

use crate::{
    error::{H2Error, Reason},
    frame::StreamId,
    proto::{ping_pong::PingPong, settings::ConnSettings},
    share::{RecvStream, SendStream},
    state::{ConnExtra, IncomingStream, SharedState, new_shared_state},
};

/// Create a new server builder for configuring HTTP/2 connection settings.
pub fn builder() -> crate::builder::ServerBuilder {
    crate::builder::ServerBuilder::new()
}

/// Perform HTTP/2 server handshake with default settings.
pub async fn handshake<IO>(io: IO) -> Result<ServerConnection, H2Error>
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

/// Perform HTTP/2 server handshake with explicit settings and keepalive.
pub async fn handshake_with_settings<IO>(
    io: IO,
    settings: ConnSettings,
    ping_pong: PingPong,
    initial_connection_window_size: Option<u32>,
    extra: ConnExtra,
) -> Result<ServerConnection, H2Error>
where
    IO: Splittable + 'static,
    IO::ReadHalf: AsyncRead + 'static,
    IO::WriteHalf: AsyncWrite + 'static,
{
    let (read_half, write_half) = io.split();

    let state = new_shared_state(
        false,
        settings,
        ping_pong,
        initial_connection_window_size,
        extra,
    );

    let state_for_io = state.clone();
    let (closed_tx, closed_rx) = flume::bounded::<Result<(), H2Error>>(1);

    // Spawn the IO driver
    compio_runtime::spawn(async move {
        let result =
            crate::proto::connection::run_server_io(state_for_io, read_half, write_half).await;
        if let Err(ref _err) = result {
            compio_log::error!("server connection error: {}", _err);
        }
        let _ = closed_tx.send(result);
    })
    .detach();

    Ok(ServerConnection { state, closed_rx })
}

/// Server-side HTTP/2 connection handle.
pub struct ServerConnection {
    state: SharedState,
    closed_rx: flume::Receiver<Result<(), H2Error>>,
}

impl ServerConnection {
    /// Initiate a graceful shutdown by sending a GOAWAY frame.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe.
    pub async fn shutdown(&self) -> Result<(), H2Error> {
        let mut s = self.state.borrow_mut();
        s.check_error()?;
        let last_stream_id = s.last_peer_stream_id;
        s.encode_goaway(last_stream_id, Reason::NoError);
        s.going_away = true;
        s.wake_io();
        Ok(())
    }

    /// Initiate an abrupt shutdown.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe.
    pub async fn abrupt_shutdown(&self, reason: Reason) -> Result<(), H2Error> {
        let mut s = self.state.borrow_mut();
        s.check_error()?;
        let last_stream_id = s.last_peer_stream_id;
        s.encode_goaway(last_stream_id, reason);

        let stream_ids: Vec<StreamId> = s.streams.iter_ids().collect();
        for id in &stream_ids {
            if let Some(stream) = s.streams.get_mut(id)
                && !stream.state.is_closed()
            {
                stream.state = stream.state.reset();
                stream.data_buf.push_back(Err(H2Error::connection(reason)));
                stream.recv_closed = true;
            }
        }
        s.going_away = true;
        s.wake_all_senders();
        s.wake_all_receivers();
        s.drain_ready_waiters();
        s.wake_io();
        Ok(())
    }

    /// Wait for the connection background task to complete.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe.
    pub async fn closed(&mut self) -> Result<(), H2Error> {
        self.closed_rx
            .recv_async()
            .await
            .map_err(|_| H2Error::Protocol("connection task dropped".into()))?
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
            return Err(H2Error::connection(Reason::FlowControlError));
        }
        let current = s.conn_recv_flow.window_size();
        let target = size as i32;
        if target > current {
            let increment = (target - current) as u32;
            s.encode_window_update(StreamId::ZERO, increment);
            s.conn_recv_flow
                .release(increment)
                .map_err(|_| H2Error::connection(Reason::FlowControlError))?;
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
            return Err(H2Error::connection(Reason::FlowControlError));
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
                        .map_err(|_| H2Error::connection(Reason::FlowControlError))?;
                }
            }
        }
        s.wake_io();
        Ok(())
    }

    /// Accept the next incoming request stream.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe.
    pub async fn accept(
        &mut self,
    ) -> Option<Result<(http::Request<RecvStream>, SendResponse), H2Error>> {
        let incoming = poll_fn(|cx| {
            let mut s = self.state.borrow_mut();

            if let Some(item) = s.incoming_streams.pop_front() {
                return Poll::Ready(Some(item));
            }

            if s.error.is_some() || s.going_away {
                // Check if there are more queued
                if let Some(item) = s.incoming_streams.pop_front() {
                    return Poll::Ready(Some(item));
                }
                return Poll::Ready(None);
            }

            s.accept_waiters.push_back(cx.waker().clone());
            Poll::Pending
        })
        .await;

        match incoming {
            Some(Ok(incoming)) => {
                let result = self.build_request(incoming);
                Some(result)
            }
            Some(Err(e)) => Some(Err(e)),
            None => None,
        }
    }

    fn build_request(
        &self,
        incoming: IncomingStream,
    ) -> Result<(http::Request<RecvStream>, SendResponse), H2Error> {
        let recv_stream = RecvStream::new(incoming.stream_id, self.state.clone());

        let mut method = None;
        let mut scheme = None;
        let mut path = None;
        let mut authority = None;
        let mut regular_headers = http::HeaderMap::new();

        for dh in &incoming.headers {
            if &dh.name[..] == b":method" {
                method = Some(
                    http::Method::from_bytes(&dh.value)
                        .map_err(|e| H2Error::Protocol(format!("invalid method: {}", e)))?,
                );
            } else if &dh.name[..] == b":scheme" {
                scheme = Some(String::from_utf8_lossy(&dh.value).to_string());
            } else if &dh.name[..] == b":path" {
                path = Some(String::from_utf8_lossy(&dh.value).to_string());
            } else if &dh.name[..] == b":authority" {
                authority = Some(String::from_utf8_lossy(&dh.value).to_string());
            } else if let (Ok(hname), Ok(hvalue)) = (
                http::header::HeaderName::from_bytes(&dh.name),
                http::header::HeaderValue::from_bytes(&dh.value),
            ) {
                regular_headers.append(hname, hvalue);
            }
        }

        let method = method.unwrap_or(http::Method::GET);
        let path = path.unwrap_or_else(|| "/".to_string());

        let uri_str = if let Some(authority) = &authority {
            let s = scheme.as_deref().unwrap_or("https");
            format!("{}://{}{}", s, authority, path)
        } else {
            path
        };

        let uri: http::Uri = uri_str
            .parse()
            .map_err(|e| H2Error::Protocol(format!("invalid URI: {}", e)))?;

        let mut request = http::Request::builder()
            .method(method)
            .uri(uri)
            .body(recv_stream)
            .map_err(|e| H2Error::Protocol(format!("failed to build request: {}", e)))?;

        *request.headers_mut() = regular_headers;

        let send_response = SendResponse {
            stream_id: incoming.stream_id,
            state: self.state.clone(),
        };

        Ok((request, send_response))
    }
}

/// Handle for sending a response on a server stream.
pub struct SendResponse {
    stream_id: StreamId,
    state: SharedState,
}

impl SendResponse {
    /// Send the response headers.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe.
    pub async fn send_response(
        &mut self,
        response: http::Response<()>,
        end_of_stream: bool,
    ) -> Result<Option<SendStream>, H2Error> {
        let mut headers = Vec::new();
        headers.push((
            Bytes::from_static(b":status"),
            Bytes::copy_from_slice(response.status().as_str().as_bytes()),
        ));

        for (name, value) in response.headers() {
            headers.push((
                Bytes::copy_from_slice(name.as_str().as_bytes()),
                Bytes::copy_from_slice(value.as_bytes()),
            ));
        }

        {
            let mut s = self.state.borrow_mut();
            s.check_error()?;
            s.encode_headers(self.stream_id, &headers, end_of_stream)?;
            if let Some(stream) = s.streams.get_mut(&self.stream_id) {
                stream.state = stream.state.send_headers(end_of_stream)?;
            }
            s.wake_io();
        }

        if end_of_stream {
            Ok(None)
        } else {
            Ok(Some(SendStream::new(self.stream_id, self.state.clone())))
        }
    }

    /// Wait for the peer to send a RST_STREAM on this stream.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe.
    pub async fn poll_reset(&mut self) -> Result<Reason, H2Error> {
        poll_fn(|cx| {
            let mut s = self.state.borrow_mut();
            if let Some(stream) = s.streams.get(&self.stream_id) {
                if let Some(reason) = stream.reset_reason {
                    return Poll::Ready(Ok(reason));
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
        .await
    }
}
