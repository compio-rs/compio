//! Connection IO driver and frame handling.
//!
//! The IO driver reads frames from TCP, processes them into the shared state
//! (waking stream waiters), and flushes the write buffer to TCP. User
//! operations (send_data, send_request, etc.) encode frames directly into the
//! shared state's write buffer and wake the IO driver.

use std::future::poll_fn;
use std::task::Poll;

use bytes::Bytes;
use compio_buf::BufResult;
use compio_io::{AsyncRead, AsyncWrite, AsyncWriteExt};

use crate::{
    codec::FrameReader,
    error::{FrameError, H2Error, Reason},
    frame::{self, Frame, StreamId, DEFAULT_MAX_FRAME_SIZE},
    hpack::DecodedHeader,
    state::{
        ConnShared, IncomingStream, SharedState,
        has_no_pseudo_headers, headers_to_header_map, parse_content_length,
    },
};

/// Configure a [`FrameReader`] from a raw reader, applying local SETTINGS.
fn configure_reader<R: AsyncRead + 'static>(
    reader_io: R,
    settings: &crate::proto::settings::ConnSettings,
) -> FrameReader<R> {
    let mut reader = FrameReader::new(reader_io);
    if settings.local().max_header_list_size != u32::MAX {
        reader.set_max_header_list_size(settings.local().max_header_list_size);
    }
    if settings.local().max_frame_size != DEFAULT_MAX_FRAME_SIZE {
        reader.set_max_frame_size(settings.local().max_frame_size);
    }
    reader
}

/// Run the IO driver for a client connection.
///
/// Sends the connection preface, initial SETTINGS, and then enters the
/// IO driver loop.
pub(crate) async fn run_client_io<R: AsyncRead + 'static, W: AsyncWrite + 'static>(
    state: SharedState,
    reader_io: R,
    mut writer_io: W,
) -> Result<(), H2Error> {
    let reader = configure_reader(reader_io, &state.borrow().settings);

    // Client connection preface: send magic bytes before SETTINGS.
    let BufResult(result, _) = writer_io.write_all(frame::PREFACE.to_vec()).await;
    result.map_err(H2Error::from)?;

    // Send initial SETTINGS + optional WINDOW_UPDATE
    {
        let mut s = state.borrow_mut();
        let settings_frame = s.settings.build_initial_settings();
        s.encode_settings(&settings_frame);

        let initial_conn_window = s.conn_recv_flow.initial_window_size();
        if initial_conn_window > 65_535 {
            let increment = (initial_conn_window - 65_535) as u32;
            s.encode_window_update(StreamId::ZERO, increment);
        }
    }

    // Flush initial frames
    flush_write_buf(&state, &mut writer_io).await?;

    run_io_loop(state, reader, writer_io).await
}

/// Run the IO driver for a server connection.
///
/// Reads and validates the client preface, sends initial SETTINGS, and then
/// enters the IO driver loop.
pub(crate) async fn run_server_io<R: AsyncRead + 'static, W: AsyncWrite + 'static>(
    state: SharedState,
    reader_io: R,
    mut writer_io: W,
) -> Result<(), H2Error> {
    let mut reader = configure_reader(reader_io, &state.borrow().settings);

    // Server: read and validate the client connection preface.
    // If the read fails (EOF, short read) or the bytes don't match, send
    // GOAWAY(PROTOCOL_ERROR) before closing — required by RFC 9113 §3.4.
    let preface_ok = match reader.read_exact_bytes(frame::PREFACE.len()).await {
        Ok(buf) => buf == frame::PREFACE,
        Err(_) => false,
    };
    if !preface_ok {
        let mut s = state.borrow_mut();
        s.encode_goaway(StreamId::ZERO, Reason::ProtocolError);
        drop(s);
        let _ = flush_write_buf(&state, &mut writer_io).await;
        return Err(H2Error::Protocol("invalid client preface".into()));
    }

    // Send initial SETTINGS + optional WINDOW_UPDATE
    {
        let mut s = state.borrow_mut();
        let settings_frame = s.settings.build_initial_settings();
        s.encode_settings(&settings_frame);

        let initial_conn_window = s.conn_recv_flow.initial_window_size();
        if initial_conn_window > 65_535 {
            let increment = (initial_conn_window - 65_535) as u32;
            s.encode_window_update(StreamId::ZERO, increment);
        }
    }

    flush_write_buf(&state, &mut writer_io).await?;

    run_io_loop(state, reader, writer_io).await
}

