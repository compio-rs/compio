use std::{rc::Rc, time::Duration};

use compio_buf::bytes::Bytes;
use compio_http::{HttpStream, TlsBackend};
use http::{header::HOST, HeaderValue};
use http_body_util::Empty;
use hyper::{body::Body, client::conn::http1::handshake, HeaderMap, Method, Uri};
use url::Url;

use crate::{Error, IntoUrl, Request, RequestBuilder, Response, Result};

/// An asynchronous `Client` to make Requests with.
#[derive(Debug, Clone)]
pub struct Client {
    client: Rc<ClientInner>,
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
    pub async fn execute<B: Body + 'static>(&self, request: Request<B>) -> Result<Response>
    where
        B::Data: Send,
        B::Error: std::error::Error + Send + Sync,
    {
        let (method, url, headers, body, timeout, version) = request.pieces();
        let request = hyper::Request::builder()
            .method(method)
            .uri(
                url.as_str()
                    .parse::<Uri>()
                    .expect("a parsed Url should always be a valid Uri"),
            )
            .version(version);
        if let Some(body) = body {
            let request = request.body(body)?;
            self.execute_impl(url, headers, timeout, request).await
        } else {
            let request = request.body(Empty::<Bytes>::new())?;
            self.execute_impl(url, headers, timeout, request).await
        }
    }

    async fn execute_impl<B: Body + 'static>(
        &self,
        url: Url,
        headers: HeaderMap,
        timeout: Option<Duration>,
        mut request: hyper::Request<B>,
    ) -> Result<Response>
    where
        B::Data: Send,
        B::Error: std::error::Error + Send + Sync,
    {
        let headers_mut = request.headers_mut();
        *headers_mut = self.client.headers.clone();
        if let Some(host) = url.host_str() {
            headers_mut.append(
                HOST,
                HeaderValue::from_str(host).map_err(|_| Error::BadScheme(url.clone()))?,
            );
        }
        crate::util::replace_headers(headers_mut, headers);
        let stream = HttpStream::connect(request.uri(), self.client.tls).await?;
        let (mut send_request, conn) = handshake(stream).await?;
        compio_runtime::spawn(conn).detach();
        let future = send_request.send_request(request);
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
    pub fn request<U: IntoUrl>(&self, method: Method, url: U) -> RequestBuilder<Empty<Bytes>> {
        RequestBuilder::new(
            self.clone(),
            url.into_url().map(|url| Request::new(method, url)),
        )
    }

    /// Convenience method to make a `GET` request to a URL.
    pub fn get<U: IntoUrl>(&self, url: U) -> RequestBuilder<Empty<Bytes>> {
        self.request(Method::GET, url)
    }

    /// Convenience method to make a `POST` request to a URL.
    pub fn post<U: IntoUrl>(&self, url: U) -> RequestBuilder<Empty<Bytes>> {
        self.request(Method::POST, url)
    }

    /// Convenience method to make a `PUT` request to a URL.
    pub fn put<U: IntoUrl>(&self, url: U) -> RequestBuilder<Empty<Bytes>> {
        self.request(Method::PUT, url)
    }

    /// Convenience method to make a `PATCH` request to a URL.
    pub fn patch<U: IntoUrl>(&self, url: U) -> RequestBuilder<Empty<Bytes>> {
        self.request(Method::PATCH, url)
    }

    /// Convenience method to make a `DELETE` request to a URL.
    pub fn delete<U: IntoUrl>(&self, url: U) -> RequestBuilder<Empty<Bytes>> {
        self.request(Method::DELETE, url)
    }

    /// Convenience method to make a `HEAD` request to a URL.
    pub fn head<U: IntoUrl>(&self, url: U) -> RequestBuilder<Empty<Bytes>> {
        self.request(Method::HEAD, url)
    }
}

#[derive(Debug)]
struct ClientInner {
    headers: HeaderMap,
    tls: TlsBackend,
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
        let client_ref = ClientInner {
            headers: self.headers,
            tls: self.tls,
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
    pub fn use_rustls(mut self) -> Self {
        self.tls = TlsBackend::Rustls;
        self
    }
}
