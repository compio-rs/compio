use compio::{
    io::{AsyncReadExt, AsyncWriteExt},
    BufResult,
};

#[compio::main]
async fn main() {
    #[cfg(windows)]
    {
        use compio::fs::named_pipe::{ClientOptions, ServerOptions};

        const PIPE_NAME: &str = r"\\.\pipe\compio-named-pipe";

        let mut server = ServerOptions::new()
            .access_inbound(false)
            .create(PIPE_NAME)
            .unwrap();
        let mut client = ClientOptions::new()
            .write(false)
            .open(PIPE_NAME)
            .await
            .unwrap();

        server.connect().await.unwrap();

        let write = server.write_all("Hello world!");
        let buffer = Vec::with_capacity(12);
        let read = client.read_exact(buffer);

        let (BufResult(write, _), BufResult(read, buffer)) = futures_util::join!(write, read);
        write.unwrap();
        read.unwrap();
        println!("{}", String::from_utf8(buffer).unwrap());
    }
    #[cfg(unix)]
    {
        use compio::fs::pipe::OpenOptions;
        use nix::{sys::stat::Mode, unistd::mkfifo};
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let file = dir.path().join("compio-named-pipe");

        mkfifo(&file, Mode::S_IRWXU).unwrap();
        let options = OpenOptions::new();
        let (mut rx, mut tx) =
            futures_util::try_join!(options.open_receiver(&file), options.open_sender(&file))
                .unwrap();

        let write = tx.write_all("Hello world!");
        let buffer = Vec::with_capacity(12);
        let read = rx.read_exact(buffer);

        let (BufResult(write, _), BufResult(read, buffer)) = futures_util::join!(write, read);
        write.unwrap();
        read.unwrap();
        println!("{}", String::from_utf8(buffer).unwrap());
    }
}
