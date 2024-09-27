use std::io::Write;

#[compio_macros::test]
async fn path_read_write() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();

    compio_fs::write(dir.join("bar"), b"bytes").await.unwrap();
    let out = compio_fs::read(dir.join("bar")).await.unwrap();

    assert_eq!(out, b"bytes");
}

#[compio_macros::test]
async fn create_dir() {
    let base_dir = tempfile::tempdir().unwrap();
    let new_dir = base_dir.path().join("foo");

    compio_fs::create_dir(&new_dir).await.unwrap();

    assert!(compio_fs::metadata(&new_dir).await.unwrap().is_dir());
}

#[compio_macros::test]
async fn create_all() {
    let base_dir = tempfile::tempdir().unwrap();
    let new_dir = base_dir.path().join("foo").join("bar");

    compio_fs::create_dir_all(&new_dir).await.unwrap();
    assert!(compio_fs::metadata(&new_dir).await.unwrap().is_dir());
}

#[compio_macros::test]
async fn build_dir() {
    let base_dir = tempfile::tempdir().unwrap();
    let new_dir = base_dir.path().join("foo").join("bar");

    compio_fs::DirBuilder::new()
        .recursive(true)
        .create(&new_dir)
        .await
        .unwrap();

    assert!(compio_fs::metadata(&new_dir).await.unwrap().is_dir());
    compio_fs::DirBuilder::new()
        .recursive(false)
        .create(&new_dir)
        .await
        .unwrap_err();
}

#[compio_macros::test]
#[cfg(unix)]
async fn build_dir_mode_read_only() {
    use std::os::unix::fs::DirBuilderExt;

    let base_dir = tempfile::tempdir().unwrap();
    let new_dir = base_dir.path().join("abc");

    compio_fs::DirBuilder::new()
        .recursive(true)
        .mode(0o444)
        .create(&new_dir)
        .await
        .unwrap();

    assert!(compio_fs::metadata(new_dir)
        .await
        .expect("metadata result")
        .permissions()
        .readonly());
}

#[compio_macros::test]
async fn remove() {
    let base_dir = tempfile::tempdir().unwrap();
    let new_dir = base_dir.path().join("foo");

    std::fs::create_dir(&new_dir).unwrap();

    compio_fs::remove_dir(&new_dir).await.unwrap();
    assert!(compio_fs::metadata(&new_dir).await.is_err());
}

#[compio_macros::test]
async fn test_hard_link() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src.txt");
    let dst = dir.path().join("dst.txt");

    std::fs::File::create(&src)
        .unwrap()
        .write_all(b"hello")
        .unwrap();

    compio_fs::hard_link(&src, &dst).await.unwrap();

    std::fs::File::create(&src)
        .unwrap()
        .write_all(b"new-data")
        .unwrap();

    let content = compio_fs::read(&dst).await.unwrap();
    assert_eq!(content, b"new-data");

    // test that this is not a symlink:
    assert!(std::fs::read_link(&dst).is_err());
}

#[compio_macros::test]
#[cfg(unix)]
async fn test_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src.txt");
    let dst = dir.path().join("dst.txt");

    std::fs::File::create(&src)
        .unwrap()
        .write_all(b"hello")
        .unwrap();

    compio_fs::symlink(&src, &dst).await.unwrap();

    std::fs::File::create(&src)
        .unwrap()
        .write_all(b"new-data")
        .unwrap();

    let content = compio_fs::read(&dst).await.unwrap();
    assert_eq!(content, b"new-data");

    let read = std::fs::read_link(dst.clone()).unwrap();
    assert!(read == src);

    let symlink_meta = compio_fs::symlink_metadata(dst.clone()).await.unwrap();
    assert!(symlink_meta.file_type().is_symlink());
}

#[compio_macros::test]
#[cfg(windows)]
async fn symlink_file_windows() {
    let dir = tempfile::tempdir().unwrap();

    let source_path = dir.path().join("foo.txt");
    let dest_path = dir.path().join("bar.txt");

    compio_fs::write(&source_path, b"Hello File!")
        .await
        .unwrap();
    compio_fs::symlink_file(&source_path, &dest_path)
        .await
        .unwrap();

    compio_fs::write(&source_path, b"new data!").await.unwrap();

    let from = compio_fs::read(&source_path).await.unwrap();
    let to = compio_fs::read(&dest_path).await.unwrap();

    assert_eq!(from, to);
}

#[compio_macros::test]
#[cfg(windows)]
async fn symlink_dir_windows() {
    const FILE_NAME: &str = "abc.txt";

    let temp_dir = tempfile::tempdir().unwrap();

    let dir1 = temp_dir.path().join("a");
    compio_fs::create_dir(&dir1).await.unwrap();

    let file1 = dir1.as_path().join(FILE_NAME);
    compio_fs::write(&file1, b"Hello File!").await.unwrap();

    let dir2 = temp_dir.path().join("b");
    compio_fs::symlink_dir(&dir1, &dir2).await.unwrap();

    compio_fs::write(&file1, b"new data!").await.unwrap();

    let file2 = dir2.as_path().join(FILE_NAME);

    let from = compio_fs::read(&file1).await.unwrap();
    let to = compio_fs::read(&file2).await.unwrap();

    assert_eq!(from, to);
}
