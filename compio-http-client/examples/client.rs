use compio_http_client::Client;

#[compio_macros::main]
async fn main() {
    let client = Client::new();
    let response = client.get("https://www.example.com/").send().await.unwrap();
    println!("{}", response.text().await.unwrap());
}
