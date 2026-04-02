use std::net::SocketAddr;

use compio_io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use compio_net::{TcpListener, TcpStream};
use compio_runtime::JoinHandle;
use compio_tls::{TlsAcceptor, TlsConnector};

async fn start_server(acceptor: TlsAcceptor) -> (SocketAddr, JoinHandle<()>) {
    let listener = TcpListener::bind("localhost:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = compio_runtime::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut stream = acceptor.accept(stream).await.unwrap();
        let (_, res) = stream.read(Vec::with_capacity(12)).await.unwrap();
        stream.write_all(res).await.unwrap();
        stream.flush().await.unwrap();
        stream.shutdown().await.unwrap();
        stream.read(vec![0u8; 1]).await.unwrap();
    });
    (addr, server)
}

async fn connect(connector: TlsConnector, addr: SocketAddr) {
    let stream = TcpStream::connect(addr).await.unwrap();
    let mut stream = connector.connect("localhost", stream).await.unwrap();

    stream.write_all("Hello world!").await.unwrap();
    stream.flush().await.unwrap();
    let (_, res) = stream.read_to_end(vec![]).await.unwrap();
    assert_eq!(res, b"Hello world!");
    stream.shutdown().await.unwrap()
}

#[cfg(feature = "native-tls")]
#[compio_macros::test]
async fn native() {
    // https://github.com/rustls/rcgen/issues/91

    use rsa::pkcs8::EncodePrivateKey;

    let mut rng = rand::rng();
    let bits = 2048;
    let private_key = rsa::RsaPrivateKey::new(&mut rng, bits).unwrap();
    let private_key_der = private_key.to_pkcs8_der().unwrap();
    let signing_key = rcgen::KeyPair::try_from(private_key_der.as_bytes()).unwrap();
    let cert = rcgen::CertificateParams::new(["localhost".into()])
        .unwrap()
        .self_signed(&signing_key)
        .unwrap();

    let acceptor = TlsAcceptor::from(
        native_tls::TlsAcceptor::builder(
            native_tls::Identity::from_pkcs8(
                cert.pem().as_bytes(),
                signing_key.serialize_pem().as_bytes(),
            )
            .unwrap(),
        )
        .build()
        .unwrap(),
    );

    let (addr, server) = start_server(acceptor).await;

    let connector = TlsConnector::from(
        native_tls::TlsConnector::builder()
            .add_root_certificate(native_tls::Certificate::from_pem(cert.pem().as_bytes()).unwrap())
            .build()
            .unwrap(),
    );

    connect(connector, addr).await;
    server.await.unwrap();
}

#[cfg(feature = "rustls")]
#[compio_macros::test]
async fn rtls() {
    use std::sync::Arc;

    use rustls::pki_types::pem::PemObject;

    let rcgen::CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();

    let acceptor = TlsAcceptor::from(Arc::new(
        rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![cert.der().clone()],
                rustls::pki_types::PrivateKeyDer::from_pem_slice(
                    signing_key.serialize_pem().as_bytes(),
                )
                .unwrap(),
            )
            .unwrap(),
    ));

    let (addr, server) = start_server(acceptor).await;

    let mut store = rustls::RootCertStore::empty();
    store.add(cert.der().clone()).unwrap();

    let connector = TlsConnector::from(Arc::new(
        rustls::ClientConfig::builder()
            .with_root_certificates(store)
            .with_no_client_auth(),
    ));

    connect(connector, addr).await;
    server.await.unwrap();
}

#[cfg(feature = "py-dynamic-openssl")]
#[compio_macros::test]
async fn py_ossl() {
    use std::io::Write;

    use compio_py_dynamic_openssl::pyo3;
    use pyo3::{
        IntoPyObjectExt,
        types::{IntoPyDict, PyDictMethods},
    };

    let rcgen::CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();

    let mut cert_path = tempfile::NamedTempFile::new().unwrap();
    cert_path.write_all(cert.pem().as_bytes()).unwrap();

    let mut key_path = tempfile::NamedTempFile::new().unwrap();
    key_path
        .write_all(signing_key.serialize_pem().as_bytes())
        .unwrap();

    pyo3::Python::initialize();
    let (client_ctx, server_ctx) = pyo3::Python::attach(|py| {
        let loaded = compio_py_dynamic_openssl::load_py(py).unwrap();
        assert!(loaded);
        let ssl = py.import("ssl").unwrap();
        let locals = [
            ("ssl", ssl.into_bound_py_any(py).unwrap()),
            ("cert", cert_path.path().into_bound_py_any(py).unwrap()),
            ("key", key_path.path().into_bound_py_any(py).unwrap()),
        ]
        .into_py_dict(py)
        .unwrap();
        py.run(
            cr#"
ctx_client = ssl.create_default_context()
ctx_client.load_verify_locations(cafile=cert)
ctx_server = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
ctx_server.load_cert_chain(certfile=cert, keyfile=key)
"#,
            None,
            Some(&locals),
        )
        .unwrap();
        let client_ctx = locals.get_item("ctx_client").unwrap().unwrap();
        let server_ctx = locals.get_item("ctx_server").unwrap().unwrap();
        (
            compio_py_dynamic_openssl::SSLContext::try_from(client_ctx).unwrap(),
            compio_py_dynamic_openssl::SSLContext::try_from(server_ctx).unwrap(),
        )
    });
    let connector = TlsConnector::from(client_ctx);
    let acceptor = TlsAcceptor::from(server_ctx);
    let (addr, server) = start_server(acceptor).await;

    connect(connector, addr).await;
    server.await.unwrap();
}
