use compio_net::{TcpListener, TcpStream};
use compio_ws::{CompatWebSocketStream, WebSocketStream, accept_async, client_async};
use futures_util::{
    SinkExt, StreamExt,
    stream::{SplitSink, SplitStream},
};
use tungstenite::Message;

const N: usize = 16384;
const MSG_LEN: usize = 256;

#[compio_macros::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:8081").await?;
    println!("Listening");

    let c_h = compio_runtime::spawn(run_client());

    let (stream, _) = listener.accept().await?;
    let ws = accept_async(stream).await.unwrap();
    println!("Accepted");

    let (w, r) = ws.into_compat().split();

    let w_h = compio_runtime::spawn(server_send_task(w));
    let r_h = compio_runtime::spawn(server_recv_task(r));
    let _ = futures_util::join!(r_h, w_h, c_h);

    Ok(())
}

async fn run_client() -> WebSocketStream<TcpStream> {
    let stream = TcpStream::connect("127.0.0.1:8081").await.unwrap();
    let (mut ws, _) = client_async("ws://127.0.0.1:8081", stream).await.unwrap();
    println!("Connected");
    let data = vec![0; MSG_LEN];
    for _ in 0..N {
        ws.send(Message::Binary(data.clone().into())).await.unwrap();
    }
    println!("Client sent all messages");

    let mut n = 0;
    loop {
        ws.read().await.unwrap();
        n += 1;
        if n >= N {
            println!("Client read all messages");
            break;
        }
    }
    ws
}

async fn server_recv_task(
    mut ws: SplitStream<CompatWebSocketStream<TcpStream>>,
) -> SplitStream<CompatWebSocketStream<TcpStream>> {
    let mut n = 0;
    loop {
        ws.next().await.unwrap().unwrap();
        n += 1;
        if n >= N {
            println!("Server read all messages");
            break;
        }
    }
    ws
}

async fn server_send_task(
    mut ws: SplitSink<CompatWebSocketStream<TcpStream>, Message>,
) -> SplitSink<CompatWebSocketStream<TcpStream>, Message> {
    let data = vec![0; MSG_LEN];
    for _ in 0..N {
        ws.send(Message::Binary(data.clone().into())).await.unwrap();
    }
    println!("Server sent all messages");
    ws
}