/// Main IO driver loop.
///
/// Uses poll_fn to create a poller that wakes when user operations write to
/// the write buffer. Reads frames from TCP and flushes the write buffer.
async fn run_io_loop<R: AsyncRead + 'static, W: AsyncWrite>(
    state: SharedState,
    mut reader: FrameReader<R>,
    mut writer_io: W,
) -> Result<(), H2Error> {
    // Spawn a reader task that reads frames and processes them into state
    let state_for_reader = state.clone();
    let reader_task = compio_runtime::spawn(async move {

        loop {
            let result = reader.read_frame().await;
            let mut s = state_for_reader.borrow_mut();
            match result {
                Ok(Some(frame)) => {

                    if let Err(e) = handle_frame(&mut s, frame) {

                        s.terminate(e);
                        return;
                    }
                }
                Ok(None) => {

                    s.drain_ready_waiters();
                    return;
                }
                Err(e) => {

                    let conn_err = frame_read_error_to_connection_error(e);
                    s.terminate(conn_err);
                    return;
                }
            }
            // After handling frames, wake IO to flush any generated responses
            s.wake_io();
        }
    });

    // IO driver: wait for wake signals, then flush write buffer
    let result = io_flush_loop(&state, &mut writer_io).await;

    // Clean up
    drop(reader_task); // Cancel reader task
    {
        let mut s = state.borrow_mut();
        if let Err(ref e) = result {
            if let Some(reason) = e.reason() {
                // Always send GOAWAY on connection error, even if
                // terminate() was already called (which sets going_away).
                // The reader task may have called terminate() but didn't
                // encode GOAWAY — we do it here before flushing.
                let last_id = s.last_peer_stream_id;
                s.encode_goaway(last_id, reason);
            }
            s.terminate(e.clone());
        }
    }
    // Final flush — sends GOAWAY to peer
    let _ = flush_write_buf(&state, &mut writer_io).await;
    let _ = writer_io.shutdown().await;
    result
}

/// IO flush loop: waits for the poller waker, then flushes write_buf + does
/// housekeeping (WINDOW_UPDATEs, pending sends, capacity fulfillment).
async fn io_flush_loop<W: AsyncWrite>(
    state: &SharedState,
    writer_io: &mut W,
) -> Result<(), H2Error> {

    loop {
        // Wait for wake signal from user operations or reader task
        poll_fn(|cx| {
            let mut s = state.borrow_mut();

            // Check for errors
            if let Some(ref e) = s.error {

                return Poll::Ready(Err(e.clone()));
            }

            // Register our waker
            let ready = s.poller.is_none() || !s.write_buf.is_empty();
            s.poller = Some(cx.waker().clone());

            if ready {

                Poll::Ready(Ok(()))
            } else {
                Poll::Pending
            }
        })
        .await?;

        // Do housekeeping under the lock
        {
            let mut s = state.borrow_mut();

            // Check for errors
            if let Some(ref e) = s.error {

                return Err(e.clone());
            }

            // Flush pending sends that now fit in flow control windows
            flush_pending_sends(&mut s);

            // Fulfill pending capacity reservations
            s.fulfill_pending_capacity();

            // GC closed streams
            s.streams.gc_closed();

            // Wake ready waiters
            s.notify_ready_waiters();
        }

        // Encode WINDOW_UPDATEs after user data (from flush_pending_sends)
        // so they share the same write_buf → same TCP write when both
        // are present.
        {
            let mut s = state.borrow_mut();
            encode_window_updates(&mut s);
        }
        flush_write_buf(state, writer_io).await?;

        // Graceful shutdown check (after flush so GOAWAY reaches the peer)
        {
            let s = state.borrow();

            if s.going_away && s.streams.active_count() == 0 && s.error.is_none() {
                drop(s);

                state.borrow_mut().drain_ready_waiters();
                break;
            }
        }
    }
    Ok(())
}

