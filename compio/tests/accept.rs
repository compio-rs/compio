use std::panic::resume_unwind;

use compio::{
    driver::{DriverType, ProactorBuilder},
    net::{TcpListener, TcpStream},
    runtime::Runtime,
};

#[test]
fn accept() {
    let mut proactor_builder = ProactorBuilder::new();
    proactor_builder.driver_type(DriverType::Poll);
    let runtime = Runtime::builder()
        .with_proactor(proactor_builder)
        .build()
        .unwrap();
    runtime.block_on(async {
        let listener = TcpListener::bind("localhost:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = compio_runtime::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            socket
        });
        let cli = TcpStream::connect(&addr).await.unwrap();
        let srv = task.await.unwrap_or_else(|e| resume_unwind(e));
        assert_eq!(cli.local_addr().unwrap(), srv.peer_addr().unwrap());
    })
}
