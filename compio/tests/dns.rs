use compio::net::resolve_sock_addrs;

#[test]
fn resolve_localhost() {
    compio::task::block_on(async {
        let addrs = resolve_sock_addrs("localhost", 0).await.unwrap();
        assert_eq!(addrs.len(), 2);
    })
}