/// Flush the shared write buffer to TCP.
async fn flush_write_buf<W: AsyncWrite>(
    state: &SharedState,
    writer_io: &mut W,
) -> Result<(), H2Error> {
    let buf = {
        let mut s = state.borrow_mut();
        if s.write_buf.is_empty() {
            return Ok(());
        }
        std::mem::replace(&mut s.write_buf, Vec::with_capacity(16_384))
    };
    let BufResult(result, _) = writer_io.write_all(buf).await;
    result.map_err(H2Error::from)
}

/// Encode pending WINDOW_UPDATEs into write_buf (connection + stream level).
fn encode_window_updates(s: &mut ConnShared) {
    // Connection-level WINDOW_UPDATE
    let threshold = (s.conn_recv_flow.initial_window_size()
        / crate::proto::streams::WINDOW_UPDATE_THRESHOLD_RATIO) as u32;
    if s.conn_recv_consumed > 0 && s.conn_recv_consumed >= threshold {
        let increment = s.conn_recv_consumed;
        s.encode_window_update(StreamId::ZERO, increment);
        let _ = s.conn_recv_flow.release(increment);
        s.conn_recv_consumed = 0;
    }

    // Stream-level WINDOW_UPDATEs
    let updates = s.streams.streams_needing_window_update();
    for (stream_id, increment) in updates {
        s.encode_window_update(stream_id, increment);
        s.streams.reset_released(&stream_id, increment);
    }
}

/// Try to flush pending DATA sends that now fit in flow control windows.
fn flush_pending_sends(s: &mut ConnShared) {
    let mut still_pending = Vec::new();
    let pending = std::mem::take(&mut s.pending_sends);

    for mut item in pending {
        let data_len = item.data.len() as u32;
        let conn_avail = s.conn_send_flow.available();
        let stream_avail = s
            .streams
            .get(&item.stream_id)
            .map(|st| st.send_flow.available())
            .unwrap_or(0);
        let sendable =
            std::cmp::min(data_len, std::cmp::min(conn_avail, stream_avail)) as usize;

        if item.data.is_empty() || sendable == item.data.len() {
            // Full send — encode and wake the sender
            s.pending_send_bytes -= item.data.len();
            let _result = encode_data_frames(s, item.stream_id, item.data, item.end_stream);
            if let Some(waker) = item.waker.take() {
                waker.wake();
            }
        } else if sendable > 0 {
            // Partial send
            let send_now = item.data.slice(..sendable);
            let remainder = item.data.slice(sendable..);
            s.pending_send_bytes -= sendable;
            let result = encode_data_frames(s, item.stream_id, send_now, false);
            if result.is_err() {
                s.pending_send_bytes -= remainder.len();
                if let Some(waker) = item.waker.take() {
                    waker.wake();
                }
            } else {
                item.data = remainder;
                still_pending.push(item);
            }
        } else if s.streams.get(&item.stream_id).is_none() && conn_avail > 0 {
            // Stream gone — wake the sender so it sees the error via check_error
            s.pending_send_bytes -= item.data.len();
            if let Some(waker) = item.waker.take() {
                waker.wake();
            }
        } else {
            still_pending.push(item);
        }
    }

    s.pending_sends = still_pending;
}

