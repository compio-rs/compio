use compio::named_pipe::{ClientOptions, ServerOptions};

const PIPE_NAME: &str = r"\\.\pipe\tokio-iocp-named-pipe";

fn main() {
    compio::task::block_on(async {
        let server = ServerOptions::new()
            .access_inbound(false)
            .create(PIPE_NAME)
            .unwrap();
        let client = ClientOptions::new().write(false).open(PIPE_NAME).unwrap();

        server.connect().await.unwrap();

        let write = server.write("Hello world!");
        let buffer = Vec::with_capacity(64);
        let read = client.read(buffer);

        let ((write, _), (read, buffer)) = futures_util::join!(write, read);
        write.unwrap();
        read.unwrap();
        println!("{}", String::from_utf8(buffer).unwrap());
    });
}
