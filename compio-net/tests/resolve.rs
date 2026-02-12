use compio_net::ToSocketAddrsAsync;

#[compio_macros::test]
async fn resolve_localhost() {
    let addrs = "localhost:80".to_socket_addrs_async().await.unwrap();
    let mut found = false;
    for addr in addrs {
        if addr.ip().is_loopback() {
            found = true;
            break;
        }
    }
    assert!(found, "localhost should resolve to a loopback address");
}

#[cfg(all(unix, feature = "dns-cache"))]
#[compio_macros::test]
async fn dns_cache_speedup() {
    use std::time::Instant;

    let target = "cloudflare.com:443";

    let t1 = Instant::now();
    let _ = target.to_socket_addrs_async().await.unwrap();
    let cold = t1.elapsed();

    let t2 = Instant::now();
    let _ = target.to_socket_addrs_async().await.unwrap();
    let warm = t2.elapsed();

    println!(
        "cold: {cold:?}, warm: {warm:?}, speedup: {:.0}x",
        cold.as_secs_f64() / warm.as_secs_f64()
    );
    assert!(warm < cold, "cached should be faster");
}
