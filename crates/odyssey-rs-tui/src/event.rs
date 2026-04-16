//! TUI event types for input and orchestration messages.

use crossterm::event::KeyEvent;
use odyssey_rs_protocol::EventMsg;

/// Application event emitted by input handlers or the server stream.
#[derive(Debug)]
pub enum AppEvent {
    /// Keyboard input event.
    Input(KeyEvent),
    /// Periodic tick event.
    Tick,
    /// Protocol event emitted by the embedded orchestrator.
    Server(EventMsg),
    /// Error from the streaming connection.
    StreamError(String),
    /// Error from an action request.
    ActionError(String),
    /// Scroll event in the chat view.
    Scroll(i16),
}
