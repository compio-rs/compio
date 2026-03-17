//! Shared stream handles: [`SendStream`], [`RecvStream`], and
//! [`RecvFlowControl`].

use std::future::poll_fn;
use std::task::Poll;

use bytes::Bytes;

use crate::{
    error::{H2Error, Reason},
    frame::StreamId,
    state::SharedState,
};

/// Handle for releasing recv flow control capacity on an HTTP/2 stream.
pub struct RecvFlowControl {
    stream_id: StreamId,
    unreleased: u32,
    state: SharedState,
}

impl RecvFlowControl {
    pub(crate) fn new(stream_id: StreamId, state: SharedState) -> Self {
        RecvFlowControl {
            stream_id,
            unreleased: 0,
            state,
        }
    }

    /// Bytes received but not yet released.
    pub fn unreleased(&self) -> usize {
        self.unreleased as usize
    }

    /// Release `sz` bytes of flow control capacity back to the peer.
    pub fn release_capacity(&mut self, sz: usize) -> Result<(), H2Error> {
        let sz = sz as u32;
        if sz > self.unreleased {
            return Err(H2Error::Protocol(format!(
                "release_capacity({}) exceeds unreleased bytes ({})",
                sz, self.unreleased
            )));
        }
        self.unreleased -= sz;
        let mut s = self.state.borrow_mut();
        let crossed_threshold = s.streams.apply_release(&self.stream_id, sz);
        if crossed_threshold {
            s.wake_io(); // IO task will send WINDOW_UPDATE
        }
        Ok(())
    }

    pub(crate) fn add_unreleased(&mut self, amount: u32) {
        self.unreleased += amount;
    }
}

/// Handle for sending data and trailers on an HTTP/2 stream.
pub struct SendStream {
    stream_id: StreamId,
    state: SharedState,
    /// Bytes of send capacity currently reserved by the application.
    reserved: u32,
}

impl SendStream {
    pub(crate) fn new(stream_id: StreamId, state: SharedState) -> Self {
        SendStream {
            stream_id,
            state,
            reserved: 0,
        }
    }

    /// Stream id.
    pub fn stream_id(&self) -> StreamId {
        self.stream_id
    }

    /// Currently reserved send capacity in bytes.
    pub fn capacity(&self) -> usize {
        self.reserved as usize
    }

    /// Request send capacity on this stream.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe.
    pub async fn reserve_capacity(&mut self, sz: usize) -> Result<(), H2Error> {
        let target = sz as u32;
        if self.reserved >= target {
            return Ok(());
        }
        let needed = target - self.reserved;

        let granted = poll_fn(|cx| {
            let mut s = self.state.borrow_mut();
            s.check_error()?;
            if let Some(stream) = s.streams.get(&self.stream_id) {
                if !stream.state.can_send() {
                    return Poll::Ready(Err(H2Error::Protocol(
                        "stream is not in a sendable state".into(),
                    )));
                }
                let avail = std::cmp::min(
                    s.conn_send_flow.available(),
                    stream.send_flow.available(),
                );
                let grant = std::cmp::min(needed, avail);
                if grant > 0 {
                    return Poll::Ready(Ok(grant));
                }
            } else {
                return Poll::Ready(Err(H2Error::Protocol("stream not found".into())));
            }
            s.writable.insert(self.stream_id, cx.waker().clone());
            Poll::Pending
        })
        .await?;

        self.reserved += granted;
        Ok(())
    }

    /// Wait for send capacity to become available on this stream.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe.
    pub async fn poll_capacity(&mut self) -> Option<Result<usize, H2Error>> {
        if self.reserved > 0 {
            return Some(Ok(self.reserved as usize));
        }

        let result = poll_fn(|cx| {
            let mut s = self.state.borrow_mut();
            if let Some(ref e) = s.error {
                return Poll::Ready(Err(e.clone()));
            }
            if let Some(stream) = s.streams.get(&self.stream_id) {
                if !stream.state.can_send() {
                    return Poll::Ready(Err(H2Error::Protocol(
                        "stream is not in a sendable state".into(),
                    )));
                }
                let avail = std::cmp::min(
                    s.conn_send_flow.available(),
                    stream.send_flow.available(),
                );
                if avail > 0 {
                    return Poll::Ready(Ok(avail));
                }
            } else {
                return Poll::Ready(Err(H2Error::Protocol("stream not found".into())));
            }
            s.writable.insert(self.stream_id, cx.waker().clone());
            Poll::Pending
        })
        .await;

        match result {
            Ok(granted) => {
                self.reserved += granted;
                Some(Ok(self.reserved as usize))
            }
            Err(e) => Some(Err(e)),
        }
    }

