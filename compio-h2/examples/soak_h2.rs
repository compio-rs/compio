//! H2 soak test — runs many requests and monitors RSS to detect memory leaks.
//!
//! Environment variables:
//! - `SOAK_REQUESTS` — total requests (default: 50,000)
//! - `SOAK_PAYLOAD_SIZE` — payload bytes (default: 1,024)
//! - `SOAK_BATCH_SIZE` — RSS sample interval (default: 5,000)
//! - `SOAK_RECONNECT_EVERY` — reconnect interval in requests, 0=never (default:
//!   0)
//! - `SOAK_MAX_RSS_GROWTH_KB` — fail threshold in KB (default: 10,240 = 10 MB)

#[path = "../benches/support.rs"]
mod support;

fn main() {
    let total_requests: u64 = support::env_or("SOAK_REQUESTS", 50_000);
    let payload_size: usize = support::env_or("SOAK_PAYLOAD_SIZE", 1024);
    let batch_size: u64 = support::env_or("SOAK_BATCH_SIZE", 5_000);
    let reconnect_every: u64 = support::env_or("SOAK_RECONNECT_EVERY", 0);
    let max_rss_growth_kb: u64 = support::env_or("SOAK_MAX_RSS_GROWTH_KB", 10_240);

    let reconnect_label = if reconnect_every == 0 {
        "never".to_string()
    } else {
        format!("every {reconnect_every} requests")
    };
    let num_batches = total_requests.div_ceil(batch_size);

    println!("=== H2 Soak Test ===");
    println!(
        "Requests: {total_requests}  Payload: {payload_size}B  Batch: {batch_size}  Reconnect: \
         {reconnect_label}"
    );

    compio_runtime::Runtime::new().unwrap().block_on(async {
        // Start server
        let listener = compio_net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        compio_runtime::spawn(support::compio_h2_server_loop(listener)).detach();

        let payload = support::make_payload(payload_size);

        // Connect client
        let mut send_req = support::compio_h2_client_connect(addr).await;

        // Warm up
        println!("Warming up (100 requests)...");
        for _ in 0..100 {
            support::compio_h2_roundtrip(&mut send_req, payload.clone()).await;
        }

        // Baseline RSS
        let baseline_rss = support::get_rss_kb().unwrap_or(0);
        let mut peak_rss = baseline_rss;
        let mut completed: u64 = 0;
        let mut batch_num: u64 = 0;

        println!("Baseline RSS: {baseline_rss} KB");
        println!();

        while completed < total_requests {
            let this_batch = std::cmp::min(batch_size, total_requests - completed);

            for _ in 0..this_batch {
                support::compio_h2_roundtrip(&mut send_req, payload.clone()).await;
                completed += 1;

                // Reconnect if configured
                if reconnect_every > 0 && completed.is_multiple_of(reconnect_every) {
                    println!("  Reconnecting client...");
                    send_req = support::compio_h2_client_connect(addr).await;
                }
            }

            batch_num += 1;

            if let Some(rss) = support::get_rss_kb() {
                if rss > peak_rss {
                    peak_rss = rss;
                }
                println!(
                    "  Batch {batch_num}/{num_batches}: {completed} requests done, RSS: {rss} KB"
                );
            }
        }

        // Final RSS
        let final_rss = support::get_rss_kb().unwrap_or(0);
        if final_rss > peak_rss {
            peak_rss = final_rss;
        }
        let growth = final_rss.saturating_sub(baseline_rss);

        println!();
        println!("--- RSS Report ---");
        println!("Baseline: {baseline_rss} KB");
        println!("Peak:     {peak_rss} KB");
        println!("Final:    {final_rss} KB");
        println!("Growth:   {growth} KB (threshold: {max_rss_growth_kb} KB)");

        if growth > max_rss_growth_kb {
            println!(
                "=== FAIL === (RSS growth {growth} KB exceeds threshold {max_rss_growth_kb} KB)"
            );
            std::process::exit(1);
        } else {
            println!("=== PASS ===");
        }
    });
}
