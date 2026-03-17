//! Shared connection state accessed directly by user handles and the IO driver.
//!
//! This replaces the channel-based command pattern with direct state machine
//! access (like compio-quic). User operations (send_data, send_request, etc.)
//! lock the state, encode frames into the write buffer, and wake the IO task.
//! The IO task reads frames from TCP, processes them into the state, and
//! flushes the write buffer.

use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    rc::Rc,
    task::Waker,
};

use bytes::Bytes;

use crate::{
    error::{H2Error, Reason},
    frame::{self, Frame, StreamId},
    hpack::{DecodedHeader, Decoder as HpackDecoder, Encoder as HpackEncoder},
    proto::{
        flow_control::FlowControl, ping_pong::PingPong, settings::ConnSettings,
        streams::StreamStore,
    },
};

/// Shared connection state type. All user handles and the IO driver hold a
/// clone of this.
///
/// Uses `Rc<RefCell<>>` — compio is single-threaded, no `Send`/`Sync` needed.
pub type SharedState = Rc<RefCell<ConnShared>>;

/// Extra connection parameters that are local policies, not conveyed via H2
/// SETTINGS frames.
#[derive(Default, Clone)]
pub struct ConnExtra {
    /// Maximum number of recently-reset streams tracked before triggering a
    /// GOAWAY with `ENHANCE_YOUR_CALM` (CVE-2023-44487 mitigation).
    pub max_concurrent_reset_streams: Option<usize>,
    /// Sliding window duration for counting reset streams.
    pub reset_stream_duration: Option<std::time::Duration>,
    /// Maximum total bytes queued in the pending-send buffer.
    pub max_send_buffer_size: Option<usize>,
}

/// A pending DATA send waiting for flow control capacity.
pub(crate) struct PendingSend {
    pub stream_id: StreamId,
    pub data: Bytes,
    pub end_stream: bool,
    /// Waker to wake when this pending send is flushed (item removed from
    /// queue).
    pub waker: Option<Waker>,
}

/// A pending send capacity reservation waiting for flow control window.
pub(crate) struct PendingCapacity {
    pub stream_id: StreamId,
    pub amount: u32,
    pub waker: Waker,
    /// Granted capacity (set when fulfilled).
    pub granted: Option<Result<u32, H2Error>>,
}

/// Incoming stream info delivered by the IO driver when a new request arrives
/// (server side).
pub struct IncomingStream {
    pub stream_id: StreamId,
    pub headers: Vec<DecodedHeader>,
}

/// All connection state, shared between user handles and the IO driver.
pub struct ConnShared {
    // --- Protocol state ---
    pub(crate) streams: StreamStore,
    pub(crate) conn_send_flow: FlowControl,
    pub(crate) conn_recv_flow: FlowControl,
    pub(crate) conn_recv_consumed: u32,
    pub(crate) settings: ConnSettings,
    pub(crate) hpack_encoder: HpackEncoder,
    pub(crate) hpack_decoder: HpackDecoder,
    pub(crate) ping_pong: PingPong,
    pub(crate) is_client: bool,
    pub(crate) last_peer_stream_id: StreamId,
    pub(crate) going_away: bool,
    pub(crate) pending_sends: Vec<PendingSend>,
    pub(crate) pending_send_bytes: usize,
    pub(crate) max_send_buffer_size: usize,
    pub(crate) pending_capacity: Vec<PendingCapacity>,

    // --- Write buffer: frames encoded by user ops, flushed by IO task ---
    pub(crate) write_buf: Vec<u8>,

    // --- Waker maps (compio-quic pattern) ---
    /// IO task waker — woken when write_buf has data to flush.
    pub(crate) poller: Option<Waker>,
    /// Per-stream send wakers — woken when flow control window opens.
    pub(crate) writable: HashMap<StreamId, Waker>,
    /// Per-stream recv wakers — woken when data/trailers/headers arrive.
    pub(crate) readable: HashMap<StreamId, Waker>,
    /// Server accept() waiters — woken when a new stream arrives.
    pub(crate) accept_waiters: VecDeque<Waker>,
    /// Client ready() waiters — woken when stream capacity is available.
    pub(crate) ready_waiters: VecDeque<Waker>,

