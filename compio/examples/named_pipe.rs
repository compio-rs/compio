#[compio::main]
async fn main() {
    #[cfg(target_os = "windows")]
    {
        use compio::{
            buf::BufResult,
            named_pipe::{ClientOptions, ServerOptions},
        };

        const PIPE_NAME: &str = r"\\.\pipe\compio-named-pipe";

        let server = ServerOptions::new()
            .access_inbound(false)
            .create(PIPE_NAME)
            .unwrap();
        let client = ClientOptions::new().write(false).open(PIPE_NAME).unwrap();

        server.connect().await.unwrap();

        let write = server.write_all("Hello world!");
        let buffer = Vec::with_capacity(12);
        let read = client.read_exact(buffer);

        let (BufResult(write, _), BufResult(read, buffer)) = futures_util::join!(write, read);
        write.unwrap();
        read.unwrap();
        println!("{}", String::from_utf8(buffer).unwrap());
    }
}
