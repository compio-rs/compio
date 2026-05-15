use std::io::Read;

use compio_compat::{Adapter, RuntimeCompat};
use compio_fs::File;
use compio_io::AsyncReadAtExt;
use compio_runtime::Runtime;

async fn test_impl<A: Adapter>() {
    let runtime = Runtime::new().unwrap();
    let runtime = RuntimeCompat::<A>::new(runtime).unwrap();
    let buffer = runtime
        .execute(async {
            let file = File::open("Cargo.toml").await.unwrap();
            let (_, buffer) = file.read_to_string_at(String::new(), 0).await.unwrap();
            buffer
        })
        .await;

    let mut file = std::fs::File::open("Cargo.toml").unwrap();
    let mut expected = String::new();
    file.read_to_string(&mut expected).unwrap();

    assert_eq!(buffer, expected);
}

#[cfg(feature = "tokio")]
#[tokio::test]
async fn tokio() {
    test_impl::<compio_compat::TokioAdapter>().await;
}

#[cfg(feature = "futures")]
#[test]
fn futures() {
    futures_executor::block_on(async {
        test_impl::<compio_compat::FuturesAdapter>().await;
    })
}
