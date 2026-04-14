#[cfg(feature = "native-tls")]
pub(crate) mod native;
#[cfg(feature = "py-dynamic-openssl")]
pub(crate) mod py_ossl;

mod common;
