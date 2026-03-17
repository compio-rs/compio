use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};

use bytes::Bytes;

use crate::{
    error::{H2Error, Reason},
    frame::StreamId,
    proto::flow_control::FlowControl,
};

/// WINDOW_UPDATE threshold divisor: emit a WINDOW_UPDATE when released bytes
/// reach `initial_window / WINDOW_UPDATE_THRESHOLD_RATIO` (i.e. 50% consumed).
/// Used in both `streams_needing_window_update` and `apply_release`.
pub(crate) const WINDOW_UPDATE_THRESHOLD_RATIO: i32 = 2;

/// Stream state machine per RFC 7540 Section 5.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamState {
    /// The stream has not yet been opened.
    Idle,
    /// The stream is fully open for both sending and receiving.
    Open,
    /// The local side has sent END_STREAM; only receiving is allowed.
    HalfClosedLocal,
    /// The remote side has sent END_STREAM; only sending is allowed.
    HalfClosedRemote,
    /// The stream is fully closed.
    Closed,
}

impl StreamState {
    /// Whether this state allows sending frames.
    pub fn can_send(self) -> bool {
        matches!(self, StreamState::Open | StreamState::HalfClosedRemote)
    }

    /// Whether this state allows receiving frames.
    pub fn can_recv(self) -> bool {
        matches!(self, StreamState::Open | StreamState::HalfClosedLocal)
    }

    /// Transition on sending headers.
    pub fn send_headers(self, end_stream: bool) -> Result<StreamState, H2Error> {
        match self {
            StreamState::Idle => {
                if end_stream {
                    Ok(StreamState::HalfClosedLocal)
                } else {
                    Ok(StreamState::Open)
                }
            }
            StreamState::Open | StreamState::HalfClosedRemote if end_stream => {
                if self == StreamState::HalfClosedRemote {
                    Ok(StreamState::Closed)
                } else {
                    Ok(StreamState::HalfClosedLocal)
                }
            }
            StreamState::Open | StreamState::HalfClosedRemote => Ok(self),
            _ => Err(H2Error::Protocol(format!(
                "cannot send headers in state {:?}",
                self
            ))),
        }
    }

    /// Transition on receiving headers.
    pub fn recv_headers(self, end_stream: bool) -> Result<StreamState, H2Error> {
        match self {
            StreamState::Idle => {
                if end_stream {
                    Ok(StreamState::HalfClosedRemote)
                } else {
                    Ok(StreamState::Open)
                }
            }
            StreamState::Open | StreamState::HalfClosedLocal if end_stream => {
                if self == StreamState::HalfClosedLocal {
                    Ok(StreamState::Closed)
                } else {
                    Ok(StreamState::HalfClosedRemote)
                }
            }
            StreamState::Open | StreamState::HalfClosedLocal => Ok(self),
            _ => Err(H2Error::Protocol(format!(
                "cannot receive headers in state {:?}",
                self
            ))),
        }
    }

    /// Transition on sending data with END_STREAM.
    pub fn send_end_stream(self) -> Result<StreamState, H2Error> {
        match self {
            StreamState::Open => Ok(StreamState::HalfClosedLocal),
            StreamState::HalfClosedRemote => Ok(StreamState::Closed),
            _ => Err(H2Error::Protocol(format!(
                "cannot send end_stream in state {:?}",
                self
            ))),
        }
    }

    /// Transition on receiving data with END_STREAM.
    pub fn recv_end_stream(self) -> Result<StreamState, H2Error> {
        match self {
            StreamState::Open => Ok(StreamState::HalfClosedRemote),
            StreamState::HalfClosedLocal => Ok(StreamState::Closed),
            _ => Err(H2Error::Protocol(format!(
                "cannot receive end_stream in state {:?}",
                self
            ))),
        }
    }

    /// Transition to the Closed state via RST_STREAM.
    pub fn reset(self) -> StreamState {
        StreamState::Closed
    }

    /// Whether the stream is in the Closed state.
    pub fn is_closed(self) -> bool {
        self == StreamState::Closed
    }
}

