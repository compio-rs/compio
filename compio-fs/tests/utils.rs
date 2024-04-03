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

    assert!(
        compio_fs::metadata(new_dir)
            .await
            .expect("metadata result")
            .permissions()
            .readonly()
    );
}

#[compio_macros::test]
async fn remove() {
    let base_dir = tempfile::tempdir().unwrap();
    let new_dir = base_dir.path().join("foo");

    std::fs::create_dir(&new_dir).unwrap();

    compio_fs::remove_dir(&new_dir).await.unwrap();
    assert!(compio_fs::metadata(&new_dir).await.is_err());
}
