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

#[compio_macros::test]
async fn rename() {
    let dir = Dir::open(".").await.unwrap();
    dir.create_dir("test").await.unwrap();
    dir.rename("test", &dir, "test2").await.unwrap();
    assert!(dir.open_dir("test").await.is_err());
    let test2 = dir.open_dir("test2").await.unwrap();
    drop(test2);
    dir.remove_dir("test2").await.unwrap();
    assert!(dir.open_dir("test2").await.is_err());
}

#[compio_macros::test]
#[should_panic]
async fn test_absolute() {
    #[cfg(unix)]
    const ABSOLUTE_PATH: &str = "/usr";
    #[cfg(windows)]
    const ABSOLUTE_PATH: &str = "C:\\";

    let dir = Dir::open(".").await.unwrap();
    dir.open_dir(ABSOLUTE_PATH).await.unwrap();
}
