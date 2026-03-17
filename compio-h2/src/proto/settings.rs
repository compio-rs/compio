use crate::frame;

/// State machine for SETTINGS frame exchange.
#[derive(Debug, Clone, PartialEq)]
enum SettingsState {
    /// No pending SETTINGS ACK — free to send new SETTINGS.
    Synced,
    /// Waiting for the peer to ACK our last SETTINGS frame.
    WaitingAck,
}

/// Connection-level settings state, tracking local and remote settings.
#[derive(Debug, Clone)]
pub struct ConnSettings {
    /// Settings we have sent to the peer (pending ACK).
    local: SettingsValues,
    /// Settings the peer has sent to us (ACKed).
    remote: SettingsValues,
    /// Current state of the SETTINGS exchange.
    state: SettingsState,
    /// Queued local settings changes to send after the current ACK arrives.
    queued: Option<SettingsValues>,
}

/// Concrete settings values with defaults per RFC 7540 Section 6.5.2.
#[derive(Debug, Clone)]
pub struct SettingsValues {
    /// Maximum size of the HPACK dynamic table in bytes.
    pub header_table_size: u32,
    /// Whether server push is enabled.
    pub enable_push: bool,
    /// Maximum number of concurrent streams the peer may open.
    pub max_concurrent_streams: u32,
    /// Initial flow control window size for new streams.
    pub initial_window_size: u32,
    /// Maximum size of a single frame payload.
    pub max_frame_size: u32,
    /// Maximum size of the header list the peer will accept.
    pub max_header_list_size: u32,
}

impl Default for SettingsValues {
    fn default() -> Self {
        SettingsValues {
            header_table_size: 4096,
            enable_push: true,
            max_concurrent_streams: u32::MAX,
            initial_window_size: 65_535,
            max_frame_size: 16_384,
            max_header_list_size: u32::MAX,
        }
    }
}

impl ConnSettings {
    /// Create a new `ConnSettings` with RFC 7540 defaults.
    pub fn new() -> Self {
        ConnSettings {
            local: SettingsValues::default(),
            remote: SettingsValues::default(),
            state: SettingsState::Synced,
            queued: None,
        }
    }

    /// The local settings values.
    pub fn local(&self) -> &SettingsValues {
        &self.local
    }

    /// The remote (peer) settings values.
    pub fn remote(&self) -> &SettingsValues {
        &self.remote
    }

    /// Whether we are waiting for the peer to ACK our SETTINGS.
    pub fn is_pending_ack(&self) -> bool {
        self.state == SettingsState::WaitingAck
    }

    /// Build a SETTINGS frame from our local settings to send to the peer.
    ///
    /// `Some(frame)` if a SETTINGS frame should be sent now,
    /// or `None` if we are already waiting for an ACK (changes are queued).
    pub fn build_local_settings(&mut self) -> Option<frame::Settings> {
        match self.state {
            SettingsState::Synced => {
                self.state = SettingsState::WaitingAck;
                Some(self.build_settings_frame(&self.local.clone()))
            }
            SettingsState::WaitingAck => {
                // Queue the current local settings for sending after ACK
                self.queued = Some(self.local.clone());
                None
            }
        }
    }

    /// Build a SETTINGS frame unconditionally (used for initial connection
    /// preface).
    ///
    /// This bypasses the state machine since the initial SETTINGS must always
    /// be sent.
    pub fn build_initial_settings(&mut self) -> frame::Settings {
        self.state = SettingsState::WaitingAck;
        self.build_settings_frame(&self.local.clone())
    }

    /// Build a `frame::Settings` from the given values.
    ///
    /// `max_concurrent_streams` is only included when explicitly limited
    /// (not `u32::MAX`). Per RFC 7540 §6.5.2, omitting it means "no limit".
    fn build_settings_frame(&self, values: &SettingsValues) -> frame::Settings {
        let mut s = frame::Settings::new();
        s.set_header_table_size(values.header_table_size);
        s.set_initial_window_size(values.initial_window_size);
        s.set_max_frame_size(values.max_frame_size);
        if values.max_concurrent_streams != u32::MAX {
            s.set_max_concurrent_streams(values.max_concurrent_streams);
        }
        if values.max_header_list_size != u32::MAX {
            s.set_max_header_list_size(values.max_header_list_size);
        }
        s
    }

