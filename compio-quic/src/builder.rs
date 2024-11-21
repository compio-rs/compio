use std::{io, sync::Arc};

use compio_net::ToSocketAddrsAsync;
use quinn_proto::{
    ClientConfig, ServerConfig,
    crypto::rustls::{QuicClientConfig, QuicServerConfig},
};

use crate::Endpoint;

/// Helper to construct an [`Endpoint`] for use with outgoing connections only.
///
/// To get one, call `new_with_xxx` methods.
///
/// [builder]: https://rust-unofficial.github.io/patterns/patterns/creational/builder.html
#[derive(Debug)]
pub struct ClientBuilder<T>(T);

impl ClientBuilder<rustls::RootCertStore> {
    /// Create a builder with an empty [`rustls::RootCertStore`].
    pub fn new_with_empty_roots() -> Self {
        ClientBuilder(rustls::RootCertStore::empty())
    }

    /// Create a builder with [`rustls_native_certs`].
    #[cfg(feature = "native-certs")]
    pub fn new_with_native_certs() -> io::Result<Self> {
        let mut roots = rustls::RootCertStore::empty();
        let mut certs = rustls_native_certs::load_native_certs();
        if certs.certs.is_empty() {
            return Err(io::Error::other(
                certs
                    .errors
                    .pop()
                    .expect("certs and errors should not be both empty"),
            ));
        }
        roots.add_parsable_certificates(certs.certs);
        Ok(ClientBuilder(roots))
    }

    /// Create a builder with [`webpki_roots`].
    #[cfg(feature = "webpki-roots")]
    pub fn new_with_webpki_roots() -> Self {
        let roots =
            rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        ClientBuilder(roots)
    }

    /// Add a custom certificate.
    pub fn with_custom_certificate(
        mut self,
        der: rustls::pki_types::CertificateDer,
    ) -> Result<Self, rustls::Error> {
        self.0.add(der)?;
        Ok(self)
    }

    /// Don't configure revocation.
    pub fn with_no_crls(self) -> ClientBuilder<rustls::ClientConfig> {
        ClientBuilder::new_with_root_certificates(self.0)
    }

    /// Verify the revocation state of presented client certificates against the
    /// provided certificate revocation lists (CRLs).
    pub fn with_crls(
        self,
        crls: impl IntoIterator<Item = rustls::pki_types::CertificateRevocationListDer<'static>>,
    ) -> Result<ClientBuilder<rustls::ClientConfig>, rustls::client::VerifierBuilderError> {
        let verifier = rustls::client::WebPkiServerVerifier::builder(Arc::new(self.0))
            .with_crls(crls)
            .build()?;
        Ok(ClientBuilder::new_with_webpki_verifier(verifier))
    }
}

impl ClientBuilder<rustls::ClientConfig> {
    /// Create a builder with the provided [`rustls::ClientConfig`].
    pub fn new_with_rustls_client_config(
        client_config: rustls::ClientConfig,
    ) -> ClientBuilder<rustls::ClientConfig> {
        ClientBuilder(client_config)
    }

    /// Do not verify the server's certificate. It is vulnerable to MITM
    /// attacks, but convenient for testing.
    pub fn new_with_no_server_verification() -> ClientBuilder<rustls::ClientConfig> {
        ClientBuilder(
            rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(verifier::SkipServerVerification::new()))
                .with_no_client_auth(),
        )
    }

    /// Create a builder with [`rustls_platform_verifier`].
    #[cfg(feature = "platform-verifier")]
    pub fn new_with_platform_verifier() -> ClientBuilder<rustls::ClientConfig> {
        use rustls_platform_verifier::BuilderVerifierExt;

        ClientBuilder(
            rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
                .with_platform_verifier()
                .with_no_client_auth(),
        )
    }

    /// Create a builder with the provided [`rustls::RootCertStore`].
    pub fn new_with_root_certificates(
        roots: rustls::RootCertStore,
    ) -> ClientBuilder<rustls::ClientConfig> {
        ClientBuilder(
            rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
                .with_root_certificates(roots)
                .with_no_client_auth(),
        )
    }

    /// Create a builder with a custom [`rustls::client::WebPkiServerVerifier`].
    pub fn new_with_webpki_verifier(
        verifier: Arc<rustls::client::WebPkiServerVerifier>,
    ) -> ClientBuilder<rustls::ClientConfig> {
        ClientBuilder(
            rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
                .with_webpki_verifier(verifier)
                .with_no_client_auth(),
        )
    }

    /// Set the ALPN protocols to use.
    pub fn with_alpn_protocols(mut self, protocols: &[&str]) -> Self {
        self.0.alpn_protocols = protocols.iter().map(|p| p.as_bytes().to_vec()).collect();
        self
    }

    /// Logging key material to a file for debugging. The file's name is given
    /// by the `SSLKEYLOGFILE` environment variable.
    ///
    /// If `SSLKEYLOGFILE` is not set, or such a file cannot be opened or cannot
    /// be written, this does nothing.
    pub fn with_key_log(mut self) -> Self {
        self.0.key_log = Arc::new(rustls::KeyLogFile::new());
        self
    }

    /// Build a [`ClientConfig`].
    pub fn build(mut self) -> ClientConfig {
        self.0.enable_early_data = true;
        ClientConfig::new(Arc::new(
            QuicClientConfig::try_from(self.0).expect("should support TLS13_AES_128_GCM_SHA256"),
        ))
    }

    /// Create a new [`Endpoint`].
    ///
    /// See [`Endpoint::client`] for more information.
    pub async fn bind(self, addr: impl ToSocketAddrsAsync) -> io::Result<Endpoint> {
        let mut endpoint = Endpoint::client(addr).await?;
        endpoint.default_client_config = Some(self.build());
        Ok(endpoint)
    }
}