    // --- Incoming streams queue (server side) ---
    pub(crate) incoming_streams: VecDeque<Result<IncomingStream, H2Error>>,

    // --- Connection error ---
    pub(crate) error: Option<H2Error>,
}

impl ConnShared {
    /// Create a new shared connection state.
    pub fn new(
        is_client: bool,
        settings: ConnSettings,
        ping_pong: PingPong,
        initial_connection_window_size: Option<u32>,
        extra: ConnExtra,
    ) -> Self {
        let mut streams = StreamStore::new(is_client);
        if !is_client {
            streams.set_max_concurrent_streams(settings.local().max_concurrent_streams);
        }
        if let Some(max) = extra.max_concurrent_reset_streams {
            streams.set_max_reset_streams(max);
        }
        if let Some(dur) = extra.reset_stream_duration {
            streams.set_reset_window(dur);
        }
        let conn_recv_flow = match initial_connection_window_size {
            Some(size) => FlowControl::new(size as i32),
            None => FlowControl::default(),
        };
        let max_header_list_size = settings.local().max_header_list_size as usize;
        let mut hpack_decoder = HpackDecoder::new(4096);
        hpack_decoder.set_max_header_list_size(max_header_list_size);
        let mut hpack_encoder = HpackEncoder::new(4096);
        if settings.local().max_header_list_size != u32::MAX {
            hpack_encoder.set_max_header_list_size(max_header_list_size);
        }

        ConnShared {
            streams,
            conn_send_flow: FlowControl::default(),
            conn_recv_flow,
            conn_recv_consumed: 0,
            settings,
            hpack_encoder,
            hpack_decoder,
            ping_pong,
            is_client,
            last_peer_stream_id: StreamId::ZERO,
            going_away: false,
            pending_sends: Vec::new(),
            pending_send_bytes: 0,
            max_send_buffer_size: extra.max_send_buffer_size.unwrap_or(409_600),
            pending_capacity: Vec::new(),
            write_buf: Vec::with_capacity(16_384),
            poller: None,
            writable: HashMap::new(),
            readable: HashMap::new(),
            accept_waiters: VecDeque::new(),
            ready_waiters: VecDeque::new(),
            incoming_streams: VecDeque::new(),
            error: None,
        }
    }

    /// Wake the IO task to flush the write buffer.
    pub(crate) fn wake_io(&mut self) {
        if let Some(waker) = self.poller.take() {
            waker.wake();
        }
    }

    /// Wake a stream's recv waker (data/trailers/headers arrived).
    pub(crate) fn wake_recv(&mut self, stream_id: &StreamId) {
        if let Some(waker) = self.readable.remove(stream_id) {
            waker.wake();
        }
    }

    /// Wake a stream's send waker (flow control window opened).
    pub(crate) fn wake_send(&mut self, stream_id: &StreamId) {
        if let Some(waker) = self.writable.remove(stream_id) {
            waker.wake();
        }
    }

    /// Wake all send wakers (e.g., after connection-level WINDOW_UPDATE).
    pub(crate) fn wake_all_senders(&mut self) {
        for (_, waker) in self.writable.drain() {
            waker.wake();
        }
    }

    /// Wake all recv wakers (e.g., on connection close).
    pub(crate) fn wake_all_receivers(&mut self) {
        for (_, waker) in self.readable.drain() {
            waker.wake();
        }
    }

