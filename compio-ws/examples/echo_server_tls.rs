use std::{fs, sync::Arc};

use compio_net::{TcpListener, TcpStream};
use compio_tls::TlsAcceptor;
use compio_ws::accept_async;
use rustls::ServerConfig;
use tungstenite::Message;

async fn create_tls_acceptor() -> Result<TlsAcceptor, Box<dyn std::error::Error>> {
    // Load certificate and key from files
    // Generate these files with:
    // openssl req -x509 -newkey rsa:2048 -keyout localhost.key -out localhost.crt
    // -days 365 -nodes -subj "/CN=localhost"

    let cert_file = fs::read_to_string("localhost.crt")?;
    let key_file = fs::read_to_string("localhost.key")?;

    // Parse certificate
    let cert_der = rustls_pemfile::certs(&mut cert_file.as_bytes())
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .next()
        .ok_or("No certificate found in localhost.crt")?;

    // Parse private key
    let key_der = rustls_pemfile::private_key(&mut key_file.as_bytes())?
        .ok_or("No private key found in localhost.key")?;

    let cert_chain = vec![cert_der];

    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, key_der)?;

    Ok(TlsAcceptor::from(Arc::new(config)))
}

#[compio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Check if certificate files exist
    if !std::path::Path::new("localhost.crt").exists()
        || !std::path::Path::new("localhost.key").exists()
    {
        eprintln!("Error: Certificate files not found!");
        eprintln!("Please generate them with:");
        eprintln!(
            "openssl req -x509 -newkey rsa:2048 -keyout localhost.key -out localhost.crt -days \
             365 -nodes -subj \"/CN=localhost\""
        );
        return Err("Missing certificate files".into());
    }

    // Create TLS acceptor
    let tls_acceptor = create_tls_acceptor().await?;

    let listener = TcpListener::bind("127.0.0.1:9002").await?;
    println!("WebSocket TLS echo server listening on wss://127.0.0.1:9002");

    loop {
        let (stream, addr) = listener.accept().await?;
        println!("New client connected: {addr}");

        let acceptor = tls_acceptor.clone();
        compio_runtime::spawn(async move {
            if let Err(e) = handle_client_tls(stream, acceptor).await {
                eprintln!("Error handling client {addr}: {e}");
            }
        })
        .detach();
    }
}

async fn handle_client_tls(
    stream: TcpStream,
    acceptor: TlsAcceptor,
) -> Result<(), Box<dyn std::error::Error>> {
    // Perform TLS handshake
    println!("Performing TLS handshake...");
    let tls_stream = acceptor.accept(stream).await?;
    println!("TLS handshake completed");

    // Perform WebSocket handshake
    println!("Performing WebSocket handshake...");
    let mut websocket = accept_async(tls_stream).await?;
    println!("WebSocket handshake successful");

    loop {
        match websocket.read().await? {
            Message::Text(text) => {
                println!("Received text: {text}");
                let echo_msg = format!("Echo: {text}");
                println!("Sending echo: {echo_msg}");

                websocket.send(Message::Text(echo_msg.into())).await?;
                println!("Echo sent successfully");
            }
            Message::Binary(data) => {
                println!("Received {} bytes of binary data", data.len());
                println!("Sending binary echo...");
                let mut echo_data = b"TLS Binary Echo: ".to_vec();
                echo_data.extend_from_slice(&data);
                websocket.send(Message::Binary(echo_data.into())).await?;
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

    println!("TLS client disconnected");
    Ok(())
}
