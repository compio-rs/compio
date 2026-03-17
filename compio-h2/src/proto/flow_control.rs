/// HTTP/2 flow control window tracking.
///
/// Tracks the available window size for both connection-level and per-stream
/// flow control as defined in RFC 7540 Section 6.9.
#[derive(Debug, Clone)]
pub struct FlowControl {
    /// Current window size (can be negative per RFC 7540 Section 6.9.2).
    window_size: i32,
    /// Initial window size (used for new streams).
    initial_window_size: i32,
}

/// Default initial window size per RFC 7540 Section 6.9.2.
const DEFAULT_INITIAL_WINDOW_SIZE: i32 = 65_535;

impl FlowControl {
    /// Create a new flow control tracker with the given initial window size.
    pub fn new(initial_window_size: i32) -> Self {
        FlowControl {
            window_size: initial_window_size,
            initial_window_size,
        }
    }

    /// The current window size (may be negative per RFC 7540).
    pub fn window_size(&self) -> i32 {
        self.window_size
    }

    /// The initial window size this tracker was configured with.
    pub fn initial_window_size(&self) -> i32 {
        self.initial_window_size
    }

    /// Bytes available to send (0 if window is non-positive).
    pub fn available(&self) -> u32 {
        if self.window_size > 0 {
            self.window_size as u32
        } else {
            0
        }
    }

    /// Consume bytes from the window (when sending or receiving DATA).
    pub fn consume(&mut self, amount: u32) -> Result<(), FlowControlError> {
        let new_size = self
            .window_size
            .checked_sub(amount as i32)
            .ok_or(FlowControlError::WindowOverflow)?;
        self.window_size = new_size;
        Ok(())
    }

    /// Release bytes back to the window (when receiving WINDOW_UPDATE).
    pub fn release(&mut self, amount: u32) -> Result<(), FlowControlError> {
        let new_size = self
            .window_size
            .checked_add(amount as i32)
            .ok_or(FlowControlError::WindowOverflow)?;
        self.window_size = new_size;
        Ok(())
    }

    /// Update the initial window size (from SETTINGS).
    pub fn update_initial_window_size(&mut self, new_initial: i32) -> Result<(), FlowControlError> {
        let delta = new_initial - self.initial_window_size;
        let new_size = self
            .window_size
            .checked_add(delta)
            .ok_or(FlowControlError::WindowOverflow)?;
        self.window_size = new_size;
        self.initial_window_size = new_initial;
        Ok(())
    }

    /// Apply a window update (increase the window size).
    pub fn apply_window_update(&mut self, increment: u32) -> Result<(), FlowControlError> {
        self.release(increment)
    }
}

impl Default for FlowControl {
    fn default() -> Self {
        FlowControl::new(DEFAULT_INITIAL_WINDOW_SIZE)
    }
}

#[derive(Debug, thiserror::Error)]
/// Errors from flow control window arithmetic.
pub enum FlowControlError {
    /// The window size would overflow the i32 range.
    #[error("flow control window overflow")]
    WindowOverflow,
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW_UPDATE_THRESHOLD_RATIO: i32 = 2;

    impl FlowControl {
        fn is_available(&self) -> bool {
            self.window_size > 0
        }

        fn should_send_window_update(&self) -> Option<u32> {
            let consumed = self.initial_window_size - self.window_size;
            if consumed > 0 && consumed >= self.initial_window_size / WINDOW_UPDATE_THRESHOLD_RATIO
            {
                Some(consumed as u32)
            } else {
                None
            }
        }
    }

    #[test]
    fn test_default_window_size() {
        let fc = FlowControl::default();
        assert_eq!(fc.window_size(), 65_535);
        assert_eq!(fc.available(), 65_535);
        assert!(fc.is_available());
    }

    #[test]
    fn test_consume_and_release() {
        let mut fc = FlowControl::default();
        fc.consume(1000).unwrap();
        assert_eq!(fc.available(), 64_535);
        fc.release(500).unwrap();
        assert_eq!(fc.available(), 65_035);
    }

    #[test]
    fn test_consume_all() {
        let mut fc = FlowControl::new(100);
        fc.consume(100).unwrap();
        assert_eq!(fc.available(), 0);
        assert!(!fc.is_available());
    }

    #[test]
    fn test_window_update_threshold() {
        let mut fc = FlowControl::new(100);
        fc.consume(40).unwrap();
        assert!(fc.should_send_window_update().is_none());
        fc.consume(20).unwrap();
        let increment = fc.should_send_window_update().unwrap();
        assert_eq!(increment, 60);
    }