/// Per-stream state tracked by the connection.
pub struct Stream {
    /// Current stream state machine position.
    pub state: StreamState,
    /// Send-side flow control window for this stream.
    pub send_flow: FlowControl,
    /// Receive-side flow control window for this stream.
    pub recv_flow: FlowControl,
    /// Bytes the application has released via
    /// [`RecvFlowControl::release_capacity`](crate::RecvFlowControl::release_capacity).
    /// A WINDOW_UPDATE is sent for this amount, then it is reset to 0.
    pub released: u32,
    /// Buffered incoming DATA payloads (read by RecvStream::data()).
    pub data_buf: VecDeque<Result<Bytes, H2Error>>,
    /// Buffered incoming trailers (read by RecvStream::trailers()).
    pub trailers_buf: Option<Result<http::HeaderMap, H2Error>>,
    /// Buffered response headers (client-side, read by ResponseFuture).
    pub response_headers: Option<Result<(http::StatusCode, http::HeaderMap), H2Error>>,
    /// Reason code from a peer RST_STREAM (read by poll_reset()).
    pub reset_reason: Option<Reason>,
    /// Whether the recv side is closed (END_STREAM received or error).
    pub recv_closed: bool,
    /// Expected content length from the content-length header, if present.
    pub expected_content_length: Option<u64>,
    /// Number of DATA bytes received so far on this stream.
    pub received_data_bytes: u64,
}

/// Default maximum number of reset streams allowed within
/// [`DEFAULT_RESET_WINDOW`].
///
/// Limits rapid RST_STREAM floods (CVE-2023-44487).
const DEFAULT_MAX_RESET_STREAMS: usize = 50;

/// Default time window for counting reset streams.
///
/// Used with [`DEFAULT_MAX_RESET_STREAMS`] for DoS detection.
const DEFAULT_RESET_WINDOW: Duration = Duration::from_secs(1);

/// Manages all active streams for a connection.
pub struct StreamStore {
    streams: HashMap<StreamId, Stream>,
    next_local_id: StreamId,
    max_concurrent_streams: u32,
    /// Timestamps of recently received RST_STREAM frames for DoS detection
    /// (CVE-2023-44487).
    recently_reset: VecDeque<Instant>,
    /// Maximum number of reset streams allowed within `reset_window`.
    max_reset_streams: usize,
    /// Time window for counting reset streams.
    reset_window: Duration,
}

impl StreamStore {
    /// Create a new StreamStore.
    /// `is_client`: if true, local streams start at 1 (odd); otherwise 2
    /// (even).
    pub fn new(is_client: bool) -> Self {
        let next_local_id = if is_client {
            StreamId::new(1)
        } else {
            StreamId::new(2)
        };
        StreamStore {
            streams: HashMap::new(),
            next_local_id,
            max_concurrent_streams: u32::MAX,
            recently_reset: VecDeque::new(),
            max_reset_streams: DEFAULT_MAX_RESET_STREAMS,
            reset_window: DEFAULT_RESET_WINDOW,
        }
    }

    /// Set the maximum number of concurrent streams allowed.
    pub fn set_max_concurrent_streams(&mut self, max: u32) {
        self.max_concurrent_streams = max;
    }

    /// Set the maximum number of reset streams allowed within the reset window.
    pub fn set_max_reset_streams(&mut self, max: usize) {
        self.max_reset_streams = max;
    }

    /// Set the time window for counting reset streams.
    pub fn set_reset_window(&mut self, window: Duration) {
        self.reset_window = window;
    }

    /// Record a received RST_STREAM and check for rapid-reset DoS
    /// (CVE-2023-44487).
    ///
    /// Whether the number of resets within the configured window
    /// exceeds the limit, indicating the peer should be disconnected.
    pub fn record_reset(&mut self) -> bool {
        let now = Instant::now();
        // Evict entries older than the window
        while let Some(&front) = self.recently_reset.front() {
            if now.duration_since(front) > self.reset_window {
                self.recently_reset.pop_front();
            } else {
                break;
            }
        }
        self.recently_reset.push_back(now);
        self.recently_reset.len() > self.max_reset_streams
    }