/// Encode DATA frames into the write buffer, consuming flow control.
/// Caller must ensure flow control capacity is available.
fn encode_data_frames(
    s: &mut ConnShared,
    stream_id: StreamId,
    data: Bytes,
    end_stream: bool,
) -> Result<(), H2Error> {
    // Reuse ConnShared::encode_data which handles flow control + frame encoding.
    // It returns Ok(false) if flow control blocked, but our caller pre-checks capacity.
    let sent = s.encode_data(stream_id, &data, end_stream)?;
    debug_assert!(sent || data.is_empty(), "caller should have checked flow control");
    Ok(())
}

// ---------------------------------------------------------------------------
// Frame handling (called by reader task with state locked)
// ---------------------------------------------------------------------------

/// Handle an incoming frame, updating shared state.
fn handle_frame(s: &mut ConnShared, frame: Frame) -> Result<(), H2Error> {
    let result = match frame {
        Frame::Data(data) => handle_data(s, data),
        Frame::Headers(headers) => handle_headers(s, headers),
        Frame::Priority(_) => Ok(()),
        Frame::RstStream(rst) => handle_rst_stream(s, rst),
        Frame::Settings(settings) => handle_settings(s, settings),
        Frame::Ping(ping) => handle_ping(s, ping),
        Frame::GoAway(goaway) => handle_goaway(s, goaway),
        Frame::WindowUpdate(wu) => handle_window_update(s, wu),
        Frame::Continuation(_) => Err(H2Error::connection(Reason::ProtocolError)),
    };

    match result {
        Ok(()) => Ok(()),
        Err(H2Error::StreamError {
            stream_id, reason, ..
        }) => {
            // Stream error: encode RST_STREAM and continue
            s.encode_rst_stream(StreamId::new(stream_id), reason);
            if let Some(stream) = s.streams.get_mut(&StreamId::new(stream_id)) {
                stream.state = stream.state.reset();
            }
            s.close_stream_recv(&StreamId::new(stream_id));
            Ok(())
        }
        Err(e) => {
            match &e {
                H2Error::HpackDecode(_) | H2Error::Hpack(_) => {
                    Err(H2Error::connection(Reason::CompressionError))
                }
                H2Error::Protocol(_) | H2Error::InvalidFrame(_) | H2Error::Frame(_) => {
                    Err(H2Error::connection(Reason::ProtocolError))
                }
                _ => Err(e),
            }
        }
    }
}

fn handle_data(s: &mut ConnShared, data: frame::Data) -> Result<(), H2Error> {
    let stream_id = data.stream_id();
    let payload_len = data.flow_controlled_len();
    let end_stream = data.is_end_stream();

    s.conn_recv_flow
        .consume(payload_len)
        .map_err(|_| H2Error::connection(Reason::FlowControlError))?;
    s.conn_recv_consumed += payload_len;

    if let Some(stream) = s.streams.get_mut(&stream_id) {
        if !stream.state.can_recv() {
            return Err(H2Error::stream(stream_id.value(), Reason::StreamClosed));
        }
        stream
            .recv_flow
            .consume(payload_len)
            .map_err(|_| H2Error::stream(stream_id.value(), Reason::FlowControlError))?;
        stream.received_data_bytes += payload_len as u64;
        if stream
            .expected_content_length
            .is_some_and(|expected| stream.received_data_bytes > expected)
        {
            return Err(H2Error::stream(stream_id.value(), Reason::ProtocolError));
        }

        stream.data_buf.push_back(Ok(data.into_payload()));

        if end_stream {
            if stream
                .expected_content_length
                .is_some_and(|expected| stream.received_data_bytes != expected)
            {
                return Err(H2Error::stream(stream_id.value(), Reason::ProtocolError));
            }
            stream.state = stream.state.recv_end_stream()?;
            stream.recv_closed = true;
        }
        s.wake_recv(&stream_id);
    } else {
        if s.is_idle_peer_stream(&stream_id) {
            return Err(H2Error::connection(Reason::ProtocolError));
        }
        s.encode_rst_stream(stream_id, Reason::StreamClosed);
    }

    Ok(())
}

