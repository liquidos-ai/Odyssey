//! Protocol event handling: maps orchestrator events to App state mutations.

use crate::app::state::App;
use crate::app::types::PendingPermission;
use crate::app::types::{
    ChatEntry, ChatRole, approval_color, exec_command_color, exec_output_color, tool_error_color,
    tool_start_color, tool_success_color,
};
use log::{debug, info};
use odyssey_rs_protocol::{EventMsg, EventPayload, PermissionRequest};

impl App {
    /// Apply a protocol event to the application state.
    pub fn apply_event(&mut self, event: EventMsg) {
        match event.payload {
            EventPayload::AgentMessageDelta { turn_id, delta } => {
                debug!("agent delta (turn_id={})", turn_id);
                self.streamed_turns.insert(turn_id);
                self.append_assistant_delta(delta);
            }
            EventPayload::TurnCompleted { turn_id, message } => {
                info!("turn completed (turn_id={})", turn_id);
                if self.streamed_turns.remove(&turn_id) {
                    self.finalize_streamed_assistant_message(message);
                } else if !message.trim().is_empty() {
                    self.append_assistant_message(message);
                }
                self.status = "idle".to_string();
            }
            EventPayload::ToolCallStarted {
                turn_id,
                tool_name,
                arguments,
                ..
            } => {
                debug!("tool call started (tool_name={})", tool_name);
                self.discard_streamed_assistant_message(turn_id);
                self.push_system_message_colored(
                    format!("tool start: {tool_name} {arguments}"),
                    tool_start_color(),
                );
            }
            EventPayload::ToolCallFinished {
                tool_call_id,
                success,
                ..
            } => {
                debug!(
                    "tool call finished (tool_call_id={}, success={})",
                    tool_call_id, success
                );
                let label = if success { "ok" } else { "error" };
                let color = if success {
                    tool_success_color()
                } else {
                    tool_error_color()
                };
                self.push_system_message_colored(
                    format!("tool finished ({label}): {tool_call_id}"),
                    color,
                );
            }
            EventPayload::ExecCommandBegin { command, .. } => {
                debug!("exec command started (argv_len={})", command.len());
                self.push_system_message_colored(
                    format!("exec: {}", command.join(" ")),
                    exec_command_color(),
                );
            }
            EventPayload::ExecCommandOutputDelta { delta, .. } => {
                if !delta.trim().is_empty() {
                    self.push_system_message_colored(
                        format!("exec output: {delta}"),
                        exec_output_color(),
                    );
                }
            }
            EventPayload::ExecCommandEnd { .. } => {
                self.status = "idle".to_string();
            }
            EventPayload::PermissionRequested {
                request_id,
                request,
                ..
            } => {
                info!("permission requested (request_id={})", request_id);
                let summary = format_permission_request(&request);
                self.push_permission_message(format!(
                    "permission requested: {summary} (y=allow once, a=allow always, n=deny)"
                ));
                self.pending_permissions.push_back(PendingPermission {
                    request_id,
                    summary,
                });
                self.enable_auto_scroll();
            }
            EventPayload::ApprovalResolved {
                decision,
                request_id,
                ..
            } => {
                info!("permission resolved (decision={:?})", decision);
                self.push_system_message_colored(
                    format!("permission resolved: {decision:?}"),
                    approval_color(decision),
                );
                self.pending_permissions
                    .retain(|p| p.request_id != request_id);
            }
            EventPayload::Error { message, .. } => {
                info!("error event received");
                self.push_system_message_colored(format!("error: {message}"), tool_error_color());
                self.status = "idle".to_string();
            }
            _ => {}
        }
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    /// Append a streamed assistant token to the last assistant entry, or create one.
    pub(crate) fn append_assistant_delta(&mut self, delta: String) {
        if let Some(last) = self.messages.last_mut()
            && matches!(last.role, ChatRole::Assistant)
        {
            last.content.push_str(&delta);
            self.maybe_enable_auto_scroll();
            return;
        }
        self.messages.push(ChatEntry {
            role: ChatRole::Assistant,
            content: delta,
            color: None,
        });
        self.maybe_enable_auto_scroll();
    }

    /// Append a full assistant message (non-streaming path).
    pub(crate) fn append_assistant_message(&mut self, message: String) {
        self.messages.push(ChatEntry {
            role: ChatRole::Assistant,
            content: message,
            color: None,
        });
        self.maybe_enable_auto_scroll();
    }

    /// Discard a provisional streamed assistant message before a tool call runs.
    pub(crate) fn discard_streamed_assistant_message(&mut self, turn_id: uuid::Uuid) {
        if !self.streamed_turns.contains(&turn_id) {
            return;
        }
        if self
            .messages
            .last()
            .is_some_and(|last| matches!(last.role, ChatRole::Assistant))
        {
            self.messages.pop();
        }
    }

    /// Replace the provisional streamed assistant text with the final completion.
    pub(crate) fn finalize_streamed_assistant_message(&mut self, message: String) {
        if message.trim().is_empty() {
            return;
        }
        if let Some(last) = self.messages.last_mut()
            && matches!(last.role, ChatRole::Assistant)
        {
            last.content = message;
            self.maybe_enable_auto_scroll();
            return;
        }
        self.append_assistant_message(message);
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn format_permission_request(request: &PermissionRequest) -> String {
    match request {
        PermissionRequest::Tool { name } => format!("Tool usage requested: {name}"),
        PermissionRequest::Path { path, mode } => {
            format!("Path access requested: {path} ({mode:?})")
        }
        PermissionRequest::ExternalPath { path, mode } => {
            format!("External path access requested: {path} ({mode:?})")
        }
        PermissionRequest::Command { argv } => {
            format!("Command execution requested: {}", argv.join(" "))
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_assistant_delta_accumulates_into_last_entry() {
        let mut app = App::default();
        app.append_assistant_delta("Hello".into());
        app.append_assistant_delta(", world".into());
        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].content, "Hello, world");
    }

    #[test]
    fn append_assistant_delta_creates_new_entry_after_user() {
        let mut app = App::default();
        app.push_user_message("hi".into());
        app.append_assistant_delta("hey".into());
        assert_eq!(app.messages.len(), 2);
    }

    #[test]
    fn append_assistant_message_always_creates_new_entry() {
        let mut app = App::default();
        app.append_assistant_message("first".into());
        app.append_assistant_message("second".into());
        assert_eq!(app.messages.len(), 2);
    }

    #[test]
    fn discard_streamed_assistant_message_removes_draft() {
        let mut app = App::default();
        let turn_id = uuid::Uuid::new_v4();
        app.streamed_turns.insert(turn_id);
        app.append_assistant_delta("draft".into());

        app.discard_streamed_assistant_message(turn_id);

        assert!(app.messages.is_empty());
    }

    #[test]
    fn finalize_streamed_assistant_message_replaces_draft() {
        let mut app = App::default();
        app.append_assistant_delta("draft".into());

        app.finalize_streamed_assistant_message("final".into());

        assert_eq!(app.messages.len(), 1);
        assert_eq!(app.messages[0].content, "final");
    }
}
