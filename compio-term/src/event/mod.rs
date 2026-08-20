//! Asynchronous terminal events.
//!
//! [`EventStream`] uses the active Compio runtime. Linux uses Compio's managed
//! multishot read operation. Other Unix systems use the Compio polling driver
//! when stdin is a terminal. If stdin is redirected, they use Compio timers
//! with nonblocking rustix reads from `/dev/tty`. Windows uses a Compio event
//! wait and `ReadConsoleInputW`. No backend starts a helper thread.
//!
//! Event values and event-control commands are re-exported from Crossterm. Code
//! that already matches on [`Event`], [`KeyCode`], or [`MouseEvent`] can use
//! the same values with this stream.
//!
//! Terminal input is normally line buffered. Enable raw mode before reading
//! individual key events. Mouse, focus, bracketed-paste, and enhanced keyboard
//! events also require their corresponding Crossterm command to be enabled.

mod stream;
mod sys;

pub use crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
    EnableFocusChange, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState,
    KeyModifiers, KeyboardEnhancementFlags, MediaKeyCode, ModifierKeyCode, MouseButton, MouseEvent,
    MouseEventKind, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
pub use stream::EventStream;

#[derive(Debug)]
pub(super) enum InternalEvent {
    Event(Event),
    Ignored,
}
