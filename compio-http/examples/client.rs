use compio_http::Client;
use http::{HeaderValue, Method, Request, Version};

#[compio_macros::main]
async fn main() {
    let client = Client::new();
    let mut request = Request::new(vec![]);
    *request.method_mut() = Method::GET;
    *request.uri_mut() = "https://www.example.com/".parse().unwrap();
    *request.version_mut() = Version::HTTP_11;
    let headers = request.headers_mut();
    headers.append("Host", HeaderValue::from_str("www.example.com").unwrap());
    headers.append("Connection", HeaderValue::from_str("close").unwrap());
    let response = client.execute(request).await.unwrap();
    let (parts, body) = response.into_parts();
    println!("{:?}", parts);
    println!("{}", String::from_utf8(body).unwrap());
}
