use std::rc::Rc;

use hyper::{Body, HeaderMap, Method, Uri};

use crate::{
    CompioExecutor, Connector, IntoUrl, Request, RequestBuilder, Response, Result, TlsBackend,
};

/// An asynchronous `Client` to make Requests with.
#[derive(Debug, Clone)]
pub struct Client {
    client: Rc<ClientRef>,
}

impl Client {
    /// Create a client with default config.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        ClientBuilder::new().build()
    }

    /// Creates a `ClientBuilder` to configure a `Client`.
    ///
    /// This is the same as `ClientBuilder::new()`.
    pub fn builder() -> ClientBuilder {
        ClientBuilder::new()
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
        *request.headers_mut() = self.client.headers.clone();
        crate::util::replace_headers(request.headers_mut(), headers);

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

    /// Convenience method to make a `GET` request to a URL.
    pub fn get<U: IntoUrl>(&self, url: U) -> RequestBuilder {
        self.request(Method::GET, url)
    }

    /// Convenience method to make a `POST` request to a URL.
    pub fn post<U: IntoUrl>(&self, url: U) -> RequestBuilder {
        self.request(Method::POST, url)
    }

    /// Convenience method to make a `PUT` request to a URL.
    pub fn put<U: IntoUrl>(&self, url: U) -> RequestBuilder {
        self.request(Method::PUT, url)
    }

    /// Convenience method to make a `PATCH` request to a URL.
    pub fn patch<U: IntoUrl>(&self, url: U) -> RequestBuilder {
        self.request(Method::PATCH, url)
    }

    /// Convenience method to make a `DELETE` request to a URL.
    pub fn delete<U: IntoUrl>(&self, url: U) -> RequestBuilder {
        self.request(Method::DELETE, url)
    }

    /// Convenience method to make a `HEAD` request to a URL.
    pub fn head<U: IntoUrl>(&self, url: U) -> RequestBuilder {
        self.request(Method::HEAD, url)
    }
}

#[derive(Debug)]
struct ClientRef {
    client: hyper::Client<Connector, Body>,
    headers: HeaderMap,
}

/// A `ClientBuilder` can be used to create a `Client` with custom
/// configuration.
#[derive(Debug)]
#[must_use]
pub struct ClientBuilder {
    headers: HeaderMap,
    tls: TlsBackend,
}

impl Default for ClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ClientBuilder {
    /// Constructs a new `ClientBuilder`.
    pub fn new() -> Self {
        Self {
            headers: HeaderMap::new(),
            tls: TlsBackend::default(),
        }
    }

    /// Returns a `Client` that uses this `ClientBuilder` configuration.
    pub fn build(self) -> Client {
        let client = hyper::Client::builder()
            .executor(CompioExecutor)
            .set_host(true)
            .build(Connector::new(self.tls));
        let client_ref = ClientRef {
            client,
            headers: self.headers,
        };
        Client {
            client: Rc::new(client_ref),
        }
    }

    /// Set the default headers for every request.
    pub fn default_headers(mut self, headers: HeaderMap) -> Self {
        for (key, value) in headers.iter() {
            self.headers.insert(key, value.clone());
        }
        self
    }

    /// Force using the native TLS backend.
    #[cfg(feature = "native-tls")]
    pub fn use_native_tls(mut self) -> Self {
        self.tls = TlsBackend::NativeTls;
        self
    }

    /// Force using the Rustls TLS backend.
    #[cfg(feature = "rustls")]
    pub fn use_rustls_tls(mut self) -> Self {
        self.tls = TlsBackend::Rustls;
        self
    }
}
