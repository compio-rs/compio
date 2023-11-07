use std::io;

use compio_tls::TlsConnector;

/// Represents TLS backend options
#[derive(Debug, Clone, Copy)]
pub enum TlsBackend {
    /// Use [`native_tls`] as TLS backend.
    #[cfg(feature = "native-tls")]
    NativeTls,
    /// Use [`rustls`] as TLS backend.
    #[cfg(feature = "rustls")]
    Rustls,
}

impl Default for TlsBackend {
    fn default() -> Self {
        cfg_if::cfg_if! {
            if #[cfg(feature = "native-tls")] {
                Self::NativeTls
            } else if #[cfg(feature = "rustls")] {
                Self::Rustls
            } else {
                compile_error!("You must choose at least one of these features: [\"native-tls\", \"rustls\"]")
            }
        }
    }
}

impl TlsBackend {
    pub(crate) fn create_connector(&self) -> io::Result<TlsConnector> {
        match self {
            #[cfg(feature = "native-tls")]
            Self::NativeTls => Ok(TlsConnector::from(
                native_tls::TlsConnector::new()
                    .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?,
            )),
            #[cfg(feature = "rustls")]
            Self::Rustls => {
                let mut store = rustls::RootCertStore::empty();
                for cert in rustls_native_certs::load_native_certs().unwrap() {
                    store.add(&rustls::Certificate(cert.0)).unwrap();
                }

                Ok(TlsConnector::from(std::sync::Arc::new(
                    rustls::ClientConfig::builder()
                        .with_safe_defaults()
                        .with_root_certificates(store)
                        .with_no_client_auth(),
                )))
            }
        }
    }
}
