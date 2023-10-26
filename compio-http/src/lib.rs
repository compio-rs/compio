//! A high level HTTP client library based on compio.

#![warn(missing_docs)]

mod client;
pub use client::*;

mod request;
pub use request::*;

mod stream;
pub(crate) use stream::*;

mod service;
pub(crate) use service::*;

mod util;

use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    #[error("request timeout")]
    Timeout,
    #[error("system error: {0}")]
    System(#[from] std::io::Error),
    #[error("`http` error: {0}")]
    Http(#[from] http::Error),
    #[error("`hyper` error: {0}")]
    Hyper(#[from] hyper::Error),
    #[error("url encode error: {0}")]
    UrlEncoded(#[from] serde_urlencoded::ser::Error),
    #[cfg(feature = "json")]
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