    /// Terminate the connection with an error — wake all waiters.
    pub(crate) fn terminate(&mut self, error: H2Error) {
        self.error = Some(error);
        self.going_away = true;
        self.wake_all_senders();
        self.wake_all_receivers();
        for waker in self.accept_waiters.drain(..) {
            waker.wake();
        }
        for waker in self.ready_waiters.drain(..) {
            waker.wake();
        }
        // Wake pending sends
        for ps in &mut self.pending_sends {
            if let Some(waker) = ps.waker.take() {
                waker.wake();
            }
        }
        // Wake pending capacity
        for pc in &mut self.pending_capacity {
            pc.granted = Some(Err(H2Error::connection(Reason::RefusedStream)));
            pc.waker.wake_by_ref();
        }
        self.wake_io();
    }

    /// Check for connection error. Returns a clone of the stored error.
    pub(crate) fn check_error(&self) -> Result<(), H2Error> {
        match &self.error {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        }
    }

    // --- Frame encoding helpers (write directly to write_buf) ---

    /// HPACK-encode headers and write HEADERS frame (with CONTINUATION if
    /// needed) to the write buffer.
    pub(crate) fn encode_headers(
        &mut self,
        stream_id: StreamId,
        headers: &[(Bytes, Bytes)],
        end_stream: bool,
    ) -> Result<(), H2Error> {
        let mut header_block = Vec::new();
        self.hpack_encoder.encode(
            headers.iter().map(|(k, v)| (k.as_ref(), v.as_ref())),
            &mut header_block,
        );

        let mut frame = frame::Headers::new(stream_id, Bytes::from(header_block));
        if end_stream {
            frame.set_end_stream();
        }

        let max_frame_size = self.settings.remote().max_frame_size as usize;
        write_headers_with_continuation_to_buf(&mut self.write_buf, frame, max_frame_size);
        Ok(())
    }

    /// Write DATA frame(s) to the write buffer, consuming flow control.
    /// Returns Ok(true) if fully sent, Ok(false) if nothing could be sent
    /// (flow control blocked).
    pub(crate) fn encode_data(
        &mut self,
        stream_id: StreamId,
        data: &Bytes,
        end_stream: bool,
    ) -> Result<bool, H2Error> {
        let data_len = data.len() as u32;

        // Check flow control
        let conn_avail = self.conn_send_flow.available();
        let stream_avail = self
            .streams
            .get(&stream_id)
            .map(|s| s.send_flow.available())
            .unwrap_or(0);
        let sendable = std::cmp::min(data_len, std::cmp::min(conn_avail, stream_avail)) as usize;

        if !data.is_empty() && sendable == 0 {
            return Ok(false); // Flow control blocked
        }

        if sendable < data.len() && !data.is_empty() {
            return Ok(false); // Partial send not supported in direct path
        }

        // Consume flow control
        if data_len > 0 {
            self.conn_send_flow
                .consume(data_len)
                .map_err(|_| H2Error::connection(Reason::FlowControlError))?;
            if let Some(stream) = self.streams.get_mut(&stream_id) {
                stream
                    .send_flow
                    .consume(data_len)
                    .map_err(|_| H2Error::stream(stream_id.value(), Reason::FlowControlError))?;
            }
        }

        if end_stream && let Some(stream) = self.streams.get_mut(&stream_id) {
            stream.state = stream.state.send_end_stream()?;
        }

        // Write DATA frames to buffer
        let max_frame = self.settings.remote().max_frame_size as usize;
        if data.is_empty() {
            let mut flags = 0u8;
            if end_stream {
                flags |= 0x1;
            }
            let header = frame::FrameHeader::new(0x0, flags, stream_id, 0);
            self.write_buf.extend_from_slice(&header.encode());
        } else {
            let mut offset = 0;
            while offset < data.len() {
                let end = std::cmp::min(offset + max_frame, data.len());
                let chunk = &data[offset..end];
                let is_last = end == data.len();
                let mut flags = 0u8;
                if end_stream && is_last {
                    flags |= 0x1;
                }
                let header = frame::FrameHeader::new(0x0, flags, stream_id, chunk.len() as u32);
                self.write_buf.extend_from_slice(&header.encode());
                self.write_buf.extend_from_slice(chunk);
                offset = end;
            }
        }

        Ok(true)
    }

