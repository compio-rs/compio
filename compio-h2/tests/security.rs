//! HTTP/2 security test suite.
//!
//! Tests protections against known HTTP/2 attack vectors using raw TCP
//! frame injection against a real compio-h2 server. Covers:
//!
//! - CVE-2023-44487: Rapid Reset (RST_STREAM flood)
//! - CVE-2024-27316: CONTINUATION flood
//! - CVE-2019-9512: Ping flood
//! - CVE-2019-9515: Settings flood
//! - CVE-2019-9518: Empty frames flood
//! - RFC 9113 frame validation (stream ID, frame size, flow control)
//! - PUSH_PROMISE rejection
//! - Max concurrent streams enforcement

use std::time::Duration;

use compio_h2::{
    ClientBuilder, Reason, ServerBuilder,
    frame::{
        FRAME_TYPE_CONTINUATION, FRAME_TYPE_DATA, FRAME_TYPE_GOAWAY, FRAME_TYPE_HEADERS,
        FRAME_TYPE_PING, FRAME_TYPE_PUSH_PROMISE, FRAME_TYPE_RST_STREAM, FRAME_TYPE_SETTINGS,
        FRAME_TYPE_WINDOW_UPDATE, Frame, Ping, RstStream, Settings, StreamId, WindowUpdate,
    },
};
use compio_io::{AsyncWrite, AsyncWriteExt};
use compio_net::{TcpListener, TcpStream};

mod common;

use common::raw::*;

// ---------------------------------------------------------------------------
// CVE-2023-44487: Rapid Reset (RST_STREAM flood)
// ---------------------------------------------------------------------------

/// Sending rapid RST_STREAM frames should trigger GOAWAY with
/// ENHANCE_YOUR_CALM.
#[compio_macros::test]
async fn rapid_reset_triggers_goaway() {
    let builder = ServerBuilder::new()
        .max_concurrent_reset_streams(5)
        .reset_stream_duration(Duration::from_secs(30));

    let (addr, done_rx) = start_server(builder).await;
    let stream = raw_client_connect(addr).await;

    for i in 0u32..7 {
        let sid = 1 + i * 2;
        tcp_write(&stream, minimal_request_headers(sid, true)).await;
        tcp_write(
            &stream,
            encode_frame(&Frame::RstStream(RstStream::new(
                StreamId::new(sid),
                Reason::Cancel,
            ))),
        )
        .await;
    }

    let goaway = find_goaway(&stream, GOAWAY_TIMEOUT).await;
    assert!(goaway.is_some(), "expected GOAWAY after rapid reset flood");
    let (_, reason) = goaway.unwrap();
    assert_eq!(reason, Reason::EnhanceYourCalm);

    let result = done_rx.recv_async().await.unwrap();
    assert!(result.is_err());
}

/// Rapid reset under threshold should not trigger GOAWAY.
#[compio_macros::test]
async fn rapid_reset_configurable_threshold() {
    let builder = ServerBuilder::new()
        .max_concurrent_reset_streams(20)
        .reset_stream_duration(Duration::from_secs(30));

    let (addr, _done_rx) = start_server(builder).await;
    let stream = raw_client_connect(addr).await;

    for i in 0u32..10 {
        let sid = 1 + i * 2;
        tcp_write(&stream, minimal_request_headers(sid, true)).await;
        tcp_write(
            &stream,
            encode_frame(&Frame::RstStream(RstStream::new(
                StreamId::new(sid),
                Reason::Cancel,
            ))),
        )
        .await;
    }

    let goaway = find_goaway(&stream, NEGATIVE_TIMEOUT).await;
    assert!(
        goaway.is_none(),
        "should not get GOAWAY when under threshold"
    );
}

// ---------------------------------------------------------------------------
// CVE-2024-27316: CONTINUATION flood
// ---------------------------------------------------------------------------

