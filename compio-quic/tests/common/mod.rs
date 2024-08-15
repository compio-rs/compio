use std::sync::Arc;

use compio_log::subscriber::DefaultGuard;
use compio_quic::{ClientBuilder, ClientConfig, ServerBuilder, ServerConfig, TransportConfig};
use tracing_subscriber::{util::SubscriberInitExt, EnvFilter};

pub fn subscribe() -> DefaultGuard {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .finish()
        .set_default()
}

pub fn config_pair(transport: Option<TransportConfig>) -> (ServerConfig, ClientConfig) {
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let cert = cert.der().clone();
    let key_der = key_pair.serialize_der().try_into().unwrap();

    let mut server_config = ServerBuilder::new_with_single_cert(vec![cert.clone()], key_der)
        .unwrap()
        .build();
    let mut client_config = ClientBuilder::new_with_empty_roots()
        .with_custom_certificate(cert)
        .unwrap()
        .with_no_crls()
        .build();
    if let Some(transport) = transport {
        let transport = Arc::new(transport);
        server_config.transport_config(transport.clone());
        client_config.transport_config(transport);
    }
    (server_config, client_config)
}
