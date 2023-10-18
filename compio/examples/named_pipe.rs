#[compio::main(crate = "compio")]
async fn main() {
    #[cfg(windows)]
    {
        use compio::{
            buf::BufResult,
            fs::named_pipe::{ClientOptions, ServerOptions},
            io::{AsyncReadExt, AsyncWriteExt},
        };

        const PIPE_NAME: &str = r"\\.\pipe\compio-named-pipe";

        let mut server = ServerOptions::new()
            .access_inbound(false)
            .create(PIPE_NAME)
            .unwrap();
        let mut client = ClientOptions::new().write(false).open(PIPE_NAME).unwrap();

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
        use compio::{buf::IntoInner, fs::pipe::OpenOptions, runtime::Unattached, BufResult};
        use nix::{sys::stat::Mode, unistd::mkfifo};
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let file = dir.path().join("compio-named-pipe");

        mkfifo(&file, Mode::S_IRWXU).unwrap();

        let (rx, tx) = std::thread::scope(|s| {
            let rx = s.spawn(|| {
                Unattached::new(OpenOptions::new().open_receiver(&file).unwrap()).unwrap()
            });
            let tx = s
                .spawn(|| Unattached::new(OpenOptions::new().open_sender(&file).unwrap()).unwrap());
            (
                rx.join().unwrap().into_inner(),
                tx.join().unwrap().into_inner(),
            )
        });

        let write = tx.write_all("Hello world!");
        let buffer = Vec::with_capacity(12);
        let read = rx.read_exact(buffer);

        let (BufResult(write, _), BufResult(read, buffer)) = futures_util::join!(write, read);
        write.unwrap();
        read.unwrap();
        println!("{}", String::from_utf8(buffer).unwrap());
    }
}
