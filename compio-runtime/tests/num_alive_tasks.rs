use compio_runtime::Runtime;

#[test]
fn num_alive_tasks() {
    let rt = Runtime::new().unwrap();
    assert_eq!(rt.num_alive_tasks(), 0);

    rt.block_on(async {
        // The future given to block_on is not a task.
        let count = || Runtime::with_current(|rt| rt.num_alive_tasks());
        assert_eq!(count(), 0);

        let quick = compio_runtime::spawn(async { 1 });
        compio_runtime::spawn(std::future::pending::<()>()).detach();
        assert_eq!(count(), 2);

        assert_eq!(quick.await.unwrap(), 1);
        assert_eq!(count(), 1);
    });

    // The detached task outlives block_on, until the runtime is dropped.
    assert_eq!(rt.num_alive_tasks(), 1);
}
