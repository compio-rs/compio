use compio_process::Command;

#[compio_macros::test]
async fn simple() {
    let mut cmd;

    if cfg!(windows) {
        cmd = Command::new("cmd");
        cmd.arg("/c");
    } else {
        cmd = Command::new("sh");
        cmd.arg("-c");
    }

    let child = cmd.arg("exit 2").spawn().unwrap();

    let id = child.id();
    assert!(id > 0);

    let status = child.wait().await.unwrap();
    assert_eq!(status.code(), Some(2));
}
