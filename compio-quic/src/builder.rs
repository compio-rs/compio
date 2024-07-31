use std::{
    io,
    net::{SocketAddrV4, SocketAddrV6},
    sync::Arc,
    time::Duration,
};

use compio_net::{ToSocketAddrsAsync, UdpSocket};
use quinn_proto::{
    crypto::rustls::{QuicClientConfig, QuicServerConfig},
    ClientConfig, EndpointConfig, ServerConfig, TransportConfig,
};

use crate::Endpoint;

/// A [builder] for [`Endpoint`] in client mode.
///
/// To get one, call [`Endpoint::client()`] or [`ClientBuilder::default()`].
///
/// [builder]: https://rust-unofficial.github.io/patterns/patterns/creational/builder.html
#[derive(Debug)]
pub struct ClientBuilder<T> {
    inner: T,

    alpn_protocols: Vec<Vec<u8>>,
    key_log: bool,
    enable_early_data: bool,

    transport: Option<TransportConfig>,
    version: Option<u32>,

    endpoint_config: EndpointConfig,
}

impl Default for ClientBuilder<()> {
    fn default() -> Self {
        Self {
            inner: (),
            alpn_protocols: Vec::new(),
            key_log: false,
            enable_early_data: true,
            transport: None,
            version: None,
            endpoint_config: EndpointConfig::default(),
        }
    }
}

impl<T, E> From<ClientBuilder<Result<T, E>>> for Result<ClientBuilder<T>, E> {
    fn from(builder: ClientBuilder<Result<T, E>>) -> Self {
        builder.inner.map(|inner| ClientBuilder {
            inner,
            alpn_protocols: builder.alpn_protocols,
            key_log: builder.key_log,
            enable_early_data: builder.enable_early_data,
            transport: builder.transport,
            version: builder.version,
            endpoint_config: builder.endpoint_config,
        })
    }
}

impl<T> ClientBuilder<T> {
    fn map_inner<S>(self, f: impl FnOnce(T) -> S) -> ClientBuilder<S> {
        ClientBuilder {
            inner: f(self.inner),
            alpn_protocols: self.alpn_protocols,
            key_log: self.key_log,
            enable_early_data: self.enable_early_data,
            transport: self.transport,
            version: self.version,
            endpoint_config: self.endpoint_config,
        }
    }

    /// Set the ALPN protocols to use.
    pub fn with_alpn_protocols(mut self, protocols: &[&str]) -> Self {
        self.alpn_protocols = protocols.iter().map(|p| p.as_bytes().to_vec()).collect();
        self
    }

    /// Logging key material to a file for debugging. The file's name is given
    /// by the `SSLKEYLOGFILE` environment variable.
    ///
    /// If `SSLKEYLOGFILE` is not set, or such a file cannot be opened or cannot
    /// be written, this does nothing.
    pub fn with_key_log(mut self) -> Self {
        self.key_log = true;
        self
    }

    /// Set a custom [`TransportConfig`].
    pub fn with_transport_config(mut self, transport: TransportConfig) -> Self {
        self.transport = Some(transport);
        self
    }

    /// Set the QUIC version to use.
    pub fn with_version(mut self, version: u32) -> Self {
        self.version = Some(version);
        self
    }

    /// Use the provided [`EndpointConfig`].
    pub fn with_endpoint_config(mut self, endpoint_config: EndpointConfig) -> Self {
        self.endpoint_config = endpoint_config;
        self
    }
}

impl ClientBuilder<()> {
    /// Use the provided [`rustls::ClientConfig`].
    pub fn with_rustls_client_config(
        self,
        client_config: rustls::ClientConfig,
    ) -> ClientBuilder<rustls::ClientConfig> {
        self.map_inner(|_| client_config)
    }

    /// Do not verify the server's certificate. It is vulnerable to MITM
    /// attacks, but convenient for testing.
    pub fn with_no_server_verification(
        self,
    ) -> ClientBuilder<Arc<dyn rustls::client::danger::ServerCertVerifier>> {
        self.map_inner(|_| Arc::new(verifier::SkipServerVerification::new()) as _)
    }

