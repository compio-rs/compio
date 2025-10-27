#[test]
fn test_drop() {
    compio_runtime::Runtime::new().unwrap().block_on(async {
        compio_runtime::spawn(async {
            loop {
                compio_runtime::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        })
        .detach();
    })
}
