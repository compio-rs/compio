use compio::fs::OpenOptions;

fn main() {
    let buffer = compio::task::block_on(async {
        let file = OpenOptions::new().read(true).open("Cargo.toml").unwrap();
        let (read, buffer) = file.read_at(Vec::with_capacity(4096), 0).await;
        let read = read.unwrap();
        assert_eq!(read, buffer.len());
        String::from_utf8(buffer).unwrap()
    });
    println!("{}", buffer);
}
