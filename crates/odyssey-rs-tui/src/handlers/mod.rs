//! Input and event handlers for the Odyssey TUI.

pub mod input;
pub mod model;
pub mod session;
pub mod slash;

use crate::app::App;
use crate::client::AgentRuntimeClient;
use crate::event::AppEvent;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Dispatch one application event.
///
/// Returns `true` when the event loop should exit.
pub async fn handle_app_event(
    event: AppEvent,
    client: &Arc<AgentRuntimeClient>,
    app: &mut App,
    sender: mpsc::Sender<AppEvent>,
    stream_handle: &mut Option<JoinHandle<()>>,
) -> anyhow::Result<bool> {
    match event {
        AppEvent::Input(key) => input::handle_input(key, client, app, sender, stream_handle).await,
        AppEvent::Server(event) => {
            let Some(active_session) = app.active_session else {
                return Ok(false);
            };
            if event.session_id != active_session {
                return Ok(false);
            }
            app.apply_event(event);
            Ok(false)
        }
        AppEvent::StreamError(message) => {
            app.push_system_message(format!("stream error: {message}"));
            Ok(false)
        }
        AppEvent::ActionError(message) => {
            app.push_system_message(message);
            app.push_status("idle");
            Ok(false)
        }
        AppEvent::Scroll(delta) => {
            if app.viewer.is_some() {
                if delta < 0 {
                    app.viewer_scroll_up((-delta) as u16);
                } else if delta > 0 {
                    app.viewer_scroll_down(delta as u16);
                }
            } else if delta < 0 {
                app.scroll_up((-delta) as u16);
            } else if delta > 0 {
                app.scroll_down(delta as u16);
            }
            Ok(false)
        }
        AppEvent::Tick => {
            app.refresh_cpu();
            Ok(false)
        }
    }
}
