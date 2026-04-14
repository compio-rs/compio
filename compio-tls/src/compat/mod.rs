#[cfg(feature = "native-tls")]
pub(crate) mod native;
#[cfg(feature = "py-dynamic-openssl")]
pub(crate) mod py_ossl;

#[cfg(any(feature = "native-tls", feature = "py-dynamic-openssl"))]
mod openssl_workaround;