fn handle_headers(s: &mut ConnShared, headers: frame::Headers) -> Result<(), H2Error> {
    let stream_id = headers.stream_id();
    let end_stream = headers.is_end_stream();

    let decoded = s.hpack_decoder.decode(headers.header_block())?;

    if !has_no_pseudo_headers(&decoded) {
        validate_pseudo_headers(&decoded, !s.is_client)?;
    }
    validate_regular_headers(&decoded, stream_id)?;

    if s.is_client {
        if let Some(stream) = s.streams.get_mut(&stream_id) {
            if !stream.state.can_recv() {
                return Err(H2Error::stream(stream_id.value(), Reason::StreamClosed));
            }
            if has_no_pseudo_headers(&decoded) && end_stream {
                let header_map = headers_to_header_map(&decoded);
                stream.trailers_buf = Some(Ok(header_map));
            } else if has_no_pseudo_headers(&decoded) && !end_stream {
                return Err(H2Error::stream(stream_id.value(), Reason::ProtocolError));
            } else {
                let mut status_code = http::StatusCode::OK;
                let mut header_map = http::HeaderMap::new();
                for dh in &decoded {
                    if &dh.name[..] == b":status" {
                        if let Ok(str) = std::str::from_utf8(&dh.value)
                            && let Ok(code) = str.parse::<u16>()
                        {
                            status_code = http::StatusCode::from_u16(code)
                                .unwrap_or(http::StatusCode::OK);
                        }
                    } else if !dh.name.starts_with(b":")
                        && let (Ok(hname), Ok(hvalue)) = (
                            http::header::HeaderName::from_bytes(&dh.name),
                            http::header::HeaderValue::from_bytes(&dh.value),
                        )
                    {
                        header_map.append(hname, hvalue);
                    }
                }
                stream.expected_content_length = parse_content_length(&decoded);
                stream.response_headers = Some(Ok((status_code, header_map)));
            }

            if end_stream {
                stream.state = stream.state.recv_end_stream()?;
                stream.recv_closed = true;
            }
            s.wake_recv(&stream_id);
        }
    } else {
        // Server side
        if !s.streams.contains(&stream_id) {
            if stream_id.value().is_multiple_of(2) {
                return Err(H2Error::connection(Reason::ProtocolError));
            }
            if stream_id.value() <= s.last_peer_stream_id.value() {
                return Err(H2Error::connection_msg(
                    Reason::ProtocolError,
                    "HEADERS on closed/non-monotonic stream ID",
                ));
            }
            if s.going_away {
                s.encode_rst_stream(stream_id, Reason::RefusedStream);
                return Ok(());
            }
            if !s.streams.can_accept_stream() {
                s.encode_rst_stream(stream_id, Reason::RefusedStream);
                return Ok(());
            }

            s.last_peer_stream_id = stream_id;
            let initial_send_window = s.settings.remote().initial_window_size as i32;
            let initial_recv_window = s.settings.local().initial_window_size as i32;
            s.streams
                .insert(stream_id, initial_send_window, initial_recv_window);

            if let Some(stream) = s.streams.get_mut(&stream_id) {
                stream.state = stream.state.recv_headers(end_stream)?;
                stream.expected_content_length = parse_content_length(&decoded);
                if end_stream {
                    stream.recv_closed = true;
                }
            }

            s.deliver_incoming_stream(IncomingStream {
                stream_id,
                headers: decoded,
            });
        } else {
            if let Some(stream) = s.streams.get_mut(&stream_id) {
                if !stream.state.can_recv() {
                    return Err(H2Error::stream(stream_id.value(), Reason::StreamClosed));
                }
                if !end_stream {
                    return Err(H2Error::stream(stream_id.value(), Reason::ProtocolError));
                }
                let header_map = headers_to_header_map(&decoded);
                stream.trailers_buf = Some(Ok(header_map));
                stream.state = stream.state.recv_end_stream()?;
                stream.recv_closed = true;
            }
            s.wake_recv(&stream_id);
        }
    }

    Ok(())
}