    /// Write RST_STREAM frame to the write buffer.
    pub(crate) fn encode_rst_stream(&mut self, stream_id: StreamId, reason: Reason) {
        let rst = frame::RstStream::new(stream_id, reason);
        Frame::RstStream(rst).encode(&mut self.write_buf);
    }

    /// Write GOAWAY frame to the write buffer.
    pub(crate) fn encode_goaway(&mut self, last_stream_id: StreamId, reason: Reason) {
        let goaway = frame::GoAway::new(last_stream_id, reason);
        Frame::GoAway(goaway).encode(&mut self.write_buf);
    }

    /// Write WINDOW_UPDATE frame to the write buffer.
    pub(crate) fn encode_window_update(&mut self, stream_id: StreamId, increment: u32) {
        let wu = frame::WindowUpdate::new(stream_id, increment);
        Frame::WindowUpdate(wu).encode(&mut self.write_buf);
    }

    /// Write SETTINGS frame to the write buffer.
    pub(crate) fn encode_settings(&mut self, settings: &frame::Settings) {
        Frame::Settings(settings.clone()).encode(&mut self.write_buf);
    }

    /// Write PING frame to the write buffer.
    pub(crate) fn encode_ping(&mut self, ping: &frame::Ping) {
        Frame::Ping(ping.clone()).encode(&mut self.write_buf);
    }

    // --- Stream data delivery (called by IO driver on incoming frames) ---

    /// Signal that a stream's recv side is done (END_STREAM or error).
    pub(crate) fn close_stream_recv(&mut self, stream_id: &StreamId) {
        if let Some(stream) = self.streams.get_mut(stream_id) {
            stream.recv_closed = true;
        }
        self.wake_recv(stream_id);
    }

    /// Signal a stream reset from the peer.
    pub(crate) fn deliver_reset(&mut self, stream_id: StreamId, reason: Reason) {
        if let Some(stream) = self.streams.get_mut(&stream_id) {
            stream.reset_reason = Some(reason);
            stream.recv_closed = true;
            stream.state = crate::proto::streams::StreamState::Closed;
            // Push error to data buffer so data() returns the error
            stream
                .data_buf
                .push_back(Err(H2Error::stream_remote(stream_id.value(), reason)));
        }
        self.wake_recv(&stream_id);
        self.wake_send(&stream_id);
    }

    /// Deliver an incoming stream (server side) and wake accept waiters.
    pub(crate) fn deliver_incoming_stream(&mut self, incoming: IncomingStream) {
        self.incoming_streams.push_back(Ok(incoming));
        if let Some(waker) = self.accept_waiters.pop_front() {
            waker.wake();
        }
    }

    /// Wake ready waiters when stream capacity becomes available.
    pub(crate) fn notify_ready_waiters(&mut self) {
        while self.streams.can_accept_stream() {
            if let Some(waker) = self.ready_waiters.pop_front() {
                waker.wake();
            } else {
                break;
            }
        }
    }

    /// Drain all ready waiters with connection closing.
    pub(crate) fn drain_ready_waiters(&mut self) {
        for waker in self.ready_waiters.drain(..) {
            waker.wake();
        }
        for waker in self.accept_waiters.drain(..) {
            waker.wake();
        }
    }

