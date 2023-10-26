use std::rc::Rc;

use hyper::{Body, Method, Uri};

use crate::{CompioExecutor, Connector, IntoUrl, Request, RequestBuilder, Response, Result};

/// An asynchronous `Client` to make Requests with.
#[derive(Debug, Clone)]
pub struct Client {
    client: Rc<ClientRef>,
}

impl Client {
    /// Create a client with default config.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            client: Rc::new(ClientRef {
                client: hyper::Client::builder()
                    .executor(CompioExecutor)
                    .set_host(true)
                    .build(Connector),
            }),
        }
    }

    /// Send a request and wait for a response.
    pub async fn execute(&self, request: Request) -> Result<Response> {
        let (method, url, headers, body, timeout, version) = request.pieces();
        let mut request = hyper::Request::builder()
            .method(method)
            .uri(
                url.as_str()
                    .parse::<Uri>()
                    .expect("a parsed Url should always be a valid Uri"),
            )
            .version(version)
            .body(body.unwrap_or_else(Body::empty))?;
        *request.headers_mut() = headers;
        let future = self.client.client.request(request);
        let res = if let Some(timeout) = timeout {
            compio_runtime::time::timeout(timeout, future)
                .await
                .map_err(|_| crate::Error::Timeout)??
        } else {
            future.await?
        };
        Ok(Response::new(res, url))
    }

    /// Send a request with method and url.
    pub fn request<U: IntoUrl>(&self, method: Method, url: U) -> RequestBuilder {
        RequestBuilder::new(
            self.clone(),
            url.into_url().map(|url| Request::new(method, url)),
        )
    }
}

#[derive(Debug)]
struct ClientRef {
    client: hyper::Client<Connector, Body>,
}