fn handle_rst_stream(s: &mut ConnShared, rst: frame::RstStream) -> Result<(), H2Error> {
    let stream_id = rst.stream_id();
    let reason = rst.reason();

    if !s.streams.contains(&stream_id) {
        if s.is_idle_peer_stream(&stream_id) {
            return Err(H2Error::connection(Reason::ProtocolError));
        }
        return Ok(());
    }

    s.deliver_reset(stream_id, reason);

    if s.streams.record_reset() {
        return Err(H2Error::connection(Reason::EnhanceYourCalm));
    }

    Ok(())
}

fn handle_settings(s: &mut ConnShared, settings: frame::Settings) -> Result<(), H2Error> {
    if settings.is_ack() {
        if let Some(queued_frame) = s.settings.recv_ack() {
            s.encode_settings(&queued_frame);
        }
        return Ok(());
    }

    s.settings.apply_remote(&settings);

    if s.is_client
        && let Some(max) = settings.max_concurrent_streams() {
            s.streams.set_max_concurrent_streams(max);
        }

    if let Some(new_window) = settings.initial_window_size() {
        let new_window = new_window as i32;
        let stream_ids: Vec<StreamId> = s.streams.iter_ids().collect();
        for id in stream_ids {
            if let Some(stream) = s.streams.get_mut(&id) {
                if stream.state.is_closed() {
                    continue;
                }
                stream
                    .send_flow
                    .update_initial_window_size(new_window)
                    .map_err(|_| H2Error::connection(Reason::FlowControlError))?;
            }
        }
        // Wake senders in case window opened up
        s.wake_all_senders();
    }

    let ack = frame::Settings::ack();
    s.encode_settings(&ack);

    Ok(())
}

fn handle_ping(s: &mut ConnShared, ping: frame::Ping) -> Result<(), H2Error> {
    if ping.is_ack() {
        s.ping_pong.recv_pong(ping.opaque_data());
        return Ok(());
    }
    let pong = frame::Ping::pong(*ping.opaque_data());
    s.encode_ping(&pong);
    Ok(())
}

fn handle_goaway(s: &mut ConnShared, goaway: frame::GoAway) -> Result<(), H2Error> {
    s.going_away = true;
    let last_stream_id = goaway.last_stream_id();
    let error_code = goaway.error_code();

    let stream_ids: Vec<StreamId> = s
        .streams
        .iter_ids()
        .filter(|id| id.value() > last_stream_id.value())
        .collect();

    for id in &stream_ids {
        s.deliver_reset(*id, error_code);
    }

    s.drain_ready_waiters();
    Ok(())
}

fn handle_window_update(s: &mut ConnShared, wu: frame::WindowUpdate) -> Result<(), H2Error> {
    let stream_id = wu.stream_id();
    let increment = wu.size_increment();

    if stream_id.is_zero() {
        s.conn_send_flow
            .apply_window_update(increment)
            .map_err(|_| H2Error::connection(Reason::FlowControlError))?;
        // Wake all senders — connection window opened
        s.wake_all_senders();
    } else if let Some(stream) = s.streams.get_mut(&stream_id) {
        stream
            .send_flow
            .apply_window_update(increment)
            .map_err(|_| H2Error::stream(stream_id.value(), Reason::FlowControlError))?;
        s.wake_send(&stream_id);
    } else if s.is_idle_peer_stream(&stream_id) {
        return Err(H2Error::connection(Reason::ProtocolError));
    }
    Ok(())
}

/// Convert a frame-read error into a connection-level error.
fn frame_read_error_to_connection_error(e: H2Error) -> H2Error {
    if e.reason().is_some() {
        return e;
    }
    match &e {
        H2Error::Frame(fe) => {
            let reason = match fe {
                FrameError::InvalidFrameSize(_) => Reason::FrameSizeError,
                FrameError::FlowControlError(_) => Reason::FlowControlError,
                _ => Reason::ProtocolError,
            };
            H2Error::connection(reason)
        }
        H2Error::HpackDecode(_) | H2Error::Hpack(_) => {
            H2Error::connection(Reason::CompressionError)
        }
        _ => H2Error::connection(Reason::ProtocolError),
    }
}

