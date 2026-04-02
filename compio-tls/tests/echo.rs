use std::{net::SocketAddr, sync::Arc};

use compio_io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use compio_net::{TcpListener, TcpStream};
use compio_tls::{TlsAcceptor, TlsConnector};
use rcgen::Certificate;
use rustls::pki_types::pem::PemObject;

async fn start_server() -> (SocketAddr, Certificate) {
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

    let listener = TcpListener::bind("localhost:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    compio_runtime::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut stream = acceptor.accept(stream).await.unwrap();
        let (_, res) = stream.read(Vec::with_capacity(12)).await.unwrap();
        stream.write_all(res).await.unwrap();
        stream.flush().await.unwrap();
        stream.shutdown().await.unwrap();
        stream.read(vec![]).await.unwrap();
    })
    .detach();
    (addr, cert)
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
    let (addr, cert) = start_server().await;

    let connector = TlsConnector::from(
        native_tls::TlsConnector::builder()
            .add_root_certificate(native_tls::Certificate::from_pem(cert.pem().as_bytes()).unwrap())
            .build()
            .unwrap(),
    );

    connect(connector, addr).await;
}

#[compio_macros::test]
async fn rtls() {
    let (addr, cert) = start_server().await;

    let mut store = rustls::RootCertStore::empty();
    store.add(cert.der().clone()).unwrap();

    let connector = TlsConnector::from(Arc::new(
        rustls::ClientConfig::builder()
            .with_root_certificates(store)
            .with_no_client_auth(),
    ));

    connect(connector, addr).await;
}

#[cfg(feature = "py-dynamic-openssl")]
#[compio_macros::test]
async fn py_ossl() {
    use compio_py_dynamic_openssl::pyo3;
    use pyo3::{IntoPyObjectExt, types::IntoPyDict};

    let (addr, cert) = start_server().await;

    pyo3::Python::initialize();
    let context = pyo3::Python::attach(|py| {
        let loaded = compio_py_dynamic_openssl::load_py(py).unwrap();
        assert!(loaded);
        let module = py.import("ssl").unwrap();
        let context = py
            .eval(
                c"ctx = ssl.create_default_context(); ctx.load_verify_locations(cafile=cert); ctx",
                None,
                Some(
                    &[
                        ("ssl", module.into_bound_py_any(py).unwrap()),
                        ("cert", cert.pem().into_bound_py_any(py).unwrap()),
                    ]
                    .into_py_dict(py)
                    .unwrap(),
                ),
            )
            .unwrap();
        compio_py_dynamic_openssl::SSLContext::try_from(context).unwrap()
    });
    let connector = TlsConnector::from(context);

    connect(connector, addr).await;
}