    /// Use [`rustls_platform_verifier`].
    #[cfg(feature = "platform-verifier")]
    pub fn with_platform_verifier(
        self,
    ) -> ClientBuilder<Arc<dyn rustls::client::danger::ServerCertVerifier>> {
        self.map_inner(|_| Arc::new(rustls_platform_verifier::Verifier::new()) as _)
    }

    /// Use an empty [`rustls::RootCertStore`].
    pub fn with_root_certificates(self) -> ClientBuilder<rustls::RootCertStore> {
        self.map_inner(|_| rustls::RootCertStore::empty())
    }
}

impl ClientBuilder<rustls::ClientConfig> {
    /// Create an [`Endpoint`] binding to the addr provided.
    pub async fn bind(self, addr: impl ToSocketAddrsAsync) -> io::Result<Endpoint> {
        let mut client_config = self.inner;

        client_config.alpn_protocols = self.alpn_protocols;
        if self.key_log {
            client_config.key_log = Arc::new(rustls::KeyLogFile::new());
        }
        client_config.enable_early_data = self.enable_early_data;

        let mut client_config = ClientConfig::new(Arc::new(
            QuicClientConfig::try_from(client_config)
                .expect("should support TLS13_AES_128_GCM_SHA256"),
        ));

        if let Some(transport) = self.transport {
            client_config.transport_config(Arc::new(transport));
        }
        if let Some(version) = self.version {
            client_config.version(version);
        }

        let socket = UdpSocket::bind(addr).await?;
        Endpoint::new(socket, self.endpoint_config, None, Some(client_config))
    }
}

impl ClientBuilder<Arc<dyn rustls::client::danger::ServerCertVerifier>> {
    /// Create an [`Endpoint`] binding to the addr provided.
    pub async fn bind(self, addr: impl ToSocketAddrsAsync) -> io::Result<Endpoint> {
        self.map_inner(|verifier| {
            rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
                .dangerous()
                .with_custom_certificate_verifier(verifier)
                .with_no_client_auth()
        })
        .bind(addr)
        .await
    }
}

impl ClientBuilder<rustls::RootCertStore> {
    /// Use [`rustls_native_certs`].
    #[cfg(feature = "native-certs")]
    pub fn with_native_certs(mut self) -> io::Result<Self> {
        self.inner
            .add_parsable_certificates(rustls_native_certs::load_native_certs()?);
        Ok(self)
    }

    /// Use [`webpki_roots`].
    #[cfg(feature = "webpki-roots")]
    pub fn with_webpki_roots(mut self) -> Self {
        self.inner
            .extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        self
    }

    /// Add a custom certificate.
    pub fn with_custom_certificate(
        mut self,
        der: rustls::pki_types::CertificateDer,
    ) -> Result<Self, rustls::Error> {
        self.inner.add(der)?;
        Ok(self)
    }

    /// Verify the revocation state of presented client certificates against the
    /// provided certificate revocation lists (CRLs).
    pub fn with_crls(
        self,
        crls: impl IntoIterator<Item = rustls::pki_types::CertificateRevocationListDer<'static>>,
    ) -> Result<
        ClientBuilder<Arc<dyn rustls::client::danger::ServerCertVerifier>>,
        rustls::client::VerifierBuilderError,
    > {
        self.map_inner(|roots| {
            rustls::client::WebPkiServerVerifier::builder(Arc::new(roots))
                .with_crls(crls)
                .build()
                .map(|v| v as _)
        })
        .into()
    }

    /// Create an [`Endpoint`] binding to the addr provided.
    pub async fn bind(self, addr: impl ToSocketAddrsAsync) -> io::Result<Endpoint> {
        self.map_inner(|roots| {
            rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
                .with_root_certificates(roots)
                .with_no_client_auth()
        })
        .bind(addr)
        .await
    }
}

/// A [builder] for [`Endpoint`] in server mode.
///
/// To get one, call [`Endpoint::server()`] or [`ServerBuilder::default()`].
///
/// [builder]: https://rust-unofficial.github.io/patterns/patterns/creational/builder.html
#[derive(Debug)]
pub struct ServerBuilder<T> {
    inner: T,