// ---------------------------------------------------------------------------
// Pseudo-header and regular header validation (unchanged from before)
// ---------------------------------------------------------------------------

/// Validate pseudo-header ordering and required fields per RFC 7540 §8.1.2.
fn validate_pseudo_headers(
    decoded: &[DecodedHeader],
    is_request: bool,
) -> Result<(), H2Error> {
    let mut seen_regular = false;
    let mut seen_pseudos = std::collections::HashSet::new();

    for dh in decoded {
        if dh.name.starts_with(b":") {
            if seen_regular {
                return Err(H2Error::Protocol(
                    "pseudo-header after regular header".into(),
                ));
            }
            if !seen_pseudos.insert(&dh.name[..]) {
                return Err(H2Error::Protocol(format!(
                    "duplicate pseudo-header: {}",
                    String::from_utf8_lossy(&dh.name)
                )));
            }
        } else {
            seen_regular = true;
        }
    }

    if is_request {
        let is_connect = seen_pseudos.contains(&b":method"[..])
            && decoded
                .iter()
                .any(|dh| &dh.name[..] == b":method" && &dh.value[..] == b"CONNECT");
        if !is_connect {
            if !seen_pseudos.contains(&b":method"[..]) {
                return Err(H2Error::Protocol("missing :method pseudo-header".into()));
            }
            if !seen_pseudos.contains(&b":scheme"[..]) {
                return Err(H2Error::Protocol("missing :scheme pseudo-header".into()));
            }
            if !seen_pseudos.contains(&b":path"[..]) {
                return Err(H2Error::Protocol("missing :path pseudo-header".into()));
            }
            // RFC 9113 §8.3.1: :path must not be empty for non-CONNECT requests
            if decoded.iter().any(|dh| &dh.name[..] == b":path" && dh.value.is_empty()) {
                return Err(H2Error::Protocol("empty :path pseudo-header".into()));
            }
        }
        // Request must not have :status
        if seen_pseudos.contains(&b":status"[..]) {
            return Err(H2Error::Protocol(
                ":status in request headers".into(),
            ));
        }
        // Response-only pseudos must not appear in requests
        for pseudo in &seen_pseudos {
            match *pseudo {
                b":method" | b":scheme" | b":path" | b":authority" | b":protocol" => {}
                b":status" => {
                    return Err(H2Error::Protocol(
                        ":status in request headers".into(),
                    ));
                }
                _ => {
                    return Err(H2Error::Protocol(format!(
                        "unknown pseudo-header: {}",
                        String::from_utf8_lossy(pseudo)
                    )));
                }
            }
        }
    } else {
        // Response headers
        if !seen_pseudos.contains(&b":status"[..]) {
            return Err(H2Error::Protocol("missing :status pseudo-header".into()));
        }
        // Response must not have request-only pseudos
        for pseudo in &seen_pseudos {
            match *pseudo {
                b":status" => {}
                b":method" | b":scheme" | b":path" | b":authority" | b":protocol" => {
                    return Err(H2Error::Protocol(format!(
                        "request pseudo-header in response: {}",
                        String::from_utf8_lossy(pseudo)
                    )));
                }
                _ => {
                    return Err(H2Error::Protocol(format!(
                        "unknown pseudo-header: {}",
                        String::from_utf8_lossy(pseudo)
                    )));
                }
            }
        }
    }

    Ok(())
}

