use compio_io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};
use compio_net::TcpStream;
use compio_tls::TlsConnector;

async fn connect(connector: TlsConnector) {
    let stream = TcpStream::connect("compio.rs:443").await.unwrap();
    let mut stream = connector.connect("compio.rs", stream).await.unwrap();

    stream
        .write_all("GET / HTTP/1.1\r\nHost:compio.rs\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    stream.flush().await.unwrap();
    let (_, res) = stream.read_to_end(vec![]).await.unwrap();
    println!("{}", String::from_utf8_lossy(&res));
    stream.shutdown().await.unwrap();
}

#[cfg(feature = "native-tls")]
#[compio_macros::test]
async fn native() {
    let connector = TlsConnector::from(native_tls::TlsConnector::new().unwrap());

    connect(connector).await;
}

#[cfg(feature = "rustls")]
#[compio_macros::test]
async fn rtls() {
    let mut store = rustls::RootCertStore::empty();
    for cert in rustls_native_certs::load_native_certs().unwrap() {
        store.add(cert).unwrap();
    }

    let connector = TlsConnector::from(std::sync::Arc::new(
        rustls::ClientConfig::builder()
            .with_root_certificates(store)
            .with_no_client_auth(),
    ));

    connect(connector).await;
}

#[cfg(feature = "py-dynamic-openssl")]
#[compio_macros::test]
async fn py_ossl() {
    use pyo3::types::IntoPyDict;

    pyo3::Python::initialize();
    let context = pyo3::Python::attach(|py| {
        let loaded = compio_py_dynamic_openssl::load_py(py).unwrap();
        assert!(loaded);
        let module = py.import("ssl").unwrap();
        let context = py
            .eval(
                c"ssl.create_default_context()",
                None,
                Some(&[("ssl", module)].into_py_dict(py).unwrap()),
            )
            .unwrap();
        compio_py_dynamic_openssl::SSLContext::try_from(&context).unwrap()
    });
    let connector = TlsConnector::from(context);

    connect(connector).await;
}