    /// Send data on this stream.
    ///
    /// # Cancel safety
    ///
    /// This method is *not* cancel safe.
    pub async fn send_data(
        &mut self,
        data: impl Into<Bytes>,
        end_of_stream: bool,
    ) -> Result<(), H2Error> {
        let data = data.into();
        let len = data.len() as u32;
        self.reserved = self.reserved.saturating_sub(len);

        // Try to encode directly into write buffer (fast path)
        {
            let mut s = self.state.borrow_mut();
            s.check_error()?;
            if s.encode_data(self.stream_id, &data, end_of_stream)? {
                s.wake_io();
                return Ok(());
            }
        }

        // Flow control blocked — queue as pending send and wait for IO driver
        {
            let mut s = self.state.borrow_mut();
            // Enforce send buffer limit
            if s.pending_send_bytes + data.len() > s.max_send_buffer_size {
                return Err(H2Error::Protocol(format!(
                    "send buffer full: {} + {} > {} bytes",
                    s.pending_send_bytes,
                    data.len(),
                    s.max_send_buffer_size
                )));
            }
            s.pending_send_bytes += data.len();
            s.pending_sends.push(crate::state::PendingSend {
                stream_id: self.stream_id,
                data,
                end_stream: end_of_stream,
                waker: None,
            });
            s.wake_io();
        }

        // Wait for the IO driver to flush this pending send.
        // The IO driver removes the item from pending_sends when flushed,
        // then wakes our waker. If we don't find our item, it was sent.
        poll_fn(|cx| {
            let mut s = self.state.borrow_mut();
            s.check_error()?;

            for ps in &mut s.pending_sends {
                if ps.stream_id == self.stream_id {
                    ps.waker = Some(cx.waker().clone());
                    return Poll::Pending;
                }
            }

            // Item removed from queue — send completed
            Poll::Ready(Ok(()))
        })
        .await
    }

    /// Send a RST_STREAM frame to reset this stream.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe.
    pub async fn send_reset(&self, reason: Reason) -> Result<(), H2Error> {
        let mut s = self.state.borrow_mut();
        s.check_error()?;
        s.encode_rst_stream(self.stream_id, reason);
        if let Some(stream) = s.streams.get_mut(&self.stream_id) {
            stream.state = stream.state.reset();
        }
        s.close_stream_recv(&self.stream_id);
        s.wake_io();
        Ok(())
    }

    /// Send trailers on this stream (implicitly sets END_STREAM).
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe.
    pub async fn send_trailers(&mut self, trailers: http::HeaderMap) -> Result<(), H2Error> {
        let trailer_vec: Vec<(Bytes, Bytes)> = trailers
            .iter()
            .map(|(k, v)| {
                (
                    Bytes::copy_from_slice(k.as_str().as_bytes()),
                    Bytes::copy_from_slice(v.as_bytes()),
                )
            })
            .collect();

        let mut s = self.state.borrow_mut();
        s.check_error()?;
        s.encode_headers(self.stream_id, &trailer_vec, true)?;
        if let Some(stream) = s.streams.get_mut(&self.stream_id) {
            stream.state = stream.state.send_headers(true)?;
        }
        s.wake_io();
        Ok(())
    }

    /// Wait for a RST_STREAM from the peer.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe.
    pub async fn poll_reset(&mut self) -> Result<Reason, H2Error> {
        poll_fn(|cx| {
            let s = self.state.borrow();
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
            drop(s);
            let mut s = self.state.borrow_mut();
            s.readable.insert(self.stream_id, cx.waker().clone());
            Poll::Pending
        })
        .await
    }
}

/// Handle for receiving data and trailers on an HTTP/2 stream.
pub struct RecvStream {
    stream_id: StreamId,
    flow_control: RecvFlowControl,
    state: SharedState,
}

impl RecvStream {
    pub(crate) fn new(stream_id: StreamId, state: SharedState) -> Self {
        RecvStream {
            stream_id,
            flow_control: RecvFlowControl::new(stream_id, state.clone()),
            state,
        }
    }

    /// The stream identifier.
    pub fn stream_id(&self) -> StreamId {
        self.stream_id
    }

    /// Mutable reference to the recv flow control handle.
    pub fn flow_control(&mut self) -> &mut RecvFlowControl {
        &mut self.flow_control
    }

    /// Receive the next chunk of data. Returns `None` when the stream ends.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe.
    pub async fn data(&mut self) -> Option<Result<Bytes, H2Error>> {
        poll_fn(|cx| {
            let mut s = self.state.borrow_mut();

            // Check for buffered data
            if let Some(stream) = s.streams.get_mut(&self.stream_id) {
                if let Some(item) = stream.data_buf.pop_front() {
                    match item {
                        Ok(bytes) => {
                            self.flow_control.add_unreleased(bytes.len() as u32);
                            return Poll::Ready(Some(Ok(bytes)));
                        }
                        Err(e) => return Poll::Ready(Some(Err(e))),
                    }
                }
                if stream.recv_closed {
                    return Poll::Ready(None);
                }
            } else {
                return Poll::Ready(None);
            }

            // Check connection error
            if let Some(ref e) = s.error {
                return Poll::Ready(Some(Err(e.clone())));
            }

            s.readable.insert(self.stream_id, cx.waker().clone());
            Poll::Pending
        })
        .await
    }

    /// Receive trailers. Returns `None` if no trailers were sent.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe.
    pub async fn trailers(&self) -> Option<Result<http::HeaderMap, H2Error>> {
        poll_fn(|cx| {
            let mut s = self.state.borrow_mut();

            if let Some(stream) = s.streams.get_mut(&self.stream_id) {
                if let Some(trailers) = stream.trailers_buf.take() {
                    return Poll::Ready(Some(trailers));
                }
                if stream.recv_closed {
                    return Poll::Ready(None);
                }
            } else {
                return Poll::Ready(None);
            }

            if let Some(ref e) = s.error {
                return Poll::Ready(Some(Err(e.clone())));
            }

            s.readable.insert(self.stream_id, cx.waker().clone());
            Poll::Pending
        })
        .await
    }
}
