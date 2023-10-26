use compio_http::Client;
use hyper::Method;

#[compio_macros::main]
async fn main() {
    let client = Client::new();
    let response = client
        .request(Method::GET, "https://www.example.com/")
        .send()
        .await
        .unwrap();
    println!("{}", response.text().await.unwrap());
}
