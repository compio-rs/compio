//! Shared test harness for compio-h2 integration tests.
#![allow(dead_code)]

use compio_h2::{
    ClientBuilder, RecvStream, SendStream, ServerBuilder, client::SendRequest,
    server::ServerConnection,
};
use compio_net::{TcpListener, TcpStream};

/// Spin up a TCP server + client pair with default settings.
/// Returns (SendRequest, ServerConnection) for inline driving.
pub async fn setup() -> (SendRequest, ServerConnection) {
    setup_with_builders(ClientBuilder::new(), ServerBuilder::new()).await
}

/// Spin up a TCP server + client pair with custom builders.
pub async fn setup_with_builders(
    client_builder: ClientBuilder,
    server_builder: ServerBuilder,
) -> (SendRequest, ServerConnection) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    // Use flume to send the ServerConnection back from the spawned task
    let (server_tx, server_rx) = flume::bounded(1);

    compio_runtime::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let conn = server_builder.handshake(stream).await.unwrap();
        let _ = server_tx.send_async(conn).await;
    })
    .detach();

    let stream = TcpStream::connect(addr).await.unwrap();
    let (send_request, connection) = client_builder.handshake(stream).await.unwrap();

    compio_runtime::spawn(async move {
        let _ = connection.run().await;
    })
    .detach();

    let server_conn = server_rx.recv_async().await.unwrap();
    (send_request, server_conn)
}

/// Drain all data chunks from a RecvStream into a Vec<u8>.
pub async fn recv_all(recv: &mut RecvStream) -> Vec<u8> {
    let mut buf = Vec::new();
    while let Some(chunk) = recv.data().await {
        let data = chunk.unwrap();
        let len = data.len();
        buf.extend_from_slice(&data);
        let _ = recv.flow_control().release_capacity(len);
    }
    buf
}

/// Send `data` on `stream` in `chunk_size`-byte pieces, ending with
/// END_STREAM.
pub async fn send_chunked(stream: &mut SendStream, data: &[u8], chunk_size: usize) {
    let mut offset = 0;
    while offset < data.len() {
        let end = std::cmp::min(offset + chunk_size, data.len());
        let is_last = end == data.len();
        stream
            .send_data(data[offset..end].to_vec(), is_last)
            .await
            .unwrap();
        offset = end;
    }
}

/// Build a GET request for the given path.
pub fn get_request(path: &str) -> http::Request<()> {
    http::Request::builder()
        .method(http::Method::GET)
        .uri(format!("http://localhost{}", path))
        .body(())
        .unwrap()
}

/// Build a POST request for the given path.
pub fn post_request(path: &str) -> http::Request<()> {
    http::Request::builder()
        .method(http::Method::POST)
        .uri(format!("http://localhost{}", path))
        .body(())
        .unwrap()
}

/// Build a 200 OK response.
pub fn ok_response() -> http::Response<()> {
    http::Response::builder().status(200).body(()).unwrap()
}

/// Raw TCP frame helpers for security/protocol-level tests.
pub mod raw {
    use std::time::Duration;

    use compio_buf::BufResult;
    use compio_h2::{
        Reason, ServerBuilder,
        frame::{
            FRAME_HEADER_SIZE, FRAME_TYPE_GOAWAY, FRAME_TYPE_HEADERS, Frame, FrameHeader, PREFACE,
            Settings, StreamId,
        },
    };
    use compio_io::{AsyncReadExt, AsyncWriteExt};
    use compio_net::{TcpListener, TcpStream};

    /// Default timeout for waiting for a GOAWAY frame in tests.
    pub const GOAWAY_TIMEOUT: Duration = Duration::from_secs(5);

    /// Short timeout for draining handshake frames.
    pub const HANDSHAKE_DRAIN_TIMEOUT: Duration = Duration::from_millis(200);

    /// Short timeout for negative tests (should NOT receive a frame).
    pub const NEGATIVE_TIMEOUT: Duration = Duration::from_millis(500);

    /// Helper that encodes a frame into bytes (9-byte header + payload).
    pub fn encode_frame(frame: &Frame) -> Vec<u8> {
        let mut buf = Vec::new();
        frame.encode(&mut buf);
        buf
    }

    /// Helper to encode raw bytes for a frame header + payload manually.
    pub fn raw_frame(frame_type: u8, flags: u8, stream_id: u32, payload: &[u8]) -> Vec<u8> {
        let header = FrameHeader::new(
            frame_type,
            flags,
            StreamId::new(stream_id),
            payload.len() as u32,
        );
        let mut buf = Vec::from(header.encode().as_slice());
        buf.extend_from_slice(payload);
        buf
    }