/// Validate regular (non-pseudo) headers per RFC 7540 §8.1.2.
fn validate_regular_headers(
    decoded: &[DecodedHeader],
    stream_id: StreamId,
) -> Result<(), H2Error> {
    for dh in decoded {
        if dh.name.starts_with(b":") {
            continue;
        }
        // No uppercase in header names (RFC 7540 §8.1.2)
        if dh.name.iter().any(|b| b.is_ascii_uppercase()) {
            return Err(H2Error::stream(stream_id.value(), Reason::ProtocolError));
        }
        // Reject connection-specific headers (RFC 7540 §8.1.2.2)
        let name = &dh.name[..];
        if name == b"connection"
            || name == b"keep-alive"
            || name == b"proxy-connection"
            || name == b"transfer-encoding"
            || name == b"upgrade"
        {
            return Err(H2Error::stream(stream_id.value(), Reason::ProtocolError));
        }
        // TE header is allowed only with value "trailers"
        if name == b"te" && &dh.value[..] != b"trailers" {
            return Err(H2Error::stream(stream_id.value(), Reason::ProtocolError));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::flow_control::FlowControl;
    use crate::proto::streams::{StreamState, StreamStore};
    use crate::state::{ConnExtra, new_shared_state};

    fn test_state() -> SharedState {
        new_shared_state(
            false,
            crate::proto::settings::ConnSettings::new(),
            crate::proto::ping_pong::PingPong::disabled(),
            None,
            ConnExtra::default(),
        )
    }

    #[test]
    fn test_validate_pseudo_headers_valid_request() {
        let decoded = vec![
            DecodedHeader {
                name: Bytes::from_static(b":method"),
                value: Bytes::from_static(b"GET"),
                sensitive: false,
            },
            DecodedHeader {
                name: Bytes::from_static(b":scheme"),
                value: Bytes::from_static(b"https"),
                sensitive: false,
            },
            DecodedHeader {
                name: Bytes::from_static(b":path"),
                value: Bytes::from_static(b"/"),
                sensitive: false,
            },
        ];
        assert!(validate_pseudo_headers(&decoded, true).is_ok());
    }

    #[test]
    fn test_validate_pseudo_headers_missing_method() {
        let decoded = vec![
            DecodedHeader {
                name: Bytes::from_static(b":scheme"),
                value: Bytes::from_static(b"https"),
                sensitive: false,
            },
            DecodedHeader {
                name: Bytes::from_static(b":path"),
                value: Bytes::from_static(b"/"),
                sensitive: false,
            },
        ];
        assert!(validate_pseudo_headers(&decoded, true).is_err());
    }

    #[test]
    fn test_validate_regular_headers_uppercase_rejected() {
        let decoded = vec![DecodedHeader {
            name: Bytes::from_static(b"Content-Type"),
            value: Bytes::from_static(b"text/html"),
            sensitive: false,
        }];
        assert!(validate_regular_headers(&decoded, StreamId::new(1)).is_err());
    }

    #[test]
    fn test_validate_regular_headers_te_trailers_allowed() {
        let decoded = vec![DecodedHeader {
            name: Bytes::from_static(b"te"),
            value: Bytes::from_static(b"trailers"),
            sensitive: false,
        }];
        assert!(validate_regular_headers(&decoded, StreamId::new(1)).is_ok());
    }

    #[test]
    fn test_handle_window_update_connection() {
        let state = test_state();
        let mut s = state.borrow_mut();
        let wu = frame::WindowUpdate::new(StreamId::ZERO, 1000);
        assert!(handle_window_update(&mut s, wu).is_ok());
        assert_eq!(s.conn_send_flow.window_size(), 65535 + 1000);
    }

    #[test]
    fn test_settings_initial_window_adjusts_streams() {
        let state = test_state();
        let mut s = state.borrow_mut();
        s.streams.insert(StreamId::new(1), 65535, 65535);
        if let Some(st) = s.streams.get_mut(&StreamId::new(1)) {
            st.state = st.state.recv_headers(false).unwrap();
        }

        let mut settings = frame::Settings::new();
        settings.set_initial_window_size(32768);
        assert!(handle_settings(&mut s, settings).is_ok());

        assert_eq!(
            s.streams.get(&StreamId::new(1)).unwrap().send_flow.window_size(),
            32768
        );
    }
}