    /// Allocate the next local stream ID.
    pub fn next_stream_id(&mut self) -> Result<StreamId, H2Error> {
        let id = self.next_local_id;
        self.next_local_id = id
            .next_id()
            .ok_or(H2Error::connection(Reason::ProtocolError))?;
        Ok(id)
    }

    /// Insert a new stream.
    pub fn insert(
        &mut self,
        stream_id: StreamId,
        initial_send_window: i32,
        initial_recv_window: i32,
    ) {
        let stream = Stream {
            state: StreamState::Idle,
            send_flow: FlowControl::new(initial_send_window),
            recv_flow: FlowControl::new(initial_recv_window),
            released: 0,
            data_buf: VecDeque::new(),
            trailers_buf: None,
            response_headers: None,
            reset_reason: None,
            recv_closed: false,
            expected_content_length: None,
            received_data_bytes: 0,
        };
        self.streams.insert(stream_id, stream);
    }

    /// Look up a stream by ID, returning a shared reference.
    pub fn get(&self, stream_id: &StreamId) -> Option<&Stream> {
        self.streams.get(stream_id)
    }

    /// Look up a stream by ID, returning a mutable reference.
    pub fn get_mut(&mut self, stream_id: &StreamId) -> Option<&mut Stream> {
        self.streams.get_mut(stream_id)
    }

    /// Whether the store contains the given stream ID.
    pub fn contains(&self, stream_id: &StreamId) -> bool {
        self.streams.contains_key(stream_id)
    }

    /// The number of non-closed streams.
    pub fn active_count(&self) -> usize {
        self.streams
            .values()
            .filter(|s| !s.state.is_closed())
            .count()
    }

    /// Check if the connection can accept a new stream without exceeding
    /// the max concurrent streams limit.
    pub fn can_accept_stream(&self) -> bool {
        (self.active_count() as u64) < (self.max_concurrent_streams as u64)
    }