/// Helper to construct an [`Endpoint`] for use with incoming connections.
///
/// To get one, call `new_with_xxx` methods.
///
/// [builder]: https://rust-unofficial.github.io/patterns/patterns/creational/builder.html
#[derive(Debug)]
pub struct ServerBuilder<T>(T);

impl ServerBuilder<rustls::ServerConfig> {
    /// Create a builder with the provided [`rustls::ServerConfig`].
    pub fn new_with_rustls_server_config(server_config: rustls::ServerConfig) -> Self {
        Self(server_config)
    }

    /// Create a builder with a single certificate chain and matching private
    /// key. Using this method gets the same result as calling
    /// [`ServerConfig::with_single_cert`].
    pub fn new_with_single_cert(
        cert_chain: Vec<rustls::pki_types::CertificateDer<'static>>,
        key_der: rustls::pki_types::PrivateKeyDer<'static>,
    ) -> Result<Self, rustls::Error> {
        let server_config =
            rustls::ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
                .with_no_client_auth()
                .with_single_cert(cert_chain, key_der)?;
        Ok(Self::new_with_rustls_server_config(server_config))
    }

    /// Set the ALPN protocols to use.
    pub fn with_alpn_protocols(mut self, protocols: &[&str]) -> Self {
        self.0.alpn_protocols = protocols.iter().map(|p| p.as_bytes().to_vec()).collect();
        self
    }

    /// Logging key material to a file for debugging. The file's name is given
    /// by the `SSLKEYLOGFILE` environment variable.
    ///
    /// If `SSLKEYLOGFILE` is not set, or such a file cannot be opened or cannot
    /// be written, this does nothing.
    pub fn with_key_log(mut self) -> Self {
        self.0.key_log = Arc::new(rustls::KeyLogFile::new());
        self
    }

    /// Build a [`ServerConfig`].
    pub fn build(mut self) -> ServerConfig {
        self.0.max_early_data_size = u32::MAX;
        ServerConfig::with_crypto(Arc::new(
            QuicServerConfig::try_from(self.0).expect("should support TLS13_AES_128_GCM_SHA256"),
        ))
    }

    /// Create a new [`Endpoint`].
    ///
    /// See [`Endpoint::server`] for more information.
    pub async fn bind(self, addr: impl ToSocketAddrsAsync) -> io::Result<Endpoint> {
        Endpoint::server(addr, self.build()).await
    }
}

mod verifier {
    use rustls::{
        DigitallySignedStruct, Error, SignatureScheme,
        client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
        crypto::WebPkiSupportedAlgorithms,
        pki_types::{CertificateDer, ServerName, UnixTime},
    };

    #[derive(Debug)]
    pub struct SkipServerVerification(WebPkiSupportedAlgorithms);

    impl SkipServerVerification {
        pub fn new() -> Self {
            Self(
                rustls::crypto::CryptoProvider::get_default()
                    .map(|provider| provider.signature_verification_algorithms)
                    .unwrap_or_else(|| {
                        #[cfg(feature = "aws-lc-rs")]
                        use rustls::crypto::aws_lc_rs::default_provider;
                        #[cfg(all(not(feature = "aws-lc-rs"), feature = "ring"))]
                        use rustls::crypto::ring::default_provider;
                        default_provider().signature_verification_algorithms
                    }),
            )
        }
    }

    impl ServerCertVerifier for SkipServerVerification {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp: &[u8],
            _now: UnixTime,
        ) -> Result<ServerCertVerified, Error> {
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            rustls::crypto::verify_tls12_signature(message, cert, dss, &self.0)
        }

        fn verify_tls13_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            rustls::crypto::verify_tls13_signature(message, cert, dss, &self.0)
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            self.0.supported_schemes()
        }
    }
}
