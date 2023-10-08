use compio::{
    buf::IntoInner,
    dispatcher::Dispatcher,
    net::{TcpListener, TcpStream},
    runtime::{spawn, Unattached},
    BufResult,
};

#[compio::main(crate = "compio")]
async fn main() {
    const THREAD_NUM: usize = 5;
    const CLIENT_NUM: usize = 10;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let dispatcher = Dispatcher::new(THREAD_NUM);
    let task = spawn(async move {
        for i in 0..CLIENT_NUM {
            let cli = TcpStream::connect(&addr).await.unwrap();
            cli.send_all(format!("Hello world {}!", i)).await.unwrap();
        }
    });
    for _i in 0..CLIENT_NUM {
        let (srv, _) = listener.accept().await.unwrap();
        let srv = Unattached::new(srv).unwrap();
        dispatcher
            .dispatch(move || {
                let srv = srv.into_inner();
                async move {
                    let BufResult(res, buf) = srv.recv(Vec::with_capacity(20)).await;
                    res?;
                    println!("{}", std::str::from_utf8(&buf).unwrap());
                    Ok(())
                }
            })
            .unwrap();
    }
    task.await;
    for res in dispatcher.join() {
        res.unwrap();
    }
}
