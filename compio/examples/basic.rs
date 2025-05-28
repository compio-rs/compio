use compio::{fs::OpenOptions, io::AsyncReadAtExt};

#[compio::main]
async fn main() {
    let file = OpenOptions::new()
        .read(true)
        .open("Cargo.toml")
        .await
        .unwrap();
    let (read, buffer) = file
        .read_to_end_at(Vec::with_capacity(4096), 0)
        .await
        .unwrap();
    assert_eq!(read, buffer.len());
    let buffer = String::from_utf8(buffer).unwrap();
    println!("{buffer}");
    file.close().await.unwrap();
}
