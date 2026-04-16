//! Local event bus for embedding the orchestrator in the TUI.

use log::debug;
use odyssey_rs_core::EventSink;
use odyssey_rs_protocol::EventMsg;
use tokio::sync::broadcast;

/// Broadcast-backed event bus for the embedded orchestrator.
#[derive(Clone, Debug)]
pub struct EventBus {
    sender: broadcast::Sender<EventMsg>,
}

impl EventBus {
    /// Create a new event bus with the given channel buffer size.
    pub fn new(buffer: usize) -> Self {
        let (sender, _) = broadcast::channel(buffer);
        debug!("tui event bus initialized (buffer={})", buffer);
        Self { sender }
    }

    /// Subscribe to the event stream.
    pub fn subscribe(&self) -> broadcast::Receiver<EventMsg> {
        self.sender.subscribe()
    }
}

impl EventSink for EventBus {
    /// Emit an event into the broadcast channel.
    fn emit(&self, event: EventMsg) {
        let _ = self.sender.send(event);
    }
}