    /// Iterate over all stream IDs.
    pub fn iter_ids(&self) -> impl Iterator<Item = StreamId> + '_ {
        self.streams.keys().copied()
    }

    /// Remove closed streams that have no unconsumed data.
    pub fn gc_closed(&mut self) {
        self.streams.retain(|_, s| {
            if !s.state.is_closed() {
                return true; // Keep open streams
            }
            // Keep closed streams that still have buffered data/trailers/headers
            !s.data_buf.is_empty() || s.trailers_buf.is_some() || s.response_headers.is_some()
        });
    }

    /// Collect stream IDs that need a WINDOW_UPDATE based on released bytes.
    ///
    /// Only includes streams where released bytes meet the threshold:
    /// `released >= initial_recv_window / threshold_ratio` (minimum 1 byte).
    /// This prevents eager per-chunk WINDOW_UPDATEs that cause write stalls.
    /// Use `threshold_ratio = 2` for 50%, matching connection-level behavior.
    pub fn streams_needing_window_update(&self) -> Vec<(StreamId, u32)> {
        let mut result = Vec::new();
        for (id, stream) in &self.streams {
            if stream.state.is_closed() {
                continue;
            }
            let threshold = (stream.recv_flow.initial_window_size() / WINDOW_UPDATE_THRESHOLD_RATIO)
                .max(1) as u32;
            if stream.released >= threshold {
                result.push((*id, stream.released));
            }
        }
        result
    }

    /// Apply a release to a stream (from the application's FlowControl handle).
    /// Returns `true` if the stream now has enough released bytes to warrant
    /// a WINDOW_UPDATE (crossed the threshold), so the caller can decide
    /// whether to wake the IO loop.
    pub fn apply_release(&mut self, stream_id: &StreamId, amount: u32) -> bool {
        if let Some(stream) = self.streams.get_mut(stream_id) {
            let was = stream.released;
            stream.released += amount;
            let threshold = (stream.recv_flow.initial_window_size() / WINDOW_UPDATE_THRESHOLD_RATIO)
                .max(1) as u32;
            was < threshold && stream.released >= threshold
        } else {
            false
        }
    }

    /// Reset released bytes for a stream after sending WINDOW_UPDATE.
    pub fn reset_released(&mut self, stream_id: &StreamId, increment: u32) {
        if let Some(stream) = self.streams.get_mut(stream_id) {
            let _ = stream.recv_flow.release(increment);
            stream.released = stream.released.saturating_sub(increment);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stream_state_transitions_client() {
        let state = StreamState::Idle;
        let state = state.send_headers(false).unwrap();
        assert_eq!(state, StreamState::Open);
        let state = state.send_end_stream().unwrap();
        assert_eq!(state, StreamState::HalfClosedLocal);
        let state = state.recv_end_stream().unwrap();
        assert_eq!(state, StreamState::Closed);
    }

    #[test]
    fn test_stream_state_transitions_server() {
        let state = StreamState::Idle;
        let state = state.recv_headers(false).unwrap();
        assert_eq!(state, StreamState::Open);
        let state = state.recv_end_stream().unwrap();
        assert_eq!(state, StreamState::HalfClosedRemote);
        let state = state.send_end_stream().unwrap();
        assert_eq!(state, StreamState::Closed);
    }

    #[test]
    fn test_stream_state_half_closed() {
        let state = StreamState::Idle;
        let state = state.send_headers(true).unwrap();
        assert_eq!(state, StreamState::HalfClosedLocal);
    }

    #[test]
    fn test_stream_state_invalid_transition() {
        let state = StreamState::Closed;
        assert!(state.send_headers(false).is_err());
        assert!(state.recv_headers(false).is_err());
    }

    #[test]
    fn test_stream_store_next_id_client() {
        let mut store = StreamStore::new(true);
        assert_eq!(store.next_stream_id().unwrap().value(), 1);
        assert_eq!(store.next_stream_id().unwrap().value(), 3);
        assert_eq!(store.next_stream_id().unwrap().value(), 5);
    }

    #[test]
    fn test_stream_store_next_id_server() {
        let mut store = StreamStore::new(false);
        assert_eq!(store.next_stream_id().unwrap().value(), 2);
        assert_eq!(store.next_stream_id().unwrap().value(), 4);
    }

    #[test]
    fn test_can_accept_stream_default_unlimited() {
        let store = StreamStore::new(true);
        // Default max is u32::MAX, so should always accept
        assert!(store.can_accept_stream());
    }

    #[test]
    fn test_can_accept_stream_with_limit() {
        let mut store = StreamStore::new(false);
        store.set_max_concurrent_streams(2);

        // No streams yet — can accept
        assert!(store.can_accept_stream());

        // Add two open streams
        store.insert(StreamId::new(1), 65535, 65535);
        if let Some(s) = store.get_mut(&StreamId::new(1)) {
            s.state = s.state.recv_headers(false).unwrap(); // Idle -> Open
        }
        assert!(store.can_accept_stream()); // 1 open, limit 2

        store.insert(StreamId::new(3), 65535, 65535);
        if let Some(s) = store.get_mut(&StreamId::new(3)) {
            s.state = s.state.recv_headers(false).unwrap();
        }
        assert!(!store.can_accept_stream()); // 2 open, limit 2

        // Close one stream
        if let Some(s) = store.get_mut(&StreamId::new(1)) {
            s.state = s.state.reset();
        }
        assert!(store.can_accept_stream()); // 1 open + 1 closed, limit 2
    }

    #[test]
    fn test_can_accept_stream_counts_half_closed_as_active() {
        let mut store = StreamStore::new(false);
        store.set_max_concurrent_streams(1);

        store.insert(StreamId::new(1), 65535, 65535);
        if let Some(s) = store.get_mut(&StreamId::new(1)) {
            s.state = s.state.recv_headers(true).unwrap(); // Idle -> HalfClosedRemote
        }
        // HalfClosedRemote is not closed, so counts as active
        assert!(!store.can_accept_stream());
    }

    #[test]
    fn test_gc_closed_removes_only_closed_streams() {
        let mut store = StreamStore::new(false);
        store.insert(StreamId::new(1), 65535, 65535);
        store.insert(StreamId::new(3), 65535, 65535);
        store.insert(StreamId::new(5), 65535, 65535);

        // Open stream 1, close stream 3, leave stream 5 idle
        if let Some(s) = store.get_mut(&StreamId::new(1)) {
            s.state = s.state.recv_headers(false).unwrap();
        }
        if let Some(s) = store.get_mut(&StreamId::new(3)) {
            s.state = s.state.reset();
        }

        assert_eq!(store.streams.len(), 3);
        store.gc_closed();
        assert_eq!(store.streams.len(), 2);
        assert!(store.contains(&StreamId::new(1)));
        assert!(!store.contains(&StreamId::new(3)));
        assert!(store.contains(&StreamId::new(5)));
    }

    #[test]
    fn test_gc_closed_empty_after_all_closed() {
        let mut store = StreamStore::new(true);
        store.insert(StreamId::new(1), 65535, 65535);
        store.insert(StreamId::new(3), 65535, 65535);

        for id in [1u32, 3] {
            if let Some(s) = store.get_mut(&StreamId::new(id)) {
                s.state = s.state.reset();
            }
        }

        store.gc_closed();
        assert_eq!(store.streams.len(), 0);
    }

    #[test]
    fn test_stream_ids_iteration() {
        let mut store = StreamStore::new(true);
        store.insert(StreamId::new(1), 65535, 65535);
        store.insert(StreamId::new(3), 65535, 65535);
        store.insert(StreamId::new(5), 65535, 65535);

        let mut ids: Vec<u32> = store.iter_ids().map(|id| id.value()).collect();
        ids.sort();
        assert_eq!(ids, vec![1, 3, 5]);
    }

    #[test]
    fn test_active_count() {
        let mut store = StreamStore::new(false);
        assert_eq!(store.active_count(), 0);

        store.insert(StreamId::new(1), 65535, 65535);
        if let Some(s) = store.get_mut(&StreamId::new(1)) {
            s.state = s.state.recv_headers(false).unwrap();
        }
        assert_eq!(store.active_count(), 1);

        store.insert(StreamId::new(3), 65535, 65535);
        // Stream 3 is Idle (not closed), counts as active
        assert_eq!(store.active_count(), 2);

        if let Some(s) = store.get_mut(&StreamId::new(1)) {
            s.state = s.state.reset();
        }
        assert_eq!(store.active_count(), 1);
    }

    #[test]
    fn test_can_send_can_recv() {
        assert!(!StreamState::Idle.can_send());
        assert!(!StreamState::Idle.can_recv());

        assert!(StreamState::Open.can_send());
        assert!(StreamState::Open.can_recv());

        assert!(!StreamState::HalfClosedLocal.can_send());
        assert!(StreamState::HalfClosedLocal.can_recv());

        assert!(StreamState::HalfClosedRemote.can_send());
        assert!(!StreamState::HalfClosedRemote.can_recv());

        assert!(!StreamState::Closed.can_send());
        assert!(!StreamState::Closed.can_recv());
    }

    #[test]
    fn test_record_reset_under_limit() {
        let mut store = StreamStore::new(false);
        store.set_max_reset_streams(5);
        // 5 resets should not exceed limit
        for _ in 0..5 {
            assert!(!store.record_reset());
        }
    }

    #[test]
    fn test_record_reset_exceeds_limit() {
        let mut store = StreamStore::new(false);
        store.set_max_reset_streams(3);
        assert!(!store.record_reset()); // 1
        assert!(!store.record_reset()); // 2
        assert!(!store.record_reset()); // 3
        assert!(store.record_reset()); // 4 > 3 — exceeded
    }

    #[test]
    fn test_record_reset_window_eviction() {
        let mut store = StreamStore::new(false);
        store.set_max_reset_streams(2);
        store.set_reset_window(Duration::from_millis(0)); // instant eviction

        // With zero window, old entries are immediately evicted
        assert!(!store.record_reset());
        assert!(!store.record_reset());
        // The third should still be fine because old entries are evicted
        assert!(!store.record_reset());
    }
}