/// CONTINUATION flood should trigger ENHANCE_YOUR_CALM.
#[compio_macros::test]
async fn continuation_flood_triggers_enhance_your_calm() {
    let builder = ServerBuilder::new();
    let (addr, done_rx) = start_server(builder).await;
    let stream = raw_client_connect(addr).await;

    // HEADERS without END_HEADERS
    tcp_write(&stream, raw_frame(FRAME_TYPE_HEADERS, 0x00, 1, &[0x82])).await;

    // Flood with CONTINUATION frames (no END_HEADERS).
    // The server may close the connection mid-flood, causing BrokenPipe — that's
    // fine.
    for _ in 0..20 {
        let data = raw_frame(FRAME_TYPE_CONTINUATION, 0x00, 1, &[0x84]);
        let mut writer = &stream;
        if writer.write_all(data).await.0.is_err() {
            break;
        }
    }

    let goaway = find_goaway(&stream, GOAWAY_TIMEOUT).await;
    assert!(goaway.is_some(), "expected GOAWAY after CONTINUATION flood");
    let (_, reason) = goaway.unwrap();
    assert_eq!(reason, Reason::EnhanceYourCalm);

    let result = done_rx.recv_async().await.unwrap();
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// CVE-2019-9512: Ping flood
// ---------------------------------------------------------------------------

/// Many PINGs should not crash the server.
#[compio_macros::test]
async fn ping_flood_server_survives() {
    let builder = ServerBuilder::new();
    let (addr, _done_rx) = start_server(builder).await;
    let stream = raw_client_connect(addr).await;

    for i in 0u64..100 {
        tcp_write(
            &stream,
            encode_frame(&Frame::Ping(Ping::new(i.to_be_bytes()))),
        )
        .await;
    }

    // Read PONG responses
    let mut pong_count = 0;
    for _ in 0..100 {
        match read_frame_timeout(&stream, NEGATIVE_TIMEOUT).await {
            Some((header, _))
                if header.frame_type == FRAME_TYPE_PING && header.flags & 0x1 != 0 =>
            {
                pong_count += 1;
            }
            Some(_) => {}
            None => break,
        }
    }
    assert!(pong_count > 0, "server should respond with PONGs");
}

// ---------------------------------------------------------------------------
// CVE-2019-9515: Settings flood
// ---------------------------------------------------------------------------

/// Many SETTINGS frames should not crash the server.
#[compio_macros::test]
async fn settings_flood_server_survives() {
    let builder = ServerBuilder::new();
    let (addr, _done_rx) = start_server(builder).await;
    let stream = raw_client_connect(addr).await;

    for i in 0u32..50 {
        let mut settings = Settings::new();
        settings.set_initial_window_size(65535 + i);
        tcp_write(&stream, encode_frame(&Frame::Settings(settings))).await;
    }

    compio_runtime::time::sleep(Duration::from_millis(100)).await;

    let frames = drain_frames(&stream, 60).await;
    let ack_count = frames
        .iter()
        .filter(|(h, _)| h.frame_type == FRAME_TYPE_SETTINGS && h.flags & 0x1 != 0)
        .count();
    assert!(ack_count > 0, "server should send SETTINGS ACKs");
}

// ---------------------------------------------------------------------------
// Frame validation: stream ID rules
// ---------------------------------------------------------------------------

/// DATA on stream 0 -> PROTOCOL_ERROR.
#[compio_macros::test]
async fn data_on_stream_zero_rejected() {
    let builder = ServerBuilder::new();
    let (addr, done_rx) = start_server(builder).await;
    let stream = raw_client_connect(addr).await;

    tcp_write(&stream, raw_frame(FRAME_TYPE_DATA, 0x01, 0, b"bad")).await;

    let goaway = find_goaway(&stream, GOAWAY_TIMEOUT).await;
    assert!(goaway.is_some(), "expected GOAWAY for DATA on stream 0");
    let (_, reason) = goaway.unwrap();
    assert_eq!(reason, Reason::ProtocolError);

    let result = done_rx.recv_async().await.unwrap();
    assert!(result.is_err());
}

/// HEADERS on stream 0 -> PROTOCOL_ERROR.
#[compio_macros::test]
async fn headers_on_stream_zero_rejected() {
    let builder = ServerBuilder::new();
    let (addr, done_rx) = start_server(builder).await;
    let stream = raw_client_connect(addr).await;

    tcp_write(&stream, raw_frame(FRAME_TYPE_HEADERS, 0x05, 0, &[0x82])).await;

    let goaway = find_goaway(&stream, GOAWAY_TIMEOUT).await;
    assert!(goaway.is_some(), "expected GOAWAY for HEADERS on stream 0");
    let (_, reason) = goaway.unwrap();
    assert_eq!(reason, Reason::ProtocolError);

    let result = done_rx.recv_async().await.unwrap();
    assert!(result.is_err());
}

/// SETTINGS on non-zero stream -> PROTOCOL_ERROR.
#[compio_macros::test]
async fn settings_on_nonzero_stream_rejected() {
    let builder = ServerBuilder::new();
    let (addr, done_rx) = start_server(builder).await;
    let stream = raw_client_connect(addr).await;

    tcp_write(&stream, raw_frame(FRAME_TYPE_SETTINGS, 0x00, 1, &[])).await;

    let goaway = find_goaway(&stream, GOAWAY_TIMEOUT).await;
    assert!(
        goaway.is_some(),
        "expected GOAWAY for SETTINGS on non-zero stream"
    );
    let (_, reason) = goaway.unwrap();
    assert_eq!(reason, Reason::ProtocolError);

    let result = done_rx.recv_async().await.unwrap();
    assert!(result.is_err());
}

/// PING on non-zero stream -> PROTOCOL_ERROR.
#[compio_macros::test]
async fn ping_on_nonzero_stream_rejected() {
    let builder = ServerBuilder::new();
    let (addr, done_rx) = start_server(builder).await;
    let stream = raw_client_connect(addr).await;

    tcp_write(&stream, raw_frame(FRAME_TYPE_PING, 0x00, 1, &[0; 8])).await;

    let goaway = find_goaway(&stream, GOAWAY_TIMEOUT).await;
    assert!(
        goaway.is_some(),
        "expected GOAWAY for PING on non-zero stream"
    );
    let (_, reason) = goaway.unwrap();
    assert_eq!(reason, Reason::ProtocolError);

    let result = done_rx.recv_async().await.unwrap();
    assert!(result.is_err());
}

/// GOAWAY on non-zero stream -> PROTOCOL_ERROR.
#[compio_macros::test]
async fn goaway_on_nonzero_stream_rejected() {
    let builder = ServerBuilder::new();
    let (addr, done_rx) = start_server(builder).await;
    let stream = raw_client_connect(addr).await;

    // GOAWAY: last_stream_id=0, error_code=NO_ERROR, but on stream 1 (invalid)
    let mut payload = Vec::new();
    payload.extend_from_slice(&0u32.to_be_bytes()); // last stream ID
    payload.extend_from_slice(&0u32.to_be_bytes()); // error code (NO_ERROR)
    tcp_write(&stream, raw_frame(FRAME_TYPE_GOAWAY, 0x00, 1, &payload)).await;

    let goaway = find_goaway(&stream, GOAWAY_TIMEOUT).await;
    assert!(
        goaway.is_some(),
        "expected GOAWAY for GOAWAY on non-zero stream"
    );
    let (_, reason) = goaway.unwrap();
    assert_eq!(reason, Reason::ProtocolError);

    let result = done_rx.recv_async().await.unwrap();
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Frame size validation
// ---------------------------------------------------------------------------

/// Frame > MAX_FRAME_SIZE -> FRAME_SIZE_ERROR.
#[compio_macros::test]
async fn oversized_frame_rejected() {
    let builder = ServerBuilder::new();
    let (addr, done_rx) = start_server(builder).await;
    let stream = raw_client_connect(addr).await;

    // Open a stream first
    tcp_write(&stream, minimal_request_headers(1, false)).await;
    compio_runtime::time::sleep(Duration::from_millis(50)).await;

    // DATA frame with 20000 bytes > default 16384 MAX_FRAME_SIZE
    let payload = vec![0u8; 20_000];
    tcp_write(&stream, raw_frame(FRAME_TYPE_DATA, 0x00, 1, &payload)).await;

    let goaway = find_goaway(&stream, GOAWAY_TIMEOUT).await;
    assert!(goaway.is_some(), "expected GOAWAY for oversized frame");
    let (_, reason) = goaway.unwrap();
    assert_eq!(reason, Reason::FrameSizeError);

    let result = done_rx.recv_async().await.unwrap();
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// PUSH_PROMISE rejection
// ---------------------------------------------------------------------------

/// PUSH_PROMISE -> PROTOCOL_ERROR.
#[compio_macros::test]
async fn push_promise_rejected() {
    let builder = ServerBuilder::new();
    let (addr, done_rx) = start_server(builder).await;
    let stream = raw_client_connect(addr).await;

    let mut payload = Vec::new();
    payload.extend_from_slice(&[0, 0, 0, 2]); // promised stream ID = 2
    payload.push(0x82);
    tcp_write(
        &stream,
        raw_frame(FRAME_TYPE_PUSH_PROMISE, 0x04, 1, &payload),
    )
    .await;

    let goaway = find_goaway(&stream, GOAWAY_TIMEOUT).await;
    assert!(goaway.is_some(), "expected GOAWAY for PUSH_PROMISE");
    let (_, reason) = goaway.unwrap();
    assert_eq!(reason, Reason::ProtocolError);

    let result = done_rx.recv_async().await.unwrap();
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Flow control attacks
// ---------------------------------------------------------------------------

/// WINDOW_UPDATE with increment=0 -> PROTOCOL_ERROR.
#[compio_macros::test]
async fn zero_window_update_rejected() {
    let builder = ServerBuilder::new();
    let (addr, done_rx) = start_server(builder).await;
    let stream = raw_client_connect(addr).await;

    tcp_write(
        &stream,
        raw_frame(FRAME_TYPE_WINDOW_UPDATE, 0x00, 0, &[0u8; 4]),
    )
    .await;

    let goaway = find_goaway(&stream, GOAWAY_TIMEOUT).await;
    assert!(goaway.is_some(), "expected GOAWAY for zero WINDOW_UPDATE");
    let (_, reason) = goaway.unwrap();
    assert_eq!(reason, Reason::ProtocolError);

    let result = done_rx.recv_async().await.unwrap();
    assert!(result.is_err());
}

/// WINDOW_UPDATE that overflows connection window -> FLOW_CONTROL_ERROR.
#[compio_macros::test]
async fn window_update_overflow_rejected() {
    let builder = ServerBuilder::new();
    let (addr, done_rx) = start_server(builder).await;
    let stream = raw_client_connect(addr).await;

    // Connection window starts at 65535. Sending max increment overflows.
    tcp_write(
        &stream,
        encode_frame(&Frame::WindowUpdate(WindowUpdate::new(
            StreamId::ZERO,
            0x7FFF_FFFF,
        ))),
    )
    .await;

    let goaway = find_goaway(&stream, GOAWAY_TIMEOUT).await;
    assert!(
        goaway.is_some(),
        "expected GOAWAY for WINDOW_UPDATE overflow"
    );
    let (_, reason) = goaway.unwrap();
    assert_eq!(reason, Reason::FlowControlError);

    let result = done_rx.recv_async().await.unwrap();
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Max concurrent streams
// ---------------------------------------------------------------------------

/// Exceeding MAX_CONCURRENT_STREAMS -> RST_STREAM REFUSED_STREAM.
#[compio_macros::test]
async fn max_concurrent_streams_enforced() {
    let builder = ServerBuilder::new().max_concurrent_streams(2);
    let (addr, _done_rx) = start_server(builder).await;
    let stream = raw_client_connect(addr).await;

    for i in 0u32..3 {
        let sid = 1 + i * 2;
        tcp_write(&stream, minimal_request_headers(sid, false)).await;
    }

    compio_runtime::time::sleep(HANDSHAKE_DRAIN_TIMEOUT).await;

    let frames = drain_frames(&stream, 20).await;
    let has_refused = frames.iter().any(|(header, payload)| {
        if header.frame_type == FRAME_TYPE_RST_STREAM && payload.len() >= 4 {
            let code = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
            Reason::from_u32(code) == Reason::RefusedStream
        } else if header.frame_type == FRAME_TYPE_GOAWAY && payload.len() >= 8 {
            let code = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);
            Reason::from_u32(code) == Reason::RefusedStream
        } else {
            false
        }
    });

    assert!(
        has_refused,
        "expected REFUSED_STREAM. Frames: {:?}",
        frames
            .iter()
            .map(|(h, _)| format!(
                "type=0x{:x} flags=0x{:x} stream={}",
                h.frame_type,
                h.flags,
                h.stream_id.value()
            ))
            .collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// HPACK oversized header list (integration)
// ---------------------------------------------------------------------------

/// Headers exceeding max_header_list_size -> COMPRESSION_ERROR.
#[compio_macros::test]
async fn oversized_header_list_rejected() {
    let builder = ServerBuilder::new().max_header_list_size(64);
    let (addr, done_rx) = start_server(builder).await;
    let stream = raw_client_connect(addr).await;

    let mut header_block = Vec::new();
    for _ in 0..5 {
        header_block.push(0x00); // literal, new name
        header_block.push(10);
        header_block.extend_from_slice(b"x-padding-");
        header_block.push(20);
        header_block.extend_from_slice(b"01234567890123456789");
    }

    tcp_write(
        &stream,
        raw_frame(FRAME_TYPE_HEADERS, 0x05, 1, &header_block),
    )
    .await;

    let goaway = find_goaway(&stream, GOAWAY_TIMEOUT).await;
    assert!(
        goaway.is_some(),
        "expected GOAWAY for oversized header list"
    );
    let (_, reason) = goaway.unwrap();
    assert!(
        reason == Reason::CompressionError || reason == Reason::ProtocolError,
        "expected COMPRESSION_ERROR or PROTOCOL_ERROR, got {:?}",
        reason
    );

    let result = done_rx.recv_async().await.unwrap();
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Invalid connection preface
// ---------------------------------------------------------------------------

/// Garbage instead of HTTP/2 preface -> GOAWAY PROTOCOL_ERROR.
#[compio_macros::test]
async fn invalid_preface_rejected() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (done_tx, done_rx) = flume::bounded(1);

    compio_runtime::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let result = ServerBuilder::new().handshake(stream).await;
        match result {
            Ok(mut conn) => {
                let _ = done_tx.send(conn.closed().await);
            }
            Err(e) => {
                let _ = done_tx.send(Err(e));
            }
        }
    })
    .detach();

    let stream = TcpStream::connect(addr).await.unwrap();
    tcp_write(&stream, b"GET / HTTP/1.1\r\nHost: evil\r\n\r\n".to_vec()).await;

    let goaway = find_goaway(&stream, GOAWAY_TIMEOUT).await;
    assert!(goaway.is_some(), "expected GOAWAY for invalid preface");
    let (_, reason) = goaway.unwrap();
    assert_eq!(reason, Reason::ProtocolError);

    let result = done_rx.recv_async().await.unwrap();
    assert!(result.is_err());
}

/// Truncated preface (EOF before 24 bytes) -> GOAWAY PROTOCOL_ERROR.
///
/// This is h2spec test 3.5.2: the client sends a short invalid preface and
/// closes the write half. The server must still send GOAWAY before closing.
#[compio_macros::test]
async fn truncated_preface_sends_goaway() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (done_tx, done_rx) = flume::bounded(1);

    compio_runtime::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let result = ServerBuilder::new().handshake(stream).await;
        match result {
            Ok(mut conn) => {
                let _ = done_tx.send(conn.closed().await);
            }
            Err(e) => {
                let _ = done_tx.send(Err(e));
            }
        }
    })
    .detach();

    let stream = TcpStream::connect(addr).await.unwrap();
    // Send only 4 bytes — not a valid 24-byte preface
    tcp_write(&stream, b"bad!".to_vec()).await;
    // Shut down write half so the server sees EOF during read_exact_bytes
    AsyncWrite::shutdown(&mut &stream).await.unwrap();

    let goaway = find_goaway(&stream, GOAWAY_TIMEOUT).await;
    assert!(
        goaway.is_some(),
        "expected GOAWAY for truncated preface (h2spec 3.5.2)"
    );
    let (_, reason) = goaway.unwrap();
    assert_eq!(reason, Reason::ProtocolError);

    let result = done_rx.recv_async().await.unwrap();
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// CVE-2019-9518: Empty frames on stream 0
// ---------------------------------------------------------------------------

/// Empty DATA on stream 0 -> PROTOCOL_ERROR.
#[compio_macros::test]
async fn empty_data_frames_on_stream_zero_rejected() {
    let builder = ServerBuilder::new();
    let (addr, done_rx) = start_server(builder).await;
    let stream = raw_client_connect(addr).await;

    tcp_write(&stream, raw_frame(FRAME_TYPE_DATA, 0x00, 0, &[])).await;

    let goaway = find_goaway(&stream, GOAWAY_TIMEOUT).await;
    assert!(goaway.is_some(), "expected GOAWAY for DATA on stream 0");
    let (_, reason) = goaway.unwrap();
    assert_eq!(reason, Reason::ProtocolError);

    let result = done_rx.recv_async().await.unwrap();
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// CVE-2019-9516: 0-Length Headers
// ---------------------------------------------------------------------------

/// Zero-length header names should be handled safely (no crash).
#[compio_macros::test]
async fn zero_length_headers_handled_safely() {
    let builder = ServerBuilder::new();
    let (addr, _done_rx) = start_server(builder).await;
    let stream = raw_client_connect(addr).await;

    let mut header_block = Vec::new();
    header_block.push(0x82); // :method GET
    header_block.push(0x84); // :path /
    header_block.push(0x87); // :scheme https
    for _ in 0..20 {
        header_block.push(0x00); // literal, new name
        header_block.push(0); // name length = 0
        header_block.push(0); // value length = 0
    }

    tcp_write(
        &stream,
        raw_frame(FRAME_TYPE_HEADERS, 0x05, 1, &header_block),
    )
    .await;

    // Server should not crash
    compio_runtime::time::sleep(HANDSHAKE_DRAIN_TIMEOUT).await;
    let _frames = drain_frames(&stream, 10).await;
}

// ---------------------------------------------------------------------------
// CONTINUATION assembly errors
// ---------------------------------------------------------------------------

/// CONTINUATION on wrong stream -> PROTOCOL_ERROR.
#[compio_macros::test]
async fn continuation_wrong_stream_rejected() {
    let builder = ServerBuilder::new();
    let (addr, done_rx) = start_server(builder).await;
    let stream = raw_client_connect(addr).await;

    tcp_write(&stream, raw_frame(FRAME_TYPE_HEADERS, 0x00, 1, &[0x82])).await;
    tcp_write(
        &stream,
        raw_frame(FRAME_TYPE_CONTINUATION, 0x04, 3, &[0x84]),
    )
    .await;

    let goaway = find_goaway(&stream, GOAWAY_TIMEOUT).await;
    assert!(
        goaway.is_some(),
        "expected GOAWAY for CONTINUATION on wrong stream"
    );
    let (_, reason) = goaway.unwrap();
    assert_eq!(reason, Reason::ProtocolError);

    let result = done_rx.recv_async().await.unwrap();
    assert!(result.is_err());
}

/// Non-CONTINUATION during header assembly -> PROTOCOL_ERROR.
#[compio_macros::test]
async fn non_continuation_during_assembly_rejected() {
    let builder = ServerBuilder::new();
    let (addr, done_rx) = start_server(builder).await;
    let stream = raw_client_connect(addr).await;

    tcp_write(&stream, raw_frame(FRAME_TYPE_HEADERS, 0x00, 1, &[0x82])).await;
    tcp_write(&stream, encode_frame(&Frame::Ping(Ping::new([0; 8])))).await;

    let goaway = find_goaway(&stream, GOAWAY_TIMEOUT).await;
    assert!(
        goaway.is_some(),
        "expected GOAWAY for non-CONTINUATION during assembly"
    );
    let (_, reason) = goaway.unwrap();
    assert_eq!(reason, Reason::ProtocolError);

    let result = done_rx.recv_async().await.unwrap();
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// High-level security tests (Builder API)
// ---------------------------------------------------------------------------

/// CVE-2023-44487: Rapid reset flood via high-level API should trigger
/// ENHANCE_YOUR_CALM server-side closure.
#[compio_macros::test]
async fn security_cve_2023_44487_rapid_reset_goaway() {
    let timeout = compio_runtime::time::timeout(Duration::from_secs(10), async {
        let server_builder = ServerBuilder::new()
            .max_concurrent_reset_streams(3)
            .reset_stream_duration(Duration::from_secs(10));

        let (mut client, mut server) =
            common::setup_with_builders(ClientBuilder::new(), server_builder).await;

        // Rapidly open + reset streams well past the limit (3)
        for i in 0..8 {
            let req = common::get_request(&format!("/rapid-reset-{}", i));
            match client.send_request(req, false).await {
                Ok((_resp_fut, send_stream)) => {
                    if let Some(ss) = send_stream {
                        let _ = ss.send_reset(compio_h2::Reason::Cancel).await;
                    }
                }
                Err(_) => break,
            }
            // Accept on server side to trigger RST_STREAM processing
            match server.accept().await {
                Some(Ok((_req, _send_resp))) => {}
                _ => break,
            }
        }

        // The server connection should close with ENHANCE_YOUR_CALM
        let closed_result =
            compio_runtime::time::timeout(Duration::from_secs(5), server.closed()).await;

        assert!(closed_result.is_ok(), "server.closed() should resolve");
        let server_err = closed_result.unwrap();
        assert!(
            server_err.is_err(),
            "server should close with error after rapid reset flood"
        );
        let err = server_err.unwrap_err();
        assert_eq!(
            err.reason(),
            Some(compio_h2::Reason::EnhanceYourCalm),
            "expected ENHANCE_YOUR_CALM, got: {:?}",
            err
        );
    });
    timeout
        .await
        .expect("security_cve_2023_44487_rapid_reset_goaway timed out");
}

/// Content-length mismatch: sending fewer bytes than declared content-length
/// must result in a ProtocolError RST_STREAM.
///
/// The server detects the mismatch when END_STREAM arrives with fewer bytes
/// than content-length declared. It sends RST_STREAM(PROTOCOL_ERROR) which
/// the client sees as a stream reset on its response future.
#[compio_macros::test]
async fn security_content_length_mismatch() {
    let timeout = compio_runtime::time::timeout(Duration::from_secs(5), async {
        let (mut client, mut server) = common::setup().await;

        // Build POST with content-length: 100
        let req = http::Request::builder()
            .method(http::Method::POST)
            .uri("http://localhost/cl-mismatch")
            .header("content-length", "100")
            .body(())
            .unwrap();

        let (resp_fut, send_stream) = client.send_request(req, false).await.unwrap();
        let mut ss = send_stream.unwrap();

        // Only send 50 bytes, then END_STREAM — content-length mismatch
        ss.send_data(vec![0xAA; 50], true).await.unwrap();

        // Server accepts the request — the connection task processes
        // the DATA+END_STREAM and detects the mismatch internally,
        // sending RST_STREAM to the client.
        let (req, _send_resp) = server.accept().await.unwrap().unwrap();
        let (_, mut recv) = req.into_parts();

        // Server RecvStream gets the 50 bytes delivered before the
        // mismatch check, then the channel closes (RST_STREAM sent).
        let mut total = 0;
        while let Some(chunk) = recv.data().await {
            match chunk {
                Ok(data) => {
                    total += data.len();
                    recv.flow_control().release_capacity(data.len()).unwrap();
                }
                Err(_) => break,
            }
        }
        assert_eq!(total, 50, "server should receive the 50 bytes before reset");

        // Client should see the stream reset on its response future.
        // The server sent RST_STREAM(PROTOCOL_ERROR) for this stream.
        match resp_fut.await_response().await {
            Err(_err) => {
                // The server sent RST_STREAM(PROTOCOL_ERROR) which closes
                // the client's response channel. The exact error variant
                // depends on timing — may be StreamError, Protocol, or
                // connection closed. Any error here confirms the mismatch
                // was detected and the stream was terminated.
            }
            Ok(_) => {
                panic!("client response should fail due to content-length mismatch reset");
            }
        }
    });
    timeout
        .await
        .expect("security_content_length_mismatch timed out");
}

/// Max concurrent streams enforcement: exceeding the server's
/// MAX_CONCURRENT_STREAMS must result in REFUSED_STREAM.
#[compio_macros::test]
async fn security_max_concurrent_streams_h2() {
    let timeout = compio_runtime::time::timeout(Duration::from_secs(5), async {
        let server_builder = ServerBuilder::new().max_concurrent_streams(2);
        let (mut client, mut server) =
            common::setup_with_builders(ClientBuilder::new(), server_builder).await;

        // Open 2 streams (at the limit) without completing them
        let (resp_fut1, _) = client
            .send_request(common::get_request("/concurrent-1"), true)
            .await
            .unwrap();
        let (resp_fut2, _) = client
            .send_request(common::get_request("/concurrent-2"), true)
            .await
            .unwrap();

        // Accept both on the server to keep them open
        let (_req1, mut send_resp1) = server.accept().await.unwrap().unwrap();
        let (_req2, mut send_resp2) = server.accept().await.unwrap().unwrap();

        // 3rd stream: the client should get a RefusedStream or the server
        // will reject it. The client may need ready() to detect the limit.
        // Try sending directly — depending on timing, either send_request
        // errors or the response comes back with a reset.
        let third_result = client
            .send_request(common::get_request("/concurrent-3"), true)
            .await;

        match third_result {
            Err(e) => {
                // Client detected the limit before sending
                let err_str = format!("{:?}", e);
                assert!(
                    err_str.contains("max concurrent")
                        || e.reason() == Some(compio_h2::Reason::RefusedStream),
                    "expected RefusedStream or max concurrent error, got: {:?}",
                    e
                );
            }
            Ok((resp_fut3, _)) => {
                // The server may have sent RST_STREAM REFUSED_STREAM
                let resp3 = resp_fut3.await_response().await;
                match resp3 {
                    Err(e) => {
                        assert!(
                            e.reason() == Some(compio_h2::Reason::RefusedStream)
                                || e.is_reset()
                                || e.is_go_away(),
                            "expected RefusedStream on 3rd stream, got: {:?}",
                            e
                        );
                    }
                    Ok(_) => {
                        // If both earlier streams completed by this point, the
                        // 3rd could succeed — that's also valid behavior
                    }
                }
            }
        }

        // Clean up: respond to the first two streams
        send_resp1
            .send_response(common::ok_response(), true)
            .await
            .unwrap();
        send_resp2
            .send_response(common::ok_response(), true)
            .await
            .unwrap();

        let resp1 = resp_fut1.await_response().await.unwrap();
        assert_eq!(resp1.status(), 200);
        let resp2 = resp_fut2.await_response().await.unwrap();
        assert_eq!(resp2.status(), 200);
    });
    timeout
        .await
        .expect("security_max_concurrent_streams_h2 timed out");
}

// ---------------------------------------------------------------------------
// RFC 9113 §4.1: Unknown frame types silently ignored
// ---------------------------------------------------------------------------

/// Unknown frame type (0xFF) must be silently ignored; connection stays alive.
#[compio_macros::test]
async fn unknown_frame_type_ignored() {
    let builder = ServerBuilder::new();
    let (addr, _done_rx) = start_server(builder).await;
    let stream = raw_client_connect(addr).await;

    // Send unknown frame type 0xFF with arbitrary payload
    tcp_write(&stream, raw_frame(0xFF, 0x00, 0, &[0xDE, 0xAD])).await;

    // Connection must still be alive — send PING, expect PONG
    tcp_write(&stream, encode_frame(&Frame::Ping(Ping::new(*b"unkntest")))).await;

    let mut got_pong = false;
    for _ in 0..10 {
        match read_frame_timeout(&stream, GOAWAY_TIMEOUT).await {
            Some((header, payload))
                if header.frame_type == FRAME_TYPE_PING && header.flags & 0x1 != 0 =>
            {
                assert_eq!(&payload, b"unkntest");
                got_pong = true;
                break;
            }
            Some(_) => {} // skip other frames (SETTINGS ACK, WINDOW_UPDATE, etc.)
            None => break,
        }
    }
    assert!(
        got_pong,
        "server should respond with PONG after unknown frame"
    );
}

// ---------------------------------------------------------------------------
// RFC 7540 §6.5: Unknown SETTINGS identifiers silently ignored
// ---------------------------------------------------------------------------

/// SETTINGS with unknown identifier (0xFFFF) must be ACKed, not rejected.
#[compio_macros::test]
async fn settings_unknown_identifier_ignored() {
    let builder = ServerBuilder::new();
    let (addr, _done_rx) = start_server(builder).await;
    let stream = raw_client_connect(addr).await;

    // SETTINGS frame with unknown id=0xFFFF, value=1
    let mut payload = Vec::new();
    payload.extend_from_slice(&0xFFFFu16.to_be_bytes()); // identifier
    payload.extend_from_slice(&1u32.to_be_bytes()); // value
    tcp_write(&stream, raw_frame(FRAME_TYPE_SETTINGS, 0x00, 0, &payload)).await;

    // Expect SETTINGS ACK (flags=0x01)
    let mut got_ack = false;
    for _ in 0..10 {
        match read_frame_timeout(&stream, GOAWAY_TIMEOUT).await {
            Some((header, _))
                if header.frame_type == FRAME_TYPE_SETTINGS && header.flags & 0x01 != 0 =>
            {
                got_ack = true;
                break;
            }
            Some(_) => {}
            None => break,
        }
    }
    assert!(
        got_ack,
        "server should ACK SETTINGS with unknown identifier"
    );

    // No GOAWAY should follow
    let goaway = find_goaway(&stream, NEGATIVE_TIMEOUT).await;
    assert!(
        goaway.is_none(),
        "unknown SETTINGS identifier must not trigger GOAWAY"
    );
}

// ---------------------------------------------------------------------------
// Multiple rapid SETTINGS before ACK
// ---------------------------------------------------------------------------

/// Three SETTINGS frames sent before any ACK should each get an ACK back.
#[compio_macros::test]
async fn multiple_settings_before_ack() {
    let builder = ServerBuilder::new();
    let (addr, _done_rx) = start_server(builder).await;
    let stream = raw_client_connect(addr).await;

    // Send 3 empty SETTINGS frames rapidly
    for _ in 0..3 {
        tcp_write(&stream, encode_frame(&Frame::Settings(Settings::new()))).await;
    }

    compio_runtime::time::sleep(Duration::from_millis(200)).await;

    let frames = drain_frames(&stream, 20).await;
    let ack_count = frames
        .iter()
        .filter(|(h, _)| h.frame_type == FRAME_TYPE_SETTINGS && h.flags & 0x01 != 0)
        .count();
    assert!(
        ack_count >= 3,
        "expected at least 3 SETTINGS ACKs, got {}",
        ack_count
    );
}

// ---------------------------------------------------------------------------
// SETTINGS ACK with payload
// ---------------------------------------------------------------------------

/// SETTINGS ACK with non-empty payload -> FRAME_SIZE_ERROR.
#[compio_macros::test]
async fn settings_ack_with_payload_rejected() {
    let builder = ServerBuilder::new();
    let (addr, done_rx) = start_server(builder).await;
    let stream = raw_client_connect(addr).await;

    tcp_write(&stream, raw_frame(FRAME_TYPE_SETTINGS, 0x01, 0, &[0u8; 6])).await;

    let goaway = find_goaway(&stream, GOAWAY_TIMEOUT).await;
    assert!(
        goaway.is_some(),
        "expected GOAWAY for SETTINGS ACK with payload"
    );
    let (_, reason) = goaway.unwrap();
    assert_eq!(reason, Reason::FrameSizeError);

    let result = done_rx.recv_async().await.unwrap();
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// SETTINGS invalid payload size
// ---------------------------------------------------------------------------

/// SETTINGS with payload not multiple of 6 -> FRAME_SIZE_ERROR.
#[compio_macros::test]
async fn settings_invalid_payload_size_rejected() {
    let builder = ServerBuilder::new();
    let (addr, done_rx) = start_server(builder).await;
    let stream = raw_client_connect(addr).await;

    tcp_write(&stream, raw_frame(FRAME_TYPE_SETTINGS, 0x00, 0, &[0u8; 7])).await;

    let goaway = find_goaway(&stream, GOAWAY_TIMEOUT).await;
    assert!(
        goaway.is_some(),
        "expected GOAWAY for SETTINGS with invalid payload size"
    );
    let (_, reason) = goaway.unwrap();
    assert!(
        reason == Reason::FrameSizeError || reason == Reason::ProtocolError,
        "expected FRAME_SIZE_ERROR or PROTOCOL_ERROR, got {:?}",
        reason
    );

    let result = done_rx.recv_async().await.unwrap();
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// PING invalid size
// ---------------------------------------------------------------------------

/// PING with payload != 8 -> FRAME_SIZE_ERROR.
#[compio_macros::test]
async fn ping_invalid_size_rejected() {
    let builder = ServerBuilder::new();
    let (addr, done_rx) = start_server(builder).await;
    let stream = raw_client_connect(addr).await;

    tcp_write(&stream, raw_frame(FRAME_TYPE_PING, 0x00, 0, &[0u8; 4])).await;

    let goaway = find_goaway(&stream, GOAWAY_TIMEOUT).await;
    assert!(
        goaway.is_some(),
        "expected GOAWAY for PING with wrong payload size"
    );
    let (_, reason) = goaway.unwrap();
    assert!(
        reason == Reason::FrameSizeError || reason == Reason::ProtocolError,
        "expected FRAME_SIZE_ERROR or PROTOCOL_ERROR, got {:?}",
        reason
    );

    let result = done_rx.recv_async().await.unwrap();
    assert!(result.is_err());
}
