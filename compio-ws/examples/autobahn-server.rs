#![allow(clippy::collapsible_match)]

use std::net::SocketAddr;

use compio_log::*;
use compio_net::{TcpListener, TcpStream};
use compio_ws::{WebSocketConfig, accept_async_with_config};
use tungstenite::{Error, Result};

async fn accept_connection(peer: SocketAddr, stream: TcpStream) {
    if let Err(e) = handle_connection(peer, stream).await {
        match e {
            Error::ConnectionClosed | Error::Protocol(_) | Error::Utf8(_) => (),
            _err => {
                error!("Error processing connection: {_err}");
            }
        }
    }
}

async fn handle_connection(_peer: SocketAddr, stream: TcpStream) -> Result<()> {
    let mut config = WebSocketConfig::default();
    config.max_message_size = Some(64 * 1024 * 1024);
    config.max_frame_size = Some(16 * 1024 * 1024);

    let mut ws_stream = accept_async_with_config(stream, Some(config))
        .await
        .expect("Failed to accept");

    info!("New WebSocket connection: {_peer}");

    loop {
        match ws_stream.read().await {
            Ok(msg) => {
                if msg.is_text() || msg.is_binary() {
                    ws_stream.send(msg).await?;
                }
            }
            Err(e) => match e {
                Error::ConnectionClosed => {
                    info!("Connection closed normally: {_peer}");
                    break;
                }
                _ => {
                    error!("Error: {e}");
                    return Err(e);
                }
            },
        }
    }

    Ok(())
}

#[compio_macros::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_max_level(Level::TRACE)
        .init();

    let addr = "127.0.0.1:9002";
    let listener = TcpListener::bind(addr).await.expect("Can't listen");
    info!("Listening on: {addr}");

    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                info!("Peer address: {addr}");
                compio_runtime::spawn(accept_connection(addr, stream)).detach();
            }
            Err(_e) => {
                error!("Error accepting connection: {_e}");
            }
        }
    }
}
