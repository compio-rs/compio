/// Connection state machine and message types.
pub(crate) mod connection;
/// Flow control window tracking.
pub(crate) mod flow_control;
/// PING/PONG keepalive mechanism.
pub(crate) mod ping_pong;
/// Connection settings negotiation.
pub(crate) mod settings;
/// Per-stream state and stream store.
pub(crate) mod streams;