    /// Fulfill pending capacity reservations from available flow control.
    pub(crate) fn fulfill_pending_capacity(&mut self) {
        let mut still_pending = Vec::new();
        let pending = std::mem::take(&mut self.pending_capacity);

        for mut item in pending {
            let avail = if let Some(stream) = self.streams.get(&item.stream_id) {
                if !stream.state.can_send() {
                    item.granted = Some(Err(H2Error::Protocol(
                        "stream is not in a sendable state".into(),
                    )));
                    item.waker.wake_by_ref();
                    continue;
                }
                std::cmp::min(
                    self.conn_send_flow.available(),
                    stream.send_flow.available(),
                )
            } else {
                item.granted = Some(Err(H2Error::Protocol("stream not found".into())));
                item.waker.wake_by_ref();
                continue;
            };

            let grant = std::cmp::min(item.amount, avail);
            if grant > 0 {
                item.granted = Some(Ok(grant));
                item.waker.wake_by_ref();
            } else {
                still_pending.push(item);
            }
        }

        self.pending_capacity = still_pending;
    }

    /// Whether `stream_id` refers to an idle peer stream.
    pub(crate) fn is_idle_peer_stream(&self, stream_id: &StreamId) -> bool {
        if self.is_client {
            stream_id.value().is_multiple_of(2)
                && stream_id.value() > self.last_peer_stream_id.value()
        } else {
            stream_id.value() > self.last_peer_stream_id.value()
        }
    }
}

/// Create a new SharedState.
pub fn new_shared_state(
    is_client: bool,
    settings: ConnSettings,
    ping_pong: PingPong,
    initial_connection_window_size: Option<u32>,
    extra: ConnExtra,
) -> SharedState {
    Rc::new(RefCell::new(ConnShared::new(
        is_client,
        settings,
        ping_pong,
        initial_connection_window_size,
        extra,
    )))
}

// --- Helper functions ---

/// Write HEADERS frame with CONTINUATION splitting into a byte buffer.
fn write_headers_with_continuation_to_buf(
    buf: &mut Vec<u8>,
    headers: frame::Headers,
    max_frame_size: usize,
) {
    let header_block = headers.header_block().clone();

    if header_block.len() <= max_frame_size {
        headers.encode(buf);
    } else {
        let stream_id = headers.stream_id();

        // First chunk — HEADERS without END_HEADERS
        let first_chunk = header_block.slice(..max_frame_size);
        let mut first_frame = frame::Headers::new(stream_id, first_chunk);
        if headers.is_end_stream() {
            first_frame.set_end_stream();
        }
        if headers.has_priority() {
            first_frame.set_priority(headers.exclusive(), headers.dependency(), headers.weight());
        }
        first_frame.clear_end_headers();
        Frame::Headers(first_frame).encode(buf);

        // Remaining chunks as CONTINUATION
        let mut offset = max_frame_size;
        while offset < header_block.len() {
            let end = std::cmp::min(offset + max_frame_size, header_block.len());
            let chunk = header_block.slice(offset..end);
            let is_last = end == header_block.len();
            let mut cont = frame::Continuation::new(stream_id, chunk);
            if is_last {
                cont.set_end_headers();
            }
            Frame::Continuation(cont).encode(buf);
            offset = end;
        }
    }
}

/// Convert decoded headers to an http::HeaderMap (non-pseudo headers only).
pub(crate) fn headers_to_header_map(decoded: &[DecodedHeader]) -> http::HeaderMap {
    let mut map = http::HeaderMap::new();
    for dh in decoded {
        if !dh.name.starts_with(b":")
            && let (Ok(name), Ok(value)) = (
                http::header::HeaderName::from_bytes(&dh.name),
                http::header::HeaderValue::from_bytes(&dh.value),
            )
        {
            map.append(name, value);
        }
    }
    map
}

/// Check if decoded headers have no pseudo-headers (i.e., trailers).
pub(crate) fn has_no_pseudo_headers(decoded: &[DecodedHeader]) -> bool {
    !decoded.iter().any(|dh| dh.name.starts_with(b":"))
}

/// Parse content-length from decoded headers.
pub(crate) fn parse_content_length(headers: &[DecodedHeader]) -> Option<u64> {
    for dh in headers {
        if dh.name.eq_ignore_ascii_case(b"content-length")
            && let Ok(s) = std::str::from_utf8(&dh.value)
        {
            return s.parse().ok();
        }
    }
    None
}
