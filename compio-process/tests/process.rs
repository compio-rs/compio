use std::process::Stdio;

use compio_process::Command;

#[compio_macros::test]
async fn exit_code() {
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

#[compio_macros::test]
async fn echo() {
    let mut cmd;

    if cfg!(windows) {
        cmd = Command::new("cmd");
        cmd.arg("/c");
    } else {
        cmd = Command::new("sh");
        cmd.arg("-c");
    }

    let child = cmd
        .arg("echo hello world")
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();

    let id = child.id();
    assert!(id > 0);

    let output = child.wait_with_output().await.unwrap();
    assert!(output.status.success());

    let out = String::from_utf8(output.stdout).unwrap();
    assert_eq!(out.trim(), "hello world");
}

#[cfg(unix)]
#[compio_macros::test]
async fn arg0() {
    let mut cmd = Command::new("sh");
    cmd.arg0("test_string")
        .arg("-c")
        .arg("echo $0")
        .stdout(Stdio::piped());

    let output = cmd.output().await.unwrap();
    assert_eq!(output.stdout, b"test_string\n");
}
