/// HTTP/2 client example.
///
/// Connects to an HTTP/2 server over plain TCP and sends a GET request.
/// Designed to pair with the `h2-server` example.
///
/// Usage: h2-client [PORT]
///        H2_PORT=9090 h2-client
///
/// Defaults to port 8080 if neither argument nor env var is provided.
use std::env;

use compio_h2::client;
use compio_net::TcpStream;

#[compio_macros::main]
async fn main() {
    let port: u16 = env::args()
        .nth(1)
        .or_else(|| env::var("H2_PORT").ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(8080);

    let addr = format!("127.0.0.1:{port}");
    eprintln!("connecting to {addr}");

    let stream = TcpStream::connect(addr).await.unwrap();
    stream.set_nodelay(true).unwrap();

    let (mut send_req, conn) = client::handshake(stream).await.unwrap();

    // Spawn connection driver
    compio_runtime::spawn(async move {
        if let Err(e) = conn.run().await {
            eprintln!("connection error: {e}");
        }
    })
    .detach();

    // Send GET /
    let request = http::Request::builder()
        .method(http::Method::GET)
        .uri("http://localhost/")
        .body(())
        .unwrap();

    let (resp_fut, _send_stream) = send_req.send_request(request, true).await.unwrap();
    let response = resp_fut.await_response().await.unwrap();

    eprintln!("response: {}", response.status());

    // Read response body
    let mut body = response.into_body();
    let mut data = Vec::new();
    while let Some(chunk) = body.data().await {
        let bytes = chunk.unwrap();
        let len = bytes.len();
        data.extend_from_slice(&bytes);
        let _ = body.flow_control().release_capacity(len);
    }

    println!("{}", String::from_utf8_lossy(&data));
}
