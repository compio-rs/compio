#![allow(clippy::collapsible_match)]

use compio_log::*;
use compio_net::TcpStream;
use compio_ws::client_async;
use tungstenite::{Error, Result};

const AGENT: &str = "Tungstenite";

async fn get_case_count() -> Result<u32> {
    let stream = TcpStream::connect("127.0.0.1:9001").await?;
    let (mut socket, _) = client_async("ws://localhost:9001/getCaseCount", stream).await?;
    let msg = socket.read().await?;
    socket.close(None).await?;
    Ok(msg
        .to_text()?
        .parse::<u32>()
        .expect("Can't parse case count"))
}

async fn update_reports() -> Result<()> {
    let stream = TcpStream::connect("127.0.0.1:9001").await?;
    let (mut socket, _) = client_async(
        &format!("ws://localhost:9001/updateReports?agent={AGENT}"),
        stream,
    )
    .await?;
    socket.close(None).await?;
    Ok(())
}

async fn run_test(case: u32) -> Result<()> {
    info!("Running test case {case}");
    let case_url = format!("ws://localhost:9001/runCase?case={case}&agent={AGENT}");
    let stream = TcpStream::connect("127.0.0.1:9001").await?;
    let (mut ws_stream, _) = client_async(&case_url, stream).await?;

    loop {
        match ws_stream.read().await {
            Ok(msg) => {
                if msg.is_text() || msg.is_binary() {
                    ws_stream.send(msg).await?;
                } else if msg.is_close() {
                    break;
                }
            }
            Err(e) => {
                error!("Error reading message: {e}");
                return Err(e);
            }
        }
    }

    Ok(())
}

#[compio_macros::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_max_level(Level::TRACE)
        .init();

    let total = get_case_count().await.expect("Error getting case count");

    for case in 1..=total {
        if let Err(e) = run_test(case).await {
            match e {
                Error::ConnectionClosed | Error::Protocol(_) | Error::Utf8(_) => (),
                _err => {
                    error!("Testcase failed: {_err}");
                }
            }
        }
    }

    update_reports().await.expect("Error updating reports");
}
