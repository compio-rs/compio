use std::time::{Duration, Instant};

use crate::frame::Ping;

/// Keepalive via PING/PONG mechanism.
pub struct PingPong {
    enabled: bool,
    interval: Duration,
    timeout: Duration,
    last_ping_sent: Option<Instant>,
    pending_ping: Option<[u8; 8]>,
    ping_counter: u64,
}

impl PingPong {
    /// Create an enabled PingPong with the given interval and timeout.
    pub fn new(interval: Duration, timeout: Duration) -> Self {
        PingPong {
            enabled: true,
            interval,
            timeout,
            last_ping_sent: None,
            pending_ping: None,
            ping_counter: 0,
        }
    }

    /// Create a disabled PingPong that never sends PINGs.
    pub fn disabled() -> Self {
        PingPong {
            enabled: false,
            interval: Duration::from_secs(0),
            timeout: Duration::from_secs(0),
            last_ping_sent: None,
            pending_ping: None,
            ping_counter: 0,
        }
    }

    /// Whether keepalive PINGs are enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Check if it's time to send a PING frame.
    pub fn maybe_send_ping(&mut self) -> Option<Ping> {
        if !self.enabled {
            return None;
        }

        if self.pending_ping.is_some() {
            return None;
        }

        let should_ping = match self.last_ping_sent {
            None => true,
            Some(last) => last.elapsed() >= self.interval,
        };

        if should_ping {
            self.ping_counter += 1;
            let data = self.ping_counter.to_be_bytes();
            self.pending_ping = Some(data);
            self.last_ping_sent = Some(Instant::now());
            Some(Ping::new(data))
        } else {
            None
        }
    }

    /// Handle receiving a PING ACK. Returns true if it matches our pending
    /// ping.
    pub fn recv_pong(&mut self, data: &[u8; 8]) -> bool {
        if let Some(pending) = &self.pending_ping
            && pending == data
        {
            self.pending_ping = None;
            return true;
        }
        false
    }

    /// Check if the pending PING has timed out.
    pub fn is_timed_out(&self) -> bool {
        if !self.enabled {
            return false;
        }

        if let (Some(_), Some(sent)) = (&self.pending_ping, self.last_ping_sent) {
            sent.elapsed() > self.timeout
        } else {
            false
        }
    }

    /// Whether a PING is awaiting its ACK.
    pub fn has_pending(&self) -> bool {
        self.pending_ping.is_some()
    }
}

/// Default PingPong is disabled (no unsolicited PINGs).
impl Default for PingPong {
    fn default() -> Self {
        PingPong::disabled()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_disabled_never_pings() {
        let mut pp = PingPong::disabled();
        assert!(!pp.is_enabled());
        assert!(pp.maybe_send_ping().is_none());
        assert!(!pp.is_timed_out());
    }

    #[test]
    fn test_default_is_disabled() {
        let pp = PingPong::default();
        assert!(!pp.is_enabled());
    }

    #[test]
    fn test_enabled_sends_ping() {
        let mut pp = PingPong::new(Duration::from_millis(0), Duration::from_secs(10));
        assert!(pp.is_enabled());
        let ping = pp.maybe_send_ping();
        assert!(ping.is_some());
        assert!(pp.has_pending());
    }

    #[test]
    fn test_recv_pong_clears_pending() {
        let mut pp = PingPong::new(Duration::from_millis(0), Duration::from_secs(10));
        let ping = pp.maybe_send_ping().unwrap();
        assert!(pp.has_pending());
        assert!(pp.recv_pong(ping.opaque_data()));
        assert!(!pp.has_pending());
    }

    #[test]
    fn test_no_double_ping() {
        let mut pp = PingPong::new(Duration::from_millis(0), Duration::from_secs(10));
        pp.maybe_send_ping().unwrap();
        assert!(pp.maybe_send_ping().is_none());
    }
}