    /// Start a server with the given builder and return the listener address.
    pub async fn start_server(
        builder: ServerBuilder,
    ) -> (
        std::net::SocketAddr,
        flume::Receiver<Result<(), compio_h2::H2Error>>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (done_tx, done_rx) = flume::bounded(1);

        compio_runtime::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let result = builder.handshake(stream).await;
            match result {
                Ok(mut conn) => {
                    let close_result = conn.closed().await;
                    let _ = done_tx.send(close_result);
                }
                Err(e) => {
                    let _ = done_tx.send(Err(e));
                }
            }
        })
        .detach();

        (addr, done_rx)
    }

    /// Write bytes to a TcpStream (helper to avoid mutability issues).
    pub async fn tcp_write(stream: &TcpStream, data: Vec<u8>) {
        let mut writer = stream;
        writer.write_all(data).await.0.unwrap();
    }

    /// Read exactly `len` bytes from a TcpStream.
    pub async fn tcp_read_exact(stream: &TcpStream, len: usize) -> Option<Vec<u8>> {
        let buf = Vec::with_capacity(len);
        let mut reader = stream;
        let BufResult(result, buf) = reader.read_exact(buf).await;
        result.ok().map(|_| buf)
    }

    /// Connect as a raw TCP client and perform the H2 handshake.
    pub async fn raw_client_connect(addr: std::net::SocketAddr) -> TcpStream {
        let stream = TcpStream::connect(addr).await.unwrap();

        // Send client connection preface
        tcp_write(&stream, PREFACE.to_vec()).await;

        // Send empty client SETTINGS
        let settings = Settings::new();
        tcp_write(&stream, encode_frame(&Frame::Settings(settings))).await;

        // Read server's SETTINGS frame
        if let Some(header_buf) = tcp_read_exact(&stream, FRAME_HEADER_SIZE).await {
            let header = FrameHeader::decode(&header_buf[..9].try_into().unwrap());
            if header.length > 0 {
                let _ = tcp_read_exact(&stream, header.length as usize).await;
            }
        }

        // Send SETTINGS ACK
        tcp_write(&stream, encode_frame(&Frame::Settings(Settings::ack()))).await;

        // Drain a few handshake frames (SETTINGS ACK, WINDOW_UPDATE, etc.)
        drain_frames(&stream, 3).await;

        stream
    }

    /// Try to read up to `max_frames` frames from the stream.
    pub async fn drain_frames(
        stream: &TcpStream,
        max_frames: usize,
    ) -> Vec<(FrameHeader, Vec<u8>)> {
        let mut frames = Vec::new();
        for _ in 0..max_frames {
            match read_frame_timeout(stream, HANDSHAKE_DRAIN_TIMEOUT).await {
                Some((header, payload)) => frames.push((header, payload)),
                None => break,
            }
        }
        frames
    }

    /// Read a single frame with a timeout.
    pub async fn read_frame_timeout(
        stream: &TcpStream,
        timeout_dur: Duration,
    ) -> Option<(FrameHeader, Vec<u8>)> {
        let result = compio_runtime::time::timeout(timeout_dur, async {
            let header_buf = tcp_read_exact(stream, FRAME_HEADER_SIZE).await?;
            let header = FrameHeader::decode(&header_buf[..9].try_into().unwrap());
            let payload = if header.length > 0 {
                tcp_read_exact(stream, header.length as usize).await?
            } else {
                Vec::new()
            };
            Some((header, payload))
        })
        .await;

        match result {
            Ok(Some(frame)) => Some(frame),
            _ => None,
        }
    }

    /// Read frames until we find a GOAWAY, or timeout. Returns (last_stream_id,
    /// reason).
    pub async fn find_goaway(
        stream: &TcpStream,
        timeout_dur: Duration,
    ) -> Option<(StreamId, Reason)> {
        let deadline = std::time::Instant::now() + timeout_dur;
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return None;
            }
            match read_frame_timeout(stream, remaining).await {
                Some((header, payload)) => {
                    if header.frame_type == FRAME_TYPE_GOAWAY && payload.len() >= 8 {
                        let last_stream_id = u32::from_be_bytes([
                            payload[0] & 0x7F,
                            payload[1],
                            payload[2],
                            payload[3],
                        ]);
                        let error_code =
                            u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);
                        return Some((StreamId::new(last_stream_id), Reason::from_u32(error_code)));
                    }
                }
                None => return None,
            }
        }
    }

    /// Encode minimal HEADERS: `:method GET, :path /, :scheme https`.
    pub fn minimal_request_headers(stream_id: u32, end_stream: bool) -> Vec<u8> {
        let header_block = &[0x82, 0x84, 0x87];
        let mut flags = 0x04; // END_HEADERS
        if end_stream {
            flags |= 0x01; // END_STREAM
        }
        raw_frame(FRAME_TYPE_HEADERS, flags, stream_id, header_block)
    }
}