    #[test]
    fn test_overflow_protection() {
        let mut fc = FlowControl::new(i32::MAX);
        assert!(fc.release(1).is_err());
    }

    #[test]
    fn test_update_initial_window_size_increase() {
        let mut fc = FlowControl::new(65535);
        fc.consume(10000).unwrap(); // window = 55535
        fc.update_initial_window_size(131070).unwrap();
        // delta = 131070 - 65535 = 65535, so 55535 + 65535 = 121070
        assert_eq!(fc.window_size(), 121070);
        assert_eq!(fc.initial_window_size(), 131070);
    }

    #[test]
    fn test_update_initial_window_size_decrease() {
        let mut fc = FlowControl::new(65535);
        fc.update_initial_window_size(32768).unwrap();
        // delta = 32768 - 65535 = -32767, so 65535 - 32767 = 32768
        assert_eq!(fc.window_size(), 32768);
        assert_eq!(fc.initial_window_size(), 32768);
    }

    #[test]
    fn test_update_initial_window_size_negative_window() {
        // RFC 7540 §6.9.2: A change can cause a window to become negative
        let mut fc = FlowControl::new(65535);
        fc.consume(60000).unwrap(); // window = 5535
        fc.update_initial_window_size(1000).unwrap();
        // delta = 1000 - 65535 = -64535, so 5535 - 64535 = -59000
        assert_eq!(fc.window_size(), -59000);
        assert_eq!(fc.available(), 0); // negative window means no available capacity
        assert!(!fc.is_available());
    }

    #[test]
    fn test_update_initial_window_size_overflow() {
        let mut fc = FlowControl::new(i32::MAX);
        // Trying to increase from MAX should overflow
        assert!(fc.update_initial_window_size(i32::MAX).is_ok()); // same value, delta=0
        // But any increase would overflow
        let mut fc2 = FlowControl::new(i32::MAX - 1);
        fc2.release(1).unwrap(); // window = MAX
        assert!(fc2.update_initial_window_size(i32::MAX).is_err()); // delta=+1 overflows
    }

    #[test]
    fn test_negative_window_from_settings_then_recovery() {
        // RFC 7540 §6.9.2: SETTINGS can make a window negative,
        // then WINDOW_UPDATE can bring it back positive.
        let mut fc = FlowControl::new(65535);
        fc.consume(60000).unwrap(); // window = 5535

        // Settings decrease: delta = 1000 - 65535 = -64535
        fc.update_initial_window_size(1000).unwrap();
        assert_eq!(fc.window_size(), -59000);
        assert_eq!(fc.available(), 0);
        assert!(!fc.is_available());

        // Cannot send data while negative
        assert_eq!(fc.available(), 0);

        // WINDOW_UPDATE restores the window
        fc.apply_window_update(100000).unwrap();
        assert_eq!(fc.window_size(), 41000);
        assert!(fc.is_available());
        assert_eq!(fc.available(), 41000);
    }

    #[test]
    fn test_consume_below_zero_and_recovery() {
        // Edge case: consume exactly to zero, then get a window update
        let mut fc = FlowControl::new(100);
        fc.consume(100).unwrap();
        assert_eq!(fc.window_size(), 0);
        assert_eq!(fc.available(), 0);

        // WINDOW_UPDATE restores
        fc.apply_window_update(50).unwrap();
        assert_eq!(fc.window_size(), 50);
        assert_eq!(fc.available(), 50);
    }

    #[test]
    fn test_window_update_after_negative_window() {
        let mut fc = FlowControl::new(100);
        fc.consume(80).unwrap(); // window = 20

        // Settings drop to 10 (delta = -90), window = 20 - 90 = -70
        fc.update_initial_window_size(10).unwrap();
        assert_eq!(fc.window_size(), -70);

        // Small window update doesn't make it positive
        fc.apply_window_update(50).unwrap();
        assert_eq!(fc.window_size(), -20);
        assert!(!fc.is_available());

        // Another update makes it positive
        fc.apply_window_update(30).unwrap();
        assert_eq!(fc.window_size(), 10);
        assert!(fc.is_available());
    }

    #[test]
    fn test_should_send_window_update_not_after_settings_decrease() {
        let mut fc = FlowControl::new(65535);
        // Settings decrease to 32768 (delta = -32767, window = 32768)
        fc.update_initial_window_size(32768).unwrap();
        // Window is exactly at the new initial size — no consumed data
        // should_send_window_update compares consumed = initial - current = 32768 -
        // 32768 = 0
        assert!(fc.should_send_window_update().is_none());
    }
}