    alpn_protocols: Vec<Vec<u8>>,
    key_log: bool,
    enable_early_data: bool,

    transport: Option<TransportConfig>,
    retry_token_lifetime: Option<Duration>,
    migration: bool,
    preferred_address_v4: Option<SocketAddrV4>,
    preferred_address_v6: Option<SocketAddrV6>,
    max_incoming: Option<usize>,
    incoming_buffer_size: Option<u64>,
    incoming_buffer_size_total: Option<u64>,

    endpoint_config: EndpointConfig,
}

impl Default for ServerBuilder<()> {
    fn default() -> Self {
        Self {
            inner: (),
            alpn_protocols: Vec::new(),
            key_log: false,
            enable_early_data: true,
            transport: None,
            retry_token_lifetime: None,
            migration: true,
            preferred_address_v4: None,
            preferred_address_v6: None,
            max_incoming: None,
            incoming_buffer_size: None,
            incoming_buffer_size_total: None,
            endpoint_config: EndpointConfig::default(),
        }
    }
}

impl<T> ServerBuilder<T> {
    fn map_inner<S>(self, f: impl FnOnce(T) -> S) -> ServerBuilder<S> {
        ServerBuilder {
            inner: f(self.inner),
            alpn_protocols: self.alpn_protocols,
            key_log: self.key_log,
            enable_early_data: self.enable_early_data,
            transport: self.transport,
            retry_token_lifetime: self.retry_token_lifetime,
            migration: self.migration,
            preferred_address_v4: self.preferred_address_v4,
            preferred_address_v6: self.preferred_address_v6,
            max_incoming: self.max_incoming,
            incoming_buffer_size: self.incoming_buffer_size,
            incoming_buffer_size_total: self.incoming_buffer_size_total,
            endpoint_config: self.endpoint_config,
        }
    }

    /// Set the ALPN protocols to use.
    pub fn with_alpn_protocols(mut self, protocols: &[&str]) -> Self {
        self.alpn_protocols = protocols.iter().map(|p| p.as_bytes().to_vec()).collect();
        self
    }

    /// Logging key material to a file for debugging. The file's name is given
    /// by the `SSLKEYLOGFILE` environment variable.
    ///
    /// If `SSLKEYLOGFILE` is not set, or such a file cannot be opened or cannot
    /// be written, this does nothing.
    pub fn with_key_log(mut self) -> Self {
        self.key_log = true;
        self
    }

    /// Set a custom [`TransportConfig`].
    pub fn with_transport_config(mut self, transport: TransportConfig) -> Self {
        self.transport = Some(transport);
        self
    }

    /// Duration after a stateless retry token was issued for which it's
    /// considered valid.
    pub fn with_retry_token_lifetime(mut self, retry_token_lifetime: Duration) -> Self {
        self.retry_token_lifetime = Some(retry_token_lifetime);
        self
    }

    /// Whether to allow clients to migrate to new addresses.
    ///
    /// See [`quinn_proto::ServerConfig::migration`].
    pub fn with_migration(mut self, migration: bool) -> Self {
        self.migration = migration;
        self
    }

    /// The preferred IPv4 address during handshaking.
    ///
    /// See [`quinn_proto::ServerConfig::preferred_address_v4`].
    pub fn with_preferred_address_v4(mut self, addr: SocketAddrV4) -> Self {
        self.preferred_address_v4 = Some(addr);
        self
    }

    /// The preferred IPv6 address during handshaking.
    ///
    /// See [`quinn_proto::ServerConfig::preferred_address_v6`].
    pub fn with_preferred_address_v6(mut self, addr: SocketAddrV6) -> Self {
        self.preferred_address_v6 = Some(addr);
        self
    }

    /// Maximum number of [`Incoming`][crate::Incoming] to allow to exist at a
    /// time.
    ///
    /// See [`quinn_proto::ServerConfig::max_incoming`].
    pub fn with_max_incoming(mut self, max_incoming: usize) -> Self {
        self.max_incoming = Some(max_incoming);
        self
    }

