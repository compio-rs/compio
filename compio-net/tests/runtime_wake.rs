use std::io::Write;

use compio_io::AsyncRead;
use compio_net::TcpStream;

/// This is a regression test for a `compio_runtime::spawn_blocking` hang that
/// was observed in compio v0.19.1, compio-net v0.12.1 when a polling driver
/// ("compio-driver/polling") was in use.
#[compio_macros::test]
async fn runtime_tcp_stream_blocking_read() {
    // 1. Construct a client/server pair of `compio_net::TcpStream`s, the server
    //    stream via `::from_std`.
    //
    // NB. Don't explicitly set it non-blocking.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let connector = std::thread::spawn(move || std::net::TcpStream::connect(addr).unwrap());
    let mut client = connector.join().unwrap();
    let (server, _) = listener.accept().unwrap();
    let mut stream = TcpStream::from_std(server).unwrap();

    // 2. Write a message into the client pipe.
    client.write_all(b"PIPELINED-FIRST-FRAME").unwrap();
    client.flush().unwrap();

    // 3. Read the whole message out of the server's stream.
    let bytes_read = stream.read(Vec::with_capacity(64)).await.0.unwrap();
    assert_eq!(bytes_read, 21);

    // 4. Spawn a runtime thread to attempt a second read from the (now empty)
    //    server stream.
    let reader =
        compio_runtime::spawn(async move { stream.read(Vec::with_capacity(64)).await.0.unwrap() });

    // 5. Spawn a second, noop runtime thread -- it just sleeps and yields.
    //
    // This is where the hang occurs when it presents. The above `stream.read`
    // blocks on the empty stream, blocking the whole runtime thread.
    compio_runtime::spawn_blocking(|| {
        std::thread::sleep(std::time::Duration::from_millis(100));
    })
    .await
    .unwrap();

    drop(reader);
    drop(client);
}
