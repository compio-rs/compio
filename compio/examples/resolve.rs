use compio::net::ToSocketAddrsAsync;
use futures_util::{stream::FuturesUnordered, StreamExt};

#[compio::main]
async fn main() {
    let mut tasks = std::env::args()
        .skip(1)
        .map(|name| async move {
            (
                (name.as_str(), 0).to_socket_addrs_async().await.unwrap(),
                name,
            )
        })
        .collect::<FuturesUnordered<_>>();
    while let Some((addrs, name)) = tasks.next().await {
        println!("Address of {}", name);
        for addr in addrs {
            println!("    {}", addr.ip());
        }
    }
}
