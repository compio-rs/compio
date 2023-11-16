use std::{fmt::Display, time::Duration};

use hyper::{
    body::Body,
    header::{HeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE},
    HeaderMap, Method, Version,
};
use serde::Serialize;
use url::Url;

use crate::{Client, Response, Result};

#[derive(Debug)]
struct RequestHeader {
    method: Method,
    url: Url,
    headers: HeaderMap,
    timeout: Option<Duration>,
    version: Version,
}

impl RequestHeader {
    pub fn new(method: Method, url: Url) -> Self {
        Self {
            method,
            url,
            headers: HeaderMap::new(),
            timeout: None,
            version: Version::default(),
        }
    }
}

/// A request which can be executed with `Client::execute()`.
#[derive(Debug)]
pub struct Request<B> {
    header: RequestHeader,
    body: Option<B>,
}

impl<B> Request<B> {
    /// Constructs a new request.
    #[inline]
    pub fn new(method: Method, url: Url) -> Self {
        Self {
            header: RequestHeader::new(method, url),
            body: None,
        }
    }

    /// Get the method.
    #[inline]
    pub fn method(&self) -> &Method {
        &self.header.method
    }

    /// Get a mutable reference to the method.
    #[inline]
    pub fn method_mut(&mut self) -> &mut Method {
        &mut self.header.method
    }

    /// Get the url.
    #[inline]
    pub fn url(&self) -> &Url {
        &self.header.url
    }

    /// Get a mutable reference to the url.
    #[inline]
    pub fn url_mut(&mut self) -> &mut Url {
        &mut self.header.url
    }

    /// Get the headers.
    #[inline]
    pub fn headers(&self) -> &HeaderMap {
        &self.header.headers
    }

    /// Get a mutable reference to the headers.
    #[inline]
    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.header.headers
    }

    /// Get the body.
    #[inline]
    pub fn body(&self) -> Option<&B> {
        self.body.as_ref()
    }

    /// Get a mutable reference to the body.
    #[inline]
    pub fn body_mut(&mut self) -> &mut Option<B> {
        &mut self.body
    }

    /// Get the timeout.
    #[inline]
    pub fn timeout(&self) -> Option<&Duration> {
        self.header.timeout.as_ref()
    }

    /// Get a mutable reference to the timeout.
    #[inline]
    pub fn timeout_mut(&mut self) -> &mut Option<Duration> {
        &mut self.header.timeout
    }

    /// Get the http version.
    #[inline]
    pub fn version(&self) -> Version {
        self.header.version
    }

    /// Get a mutable reference to the http version.
    #[inline]
    pub fn version_mut(&mut self) -> &mut Version {
        &mut self.header.version
    }

    pub(super) fn pieces(self) -> (Method, Url, HeaderMap, Option<B>, Option<Duration>, Version) {
        (
            self.header.method,
            self.header.url,
            self.header.headers,
            self.body,
            self.header.timeout,
            self.header.version,
        )
    }
}

/// A builder to construct the properties of a `Request`.
#[derive(Debug)]
pub struct RequestBuilder<B> {
    client: Client,
    request: Result<Request<B>>,
}

impl<B> RequestBuilder<B> {
    pub(crate) fn new(client: Client, request: Result<Request<B>>) -> RequestBuilder<B> {
        RequestBuilder { client, request }
    }

    /// Assemble a builder starting from an existing `Client` and a `Request`.
    pub fn from_parts(client: Client, request: Request<B>) -> RequestBuilder<B> {
        RequestBuilder {
            client,
            request: Ok(request),
        }
    }

    /// Add a `Header` to this Request.
    pub fn header<K: TryInto<HeaderName>, V: TryInto<HeaderValue>>(
        self,
        key: K,
        value: V,
    ) -> RequestBuilder<B>
    where
        K::Error: Into<http::Error>,
        V::Error: Into<http::Error>,
    {
        self.header_sensitive(key, value, false)
    }

