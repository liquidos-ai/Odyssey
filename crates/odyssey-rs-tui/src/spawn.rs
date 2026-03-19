//! Async task spawners for streaming, message sending, input polling, and ticks.

use crate::client::AgentRuntimeClient;
use crate::event::AppEvent;
use crossterm::event::{Event as CrosstermEvent, KeyModifiers, MouseEventKind};
use log::debug;
use odyssey_rs_protocol::Task;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use uuid::Uuid;

/// Replace any existing stream task with a new one for `session_id`.
pub fn spawn_stream(
    client: Arc<AgentRuntimeClient>,
    session_id: Uuid,
    sender: mpsc::Sender<AppEvent>,
    stream_handle: &mut Option<JoinHandle<()>>,
) {
    if let Some(handle) = stream_handle.take() {
        handle.abort();
    }
    debug!("starting event stream (session_id={})", session_id);
    let handle = tokio::spawn(async move {
        if let Err(err) = client.stream_events(session_id, sender.clone()).await {
            let _ = sender.send(AppEvent::StreamError(err.to_string())).await;
        }
    });
    *stream_handle = Some(handle);
}

/// Spawn a background task to send a prompt to a session.
pub fn spawn_send_message(
    client: Arc<AgentRuntimeClient>,
    session_id: Uuid,
    prompt: String,
    agent_id: Option<String>,
    llm_id: String,
    sender: mpsc::Sender<AppEvent>,
) {
    let prompt_len = prompt.len();
    let agent_set = agent_id.is_some();
    tokio::spawn(async move {
        debug!(
            "dispatching send message (session_id={}, prompt_len={}, agent_set={})",
            session_id, prompt_len, agent_set
        );
        let task = Task::new(prompt);
        if let Err(err) = client
            .send_message(session_id, task, agent_id, llm_id)
            .await
        {
            let _ = sender
                .send(AppEvent::ActionError(format!("send message failed: {err}")))
                .await;
        }
    });
}

/// Spawn a task that polls crossterm for keyboard and mouse events.
pub fn spawn_input_handler(sender: mpsc::Sender<AppEvent>) {
    const MOUSE_SCROLL_LINES: i16 = 3;
    tokio::spawn(async move {
        loop {
            if crossterm::event::poll(Duration::from_millis(30)).unwrap_or(false) {
                // Drain all pending events before yielding
                while crossterm::event::poll(Duration::from_millis(0)).unwrap_or(false) {
                    let event = match crossterm::event::read() {
                        Ok(e) => e,
                        Err(_) => break,
                    };
                    match event {
                        CrosstermEvent::Key(key) => {
                            let _ = sender.send(AppEvent::Input(key)).await;
                        }
                        CrosstermEvent::Mouse(mouse) => {
                            let shift = mouse.modifiers.contains(KeyModifiers::SHIFT);
                            let lines = if shift {
                                MOUSE_SCROLL_LINES.saturating_mul(2)
                            } else {
                                MOUSE_SCROLL_LINES
                            };
                            match mouse.kind {
                                MouseEventKind::ScrollUp => {
                                    let _ = sender.send(AppEvent::Scroll(-lines)).await;
                                }
                                MouseEventKind::ScrollDown => {
                                    let _ = sender.send(AppEvent::Scroll(lines)).await;
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    });
}

/// Spawn a task that emits a [`AppEvent::Tick`] every 250 ms.
pub fn spawn_tick(sender: mpsc::Sender<AppEvent>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(250));
        loop {
            interval.tick().await;
            let _ = sender.send(AppEvent::Tick).await;
        }
    });
}
