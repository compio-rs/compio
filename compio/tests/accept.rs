use compio::{
    driver::DriverType,
    net::{TcpListener, TcpStream},
};
use compio_runtime::ResumeUnwind;

#[compio_macros::test(with_proactor(driver_type = DriverType::Poll))]
async fn accept() {
    let listener = TcpListener::bind("localhost:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = compio_runtime::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        socket
    });
    let cli = TcpStream::connect(&addr).await.unwrap();
    let srv = task.await.resume_unwind().expect("shouldn't be cancelled");
    assert_eq!(cli.local_addr().unwrap(), srv.peer_addr().unwrap());
}
