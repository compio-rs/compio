use compio_fs::Dir;

#[compio_macros::test]
async fn open_dir() {
    let dir = Dir::open(".").await.unwrap();
    let meta = dir.dir_metadata().await.unwrap();
    assert!(meta.is_dir());
}

#[compio_macros::test]
async fn read_file() {
    let dir = Dir::open(".").await.unwrap();
    let contents = dir.read("Cargo.toml").await.unwrap();

    let file = dir.open_file("Cargo.toml").await.unwrap();
    let meta = file.metadata().await.unwrap();
    assert!(meta.is_file());
    assert_eq!(contents.len() as u64, meta.len());
}