    /// Add a `Header` to this Request with ability to define if `header_value`
    /// is sensitive.
    fn header_sensitive<K: TryInto<HeaderName>, V: TryInto<HeaderValue>>(
        mut self,
        key: K,
        value: V,
        sensitive: bool,
    ) -> RequestBuilder<B>
    where
        K::Error: Into<http::Error>,
        V::Error: Into<http::Error>,
    {
        let mut error = None;
        if let Ok(ref mut req) = self.request {
            match key.try_into() {
                Ok(key) => match value.try_into() {
                    Ok(mut value) => {
                        // We want to potentially make an unsensitive header
                        // to be sensitive, not the reverse. So, don't turn off
                        // a previously sensitive header.
                        if sensitive {
                            value.set_sensitive(true);
                        }
                        req.headers_mut().append(key, value);
                    }
                    Err(e) => error = Some(e.into()),
                },
                Err(e) => error = Some(e.into()),
            };
        }
        if let Some(err) = error {
            self.request = Err(err.into());
        }
        self
    }

    /// Add a set of Headers to the existing ones on this Request.
    ///
    /// The headers will be merged in to any already set.
    pub fn headers(mut self, headers: HeaderMap) -> RequestBuilder<B> {
        if let Ok(ref mut req) = self.request {
            crate::util::replace_headers(req.headers_mut(), headers);
        }
        self
    }

    /// Enable HTTP basic authentication.
    ///
    /// ```rust
    /// # use compio_http_client::Error;
    ///
    /// # async fn run() -> Result<(), Error> {
    /// let client = compio_http_client::Client::new();
    /// let resp = client
    ///     .delete("http://httpbin.org/delete")
    ///     .basic_auth("admin", Some("good password"))
    ///     .send()
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn basic_auth<U: Display, P: Display>(
        self,
        username: U,
        password: Option<P>,
    ) -> RequestBuilder<B> {
        let header_value = crate::util::basic_auth(username, password);
        self.header_sensitive(AUTHORIZATION, header_value, true)
    }

    /// Enable HTTP bearer authentication.
    pub fn bearer_auth<T: Display>(self, token: T) -> RequestBuilder<B> {
        let header_value = format!("Bearer {}", token);
        self.header_sensitive(AUTHORIZATION, header_value, true)
    }

    /// Set the request body.
    pub fn body<T: Body>(self, body: T) -> RequestBuilder<T> {
        match self.request {
            Ok(req) => {
                let new_request = Request {
                    header: req.header,
                    body: Some(body),
                };
                RequestBuilder::from_parts(self.client, new_request)
            }
            Err(err) => RequestBuilder::new(self.client, Err(err)),
        }
    }

    /// Enables a request timeout.
    ///
    /// The timeout is applied from when the request starts connecting until the
    /// response body has finished. It affects only this request and overrides
    /// the timeout configured using `ClientBuilder::timeout()`.
    pub fn timeout(mut self, timeout: Duration) -> RequestBuilder<B> {
        if let Ok(ref mut req) = self.request {
            *req.timeout_mut() = Some(timeout);
        }
        self
    }

    /// Modify the query string of the URL.
    ///
    /// Modifies the URL of this request, adding the parameters provided.
    /// This method appends and does not overwrite. This means that it can
    /// be called multiple times and that existing query parameters are not
    /// overwritten if the same key is used. The key will simply show up
    /// twice in the query string.
    /// Calling `.query(&[("foo", "a"), ("foo", "b")])` gives `"foo=a&foo=b"`.
    ///
    /// # Note
    /// This method does not support serializing a single key-value
    /// pair. Instead of using `.query(("key", "val"))`, use a sequence, such
    /// as `.query(&[("key", "val")])`. It's also possible to serialize structs
    /// and maps into a key-value pair.
    ///
    /// # Errors
    /// This method will fail if the object you provide cannot be serialized
    /// into a query string.
    pub fn query<T: Serialize + ?Sized>(mut self, query: &T) -> RequestBuilder<B> {
        let mut error = None;
        if let Ok(ref mut req) = self.request {
            let url = req.url_mut();
            let mut pairs = url.query_pairs_mut();
            let serializer = serde_urlencoded::Serializer::new(&mut pairs);

            if let Err(err) = query.serialize(serializer) {
                error = Some(err.into());
            }
        }
        if let Ok(ref mut req) = self.request {
            if let Some("") = req.url().query() {
                req.url_mut().set_query(None);
            }
        }
        if let Some(err) = error {
            self.request = Err(err);
        }
        self
    }

