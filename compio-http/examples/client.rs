use compio_http::Client;
use hyper::Method;

#[compio_macros::main]
async fn main() {
    let client = Client::new();
    let response = client
        .request(Method::GET, "https://www.example.com/".parse().unwrap())
        .await
        .unwrap();
    let (parts, body) = response.into_parts();
    println!("{:?}", parts);
    println!(
        "{}",
        std::str::from_utf8(&hyper::body::to_bytes(body).await.unwrap()).unwrap()
    );
}
