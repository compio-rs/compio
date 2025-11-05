use compio_net::{TcpListener, TcpStream};
use compio_ws::accept_async;
use tungstenite::Message;

#[compio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:9002").await?;
    println!("WebSocket echo server listening on ws://127.0.0.1:9002");

    loop {
        let (stream, addr) = listener.accept().await?;
        println!("New client connected: {addr}");

        compio_runtime::spawn(async move {
            if let Err(e) = handle_client(stream).await {
                eprintln!("Error handling client {addr}: {e}");
            }
        })
        .detach();
    }
}

async fn handle_client(stream: TcpStream) -> Result<(), Box<dyn std::error::Error>> {
    let mut websocket = accept_async(stream).await?;
    println!("Handshake successful");

    loop {
        match websocket.read().await? {
            Message::Text(text) => {
                println!("Received text: {}", text.len());
                let echo_msg = format!("Echo: {text}");
                println!("Sending echo: {echo_msg}");

                websocket.send(Message::Text(text)).await?;
                println!("Echo sent successfully");
            }
            Message::Binary(data) => {
                println!("Received {} bytes of binary data", data.len());
                println!("Sending binary echo...");
                websocket.send(Message::Binary(data)).await?;
                println!("Binary echo sent successfully");
            }
            Message::Ping(data) => {
                println!("Received ping, sending pong");
                websocket.send(Message::Pong(data)).await?;
                println!("Pong sent successfully");
            }
            Message::Pong(_) => {
                println!("Received pong");
            }
            Message::Close(frame) => {
                println!("Received close frame: {frame:?}");
                break;
            }
            Message::Frame(_) => {
                println!("Received raw frame");
            }
        }
    }

    println!("Client disconnected");
    Ok(())
}