    /// Set HTTP version
    pub fn version(mut self, version: Version) -> RequestBuilder<B> {
        if let Ok(ref mut req) = self.request {
            req.header.version = version;
        }
        self
    }

    /// Send a form body.
    ///
    /// Sets the body to the url encoded serialization of the passed value,
    /// and also sets the `Content-Type: application/x-www-form-urlencoded`
    /// header.
    ///
    /// ```rust
    /// # use compio_http_client::Error;
    /// # use std::collections::HashMap;
    /// #
    /// # async fn run() -> Result<(), Error> {
    /// let mut params = HashMap::new();
    /// params.insert("lang", "rust");
    ///
    /// let client = compio_http_client::Client::new();
    /// let res = client
    ///     .post("http://httpbin.org")
    ///     .form(&params)
    ///     .send()
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// This method fails if the passed value cannot be serialized into
    /// url encoded format
    pub fn form<T: Serialize + ?Sized>(self, form: &T) -> RequestBuilder<String> {
        match self.request {
            Ok(req) => match serde_urlencoded::to_string(form) {
                Ok(body) => {
                    let mut new_request = Request {
                        header: req.header,
                        body: Some(body),
                    };
                    new_request.headers_mut().insert(
                        CONTENT_TYPE,
                        HeaderValue::from_static("application/x-www-form-urlencoded"),
                    );
                    RequestBuilder::from_parts(self.client, new_request)
                }
                Err(err) => RequestBuilder::new(self.client, Err(err.into())),
            },
            Err(err) => RequestBuilder::new(self.client, Err(err)),
        }
    }

    /// Send a JSON body.
    ///
    /// # Errors
    ///
    /// Serialization can fail if `T`'s implementation of `Serialize` decides to
    /// fail, or if `T` contains a map with non-string keys.
    #[cfg(feature = "json")]
    pub fn json<T: Serialize + ?Sized>(self, json: &T) -> RequestBuilder<Vec<u8>> {
        match self.request {
            Ok(req) => match serde_json::to_vec(json) {
                Ok(body) => {
                    let mut new_request = Request {
                        header: req.header,
                        body: Some(body),
                    };
                    if !new_request.headers().contains_key(CONTENT_TYPE) {
                        new_request
                            .headers_mut()
                            .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
                    }
                    RequestBuilder::from_parts(self.client, new_request)
                }
                Err(err) => RequestBuilder::new(self.client, Err(err.into())),
            },
            Err(err) => RequestBuilder::new(self.client, Err(err)),
        }
    }

    /// Build a `Request`, which can be inspected, modified and executed with
    /// `Client::execute()`.
    pub fn build(self) -> Result<Request<B>> {
        self.request
    }

    /// Build a `Request`, which can be inspected, modified and executed with
    /// `Client::execute()`.
    ///
    /// This is similar to [`RequestBuilder::build()`], but also returns the
    /// embedded `Client`.
    pub fn build_split(self) -> (Client, Result<Request<B>>) {
        (self.client, self.request)
    }
}

impl<B: Body + 'static> RequestBuilder<B>
where
    B::Data: Send,
    B::Error: std::error::Error + Send + Sync,
{
    /// Constructs the Request and sends it to the target URL, returning a
    /// future Response.
    pub async fn send(self) -> Result<Response> {
        self.client.execute(self.request?).await
    }
}
