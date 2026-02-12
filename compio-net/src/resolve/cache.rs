//! DNS resolution cache backed by `scc::HashCache`.
//!
//! Enabled by the default `dns-cache` feature.  The cache uses per-bucket LRU
//! eviction (32-way associative) and honours DNS TTL with configurable
//! min/max clamping, following the approach used by blastdns.

use std::{
    net::IpAddr,
    time::{Duration, Instant},
};

use scc::HashCache;

/// Capacity of the DNS cache.
const CACHE_CAPACITY: usize = 32768;
/// Minimum TTL for cached entries (clamp floor).
const MIN_TTL: Duration = Duration::from_secs(60);
/// Maximum TTL for cached entries (clamp ceiling).
const MAX_TTL: Duration = Duration::from_secs(86400); // 1 day

#[derive(Clone)]
struct CacheEntry {
    addrs: Vec<IpAddr>,
    expire_at: Instant,
}

/// A global, lock-free DNS cache.
pub(crate) struct DnsCache {
    inner: HashCache<String, CacheEntry>,
}

impl DnsCache {
    pub fn new() -> Self {
        Self {
            inner: HashCache::with_capacity(CACHE_CAPACITY, CACHE_CAPACITY * 2),
        }
    }

    /// Look up cached addresses for `name`.
    /// Returns `None` on miss or if the entry has expired.
    pub async fn get(&self, name: &str) -> Option<Vec<IpAddr>> {
        let key = name.to_lowercase();
        let entry = self.inner.get_async(&key).await?;
        let val = entry.get();
        if val.expire_at > Instant::now() {
            Some(val.addrs.clone())
        } else {
            drop(entry);
            // Lazily evict expired entry.
            let _ = self.inner.remove_async(&key).await;
            None
        }
    }

    /// Insert resolved addresses with a given TTL (in seconds).
    /// Zero-TTL results are not cached.
    pub async fn insert(&self, name: String, addrs: Vec<IpAddr>, ttl_secs: u32) {
        if ttl_secs == 0 {
            return;
        }
        let ttl = Duration::from_secs(ttl_secs as u64).clamp(MIN_TTL, MAX_TTL);
        let entry = CacheEntry {
            addrs,
            expire_at: Instant::now() + ttl,
        };
        let _ = self.inner.put_async(name.to_lowercase(), entry).await;
    }
}
