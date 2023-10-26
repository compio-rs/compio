use compio_http::Client;
use hyper::{body::HttpBody, Method};

#[compio_macros::main]
async fn main() {
    let client = Client::new();
    let response = client
        .request(Method::GET, "https://www.example.com/".parse().unwrap())
        .await
        .unwrap();
    let (parts, mut body) = response.into_parts();
    println!("{:?}", parts);
    println!(
        "{}",
        std::str::from_utf8(&body.data().await.unwrap().unwrap()).unwrap()
    );
}
