use hyper::{Body, Method, Request, Response, Uri};

use crate::{CompioExecutor, Connector};

#[derive(Debug, Clone)]
pub struct Client {
    client: hyper::Client<Connector, Body>,
}

impl Client {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            client: hyper::Client::builder()
                .executor(CompioExecutor)
                .set_host(true)
                .build(Connector),
        }
    }

    pub async fn execute(&self, request: Request<Body>) -> hyper::Result<Response<Body>> {
        self.client.request(request).await
    }

    pub async fn request(&self, method: Method, uri: Uri) -> hyper::Result<Response<Body>> {
        let mut request = Request::new(Body::empty());
        *request.method_mut() = method;
        *request.uri_mut() = uri;
        self.execute(request).await
    }
}
