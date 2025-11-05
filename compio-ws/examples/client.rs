use compio_net::TcpStream;
use compio_ws::client_async;
use tungstenite::Message;

#[compio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Connecting to WebSocket server");

    let stream = TcpStream::connect("127.0.0.1:9002").await?;
    let (mut websocket, _response) = client_async("ws://127.0.0.1:9002", stream).await?;

    println!("Connected to WebSocket server");

    println!("Sending text message");
    websocket
        .send(Message::Text("Hello, server!".into()))
        .await?;

    println!("Sending binary message");
    websocket
        .send(Message::Binary(vec![1, 2, 3, 4, 5].into()))
        .await?;

    println!("Sending ping");
    websocket.send(Message::Ping(vec![42].into())).await?;

    println!("Reading responses");
    for i in 0..3 {
        match websocket.read().await? {
            Message::Text(text) => println!("  Response {}: Text: {}", i + 1, text),
            Message::Binary(data) => println!("  Response {}: Binary: {} bytes", i + 1, data.len()),
            Message::Pong(data) => println!("  Response {}: Pong: {:?}", i + 1, data),
            other => println!("  Response {}: {:?}", i + 1, other),
        }
    }

    println!("Closing connection");
    websocket.close(None).await?;
    println!("Connection closed successfully");

    Ok(())
}
