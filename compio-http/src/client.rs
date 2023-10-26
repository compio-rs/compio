use std::rc::Rc;

use hyper::{Body, Method, Response, Uri};
use url::Url;

use crate::{CompioExecutor, Connector, Request, Result};

/// An asynchronous `Client` to make Requests with.
#[derive(Debug, Clone)]
pub struct Client {
    client: Rc<ClientRef>,
}

impl Client {
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

    pub async fn execute(&self, request: Request) -> Result<Response<Body>> {
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
        Ok(res)
    }

    pub async fn request(&self, method: Method, url: Url) -> Result<Response<Body>> {
        let request = Request::new(method, url);
        self.execute(request).await
    }
}

#[derive(Debug)]
struct ClientRef {
    client: hyper::Client<Connector, Body>,
}