    /// Apply an incoming SETTINGS frame from the peer.
    pub fn apply_remote(&mut self, settings: &frame::Settings) {
        if let Some(v) = settings.header_table_size() {
            self.remote.header_table_size = v;
        }
        if let Some(v) = settings.enable_push() {
            self.remote.enable_push = v;
        }
        if let Some(v) = settings.max_concurrent_streams() {
            self.remote.max_concurrent_streams = v;
        }
        if let Some(v) = settings.initial_window_size() {
            self.remote.initial_window_size = v;
        }
        if let Some(v) = settings.max_frame_size() {
            self.remote.max_frame_size = v;
        }
        if let Some(v) = settings.max_header_list_size() {
            self.remote.max_header_list_size = v;
        }
    }

    /// Handle an ACK for our SETTINGS from the peer.
    ///
    /// If queued changes exist, returns `Some(frame)` — the caller should send
    /// this new SETTINGS frame immediately.
    pub fn recv_ack(&mut self) -> Option<frame::Settings> {
        if let Some(queued) = self.queued.take() {
            // Stay in WaitingAck, send the queued settings
            let frame = self.build_settings_frame(&queued);
            Some(frame)
        } else {
            self.state = SettingsState::Synced;
            None
        }
    }

    /// Set the local initial window size for new streams.
    pub fn set_local_initial_window_size(&mut self, size: u32) {
        self.local.initial_window_size = size;
    }

    /// Set the local maximum concurrent streams limit.
    pub fn set_local_max_concurrent_streams(&mut self, max: u32) {
        self.local.max_concurrent_streams = max;
    }

    /// Set the local maximum frame payload size.
    pub fn set_local_max_frame_size(&mut self, size: u32) {
        self.local.max_frame_size = size;
    }

    /// Set the local maximum header list size.
    pub fn set_local_max_header_list_size(&mut self, size: u32) {
        self.local.max_header_list_size = size;
    }

    /// Set the local HPACK dynamic table size.
    pub fn set_local_header_table_size(&mut self, size: u32) {
        self.local.header_table_size = size;
    }

    /// Set whether server push is enabled locally.
    pub fn set_local_enable_push(&mut self, enabled: bool) {
        self.local.enable_push = enabled;
    }

    /// Whether we can send a new SETTINGS frame (not waiting for ACK).
    pub fn can_send_settings(&self) -> bool {
        self.state == SettingsState::Synced
    }
}

impl Default for ConnSettings {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_state_is_synced() {
        let settings = ConnSettings::new();
        assert!(settings.can_send_settings());
        assert!(!settings.is_pending_ack());
    }

    #[test]
    fn test_build_initial_settings_transitions_to_waiting() {
        let mut settings = ConnSettings::new();
        let _frame = settings.build_initial_settings();
        assert!(settings.is_pending_ack());
        assert!(!settings.can_send_settings());
    }

    #[test]
    fn test_build_local_settings_synced_returns_frame() {
        let mut settings = ConnSettings::new();
        let frame = settings.build_local_settings();
        assert!(frame.is_some());
        assert!(settings.is_pending_ack());
    }

    #[test]
    fn test_build_local_settings_waiting_queues() {
        let mut settings = ConnSettings::new();
        let _first = settings.build_local_settings();
        assert!(settings.is_pending_ack());

        // Second call while waiting should queue and return None
        let second = settings.build_local_settings();
        assert!(second.is_none());
        assert!(settings.queued.is_some());
    }

    #[test]
    fn test_recv_ack_no_queued_transitions_to_synced() {
        let mut settings = ConnSettings::new();
        let _frame = settings.build_initial_settings();
        assert!(settings.is_pending_ack());

        let queued_frame = settings.recv_ack();
        assert!(queued_frame.is_none());
        assert!(settings.can_send_settings());
        assert!(!settings.is_pending_ack());
    }

    #[test]
    fn test_recv_ack_with_queued_stays_waiting_returns_frame() {
        let mut settings = ConnSettings::new();
        let _first = settings.build_initial_settings();

        // Queue a change
        settings.set_local_initial_window_size(32_768);
        let _second = settings.build_local_settings();
        assert!(settings.queued.is_some());

        // ACK arrives: should return queued frame, stay WaitingAck
        let queued_frame = settings.recv_ack();
        assert!(queued_frame.is_some());
        assert!(settings.is_pending_ack());
        assert!(settings.queued.is_none());

        // Second ACK: now transitions to Synced
        let none = settings.recv_ack();
        assert!(none.is_none());
        assert!(settings.can_send_settings());
    }

    #[test]
    fn test_apply_remote_updates_remote_values() {
        let mut settings = ConnSettings::new();
        let mut frame = frame::Settings::new();
        frame.set_max_concurrent_streams(100);
        frame.set_initial_window_size(32_768);

        settings.apply_remote(&frame);
        assert_eq!(settings.remote().max_concurrent_streams, 100);
        assert_eq!(settings.remote().initial_window_size, 32_768);
    }
}
