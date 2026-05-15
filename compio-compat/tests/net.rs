use compio_compat::{RuntimeCompat, TokioAdapter};
use compio_runtime::Runtime;

#[tokio::test]
async fn compio_client() {
    use compio_net::TcpStream;
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = async {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buffer = [0u8; 12];
        stream.read_exact(&mut buffer).await.unwrap();
        stream.write_all(&buffer).await.unwrap();
        stream.shutdown().await.unwrap();
    };

    let runtime = RuntimeCompat::<TokioAdapter>::new(Runtime::new().unwrap()).unwrap();
    let client = runtime.execute(async {
        use compio_io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};

        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream.write_all(b"hello world!").await.unwrap();
        stream.shutdown().await.unwrap();
        let buffer = [0u8; 12];
        let (_, buffer) = stream.read_exact(buffer).await.unwrap();
        assert_eq!(&buffer, b"hello world!");
        stream.close().await.unwrap();
    });

    tokio::join!(server, client);
}

#[tokio::test]
async fn compio_server() {
    use compio_net::TcpListener;
    use tokio::net::TcpStream;

    let runtime = RuntimeCompat::<TokioAdapter>::new(Runtime::new().unwrap()).unwrap();
    let listener = runtime
        .execute(TcpListener::bind("127.0.0.1:0"))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let server = runtime.execute(async {
        use compio_io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

        let (mut stream, _) = listener.accept().await.unwrap();
        let buffer = [0u8; 12];
        let (_, buffer) = stream.read_exact(buffer).await.unwrap();
        stream.write_all(buffer).await.unwrap();
        stream.shutdown().await.unwrap();
        // It's a good practice to read after shutdown to ensure that FIN is sent and
        // received properly, especially when using compio.
        stream.read([0u8]).await.unwrap();
        stream.close().await.unwrap();
    });

    let client = async {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream.write_all(b"hello world!").await.unwrap();
        stream.shutdown().await.unwrap();
        let mut buffer = [0u8; 12];
        stream.read_exact(&mut buffer).await.unwrap();
        assert_eq!(&buffer, b"hello world!");
        stream.shutdown().await.unwrap();
    };

    tokio::join!(server, client);
}