    /// Maximum number of received bytes to buffer for each
    /// [`Incoming`][crate::Incoming].
    ///
    /// See [`quinn_proto::ServerConfig::incoming_buffer_size`].
    pub fn with_incoming_buffer_size(mut self, incoming_buffer_size: u64) -> Self {
        self.incoming_buffer_size = Some(incoming_buffer_size);
        self
    }

    /// Maximum number of received bytes to buffer for all
    /// [`Incoming`][crate::Incoming] collectively.
    ///
    /// See [`quinn_proto::ServerConfig::incoming_buffer_size_total`].
    pub fn with_incoming_buffer_size_total(mut self, incoming_buffer_size_total: u64) -> Self {
        self.incoming_buffer_size_total = Some(incoming_buffer_size_total);
        self
    }

    /// Use the provided [`EndpointConfig`].
    pub fn with_endpoint_config(mut self, endpoint_config: EndpointConfig) -> Self {
        self.endpoint_config = endpoint_config;
        self
    }
}

impl ServerBuilder<()> {
    /// Use the provided [`rustls::ServerConfig`].
    pub fn with_rustls_server_config(
        self,
        server_config: rustls::ServerConfig,
    ) -> ServerBuilder<rustls::ServerConfig> {
        self.map_inner(|_| server_config)
    }

    /// Sets a single certificate chain and matching private key.
    pub fn with_single_cert(
        self,
        cert_chain: Vec<rustls::pki_types::CertificateDer<'static>>,
        key_der: rustls::pki_types::PrivateKeyDer<'static>,
    ) -> Result<ServerBuilder<rustls::ServerConfig>, rustls::Error> {
        let server_config =
            rustls::ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
                .with_no_client_auth()
                .with_single_cert(cert_chain, key_der)?;
        Ok(self.with_rustls_server_config(server_config))
    }
}

impl ServerBuilder<rustls::ServerConfig> {
    /// Create an [`Endpoint`] binding to the addr provided.
    pub async fn bind(self, addr: impl ToSocketAddrsAsync) -> io::Result<Endpoint> {
        let mut server_config = self.inner;

        server_config.alpn_protocols = self.alpn_protocols;
        if self.key_log {
            server_config.key_log = Arc::new(rustls::KeyLogFile::new());
        }
        if self.enable_early_data {
            server_config.max_early_data_size = u32::MAX;
        }

        let mut server_config = ServerConfig::with_crypto(Arc::new(
            QuicServerConfig::try_from(server_config)
                .expect("should support TLS13_AES_128_GCM_SHA256"),
        ));

        if let Some(transport) = self.transport {
            server_config.transport_config(Arc::new(transport));
        }
        if let Some(retry_token_lifetime) = self.retry_token_lifetime {
            server_config.retry_token_lifetime(retry_token_lifetime);
        }
        server_config
            .migration(self.migration)
            .preferred_address_v4(self.preferred_address_v4)
            .preferred_address_v6(self.preferred_address_v6);
        if let Some(max_incoming) = self.max_incoming {
            server_config.max_incoming(max_incoming);
        }
        if let Some(incoming_buffer_size) = self.incoming_buffer_size {
            server_config.incoming_buffer_size(incoming_buffer_size);
        }
        if let Some(incoming_buffer_size_total) = self.incoming_buffer_size_total {
            server_config.incoming_buffer_size_total(incoming_buffer_size_total);
        }

        let socket = UdpSocket::bind(addr).await?;
        Endpoint::new(socket, self.endpoint_config, Some(server_config), None)
    }
}

mod verifier {
    use rustls::{
        client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
        crypto::WebPkiSupportedAlgorithms,
        pki_types::{CertificateDer, ServerName, UnixTime},
        DigitallySignedStruct, Error, SignatureScheme,
    };

    #[derive(Debug)]
    pub struct SkipServerVerification(WebPkiSupportedAlgorithms);

    impl SkipServerVerification {
        pub fn new() -> Self {
            Self(
                rustls::crypto::CryptoProvider::get_default()
                    .unwrap()
                    .signature_verification_algorithms,
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
