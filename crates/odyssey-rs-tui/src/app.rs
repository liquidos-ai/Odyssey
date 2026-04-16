//! Application state for the Odyssey TUI.

use log::{debug, info};
use odyssey_rs_core::types::{Message, Role, SessionSummary};
use odyssey_rs_protocol::{
    ApprovalDecision, EventMsg, EventPayload, PermissionRequest, SkillSummary,
};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use std::cmp::min;
use std::collections::{HashSet, VecDeque};
use sysinfo::{Components, System};
use uuid::Uuid;

/// Chat roles displayed in the UI.
#[derive(Debug, Clone)]
pub enum ChatRole {
    /// User message.
    User,
    /// Assistant message.
    Assistant,
    /// System/status message.
    System,
    /// Permission prompt message.
    Permission,
}

/// Single chat entry rendered in the transcript.
#[derive(Debug, Clone)]
pub struct ChatEntry {
    /// Role that produced the message.
    pub role: ChatRole,
    /// Message content.
    pub content: String,
    /// Optional color for the message.
    pub color: Option<Color>,
}

/// Pending permission request displayed to the user.
#[derive(Debug, Clone)]
pub struct PendingPermission {
    /// Permission request id.
    pub request_id: Uuid,
    /// Summary text presented to the user.
    pub summary: String,
}

/// Top-level application state for the TUI.
pub struct App {
    /// List of available agent ids.
    pub agents: Vec<String>,
    /// List of sessions returned by the orchestrator.
    pub sessions: Vec<SessionSummary>,
    /// List of available skills.
    pub skills: Vec<SkillSummary>,
    /// List of available model ids.
    pub models: Vec<String>,
    /// Index of the selected session in the list.
    pub selected_session: usize,
    /// Index of the selected model in the list.
    pub selected_model: usize,
    /// Active session id.
    pub active_session: Option<Uuid>,
    /// Active agent id.
    pub active_agent: Option<String>,
    /// Current user name.
    pub user_name: String,
    /// Active model id used for LLM requests.
    pub model_id: String,
    /// Model name used by the default LLM.
    pub model: String,
    /// Current working directory.
    pub cwd: String,
    /// Chat transcript entries.
    pub messages: Vec<ChatEntry>,
    /// Current input buffer.
    pub input: String,
    /// Whether to show the slash command palette.
    pub show_slash_commands: bool,
    /// Status line text.
    pub status: String,
    /// Pending permission requests.
    pub pending_permissions: VecDeque<PendingPermission>,
    /// Current viewer mode, if any.
    pub viewer: Option<ViewerKind>,
    /// Current viewer scroll offset.
    pub viewer_scroll: u16,
    /// Maximum viewer scroll offset.
    pub viewer_max_scroll: u16,
    /// Current scroll offset.
    pub scroll: u16,
    /// Whether to auto-scroll to the bottom.
    pub auto_scroll: bool,
    /// Maximum scroll offset for the chat view.
    pub chat_max_scroll: u16,
    /// Current CPU usage percentage (0.0–100.0).
    pub cpu_usage: f32,
    /// Current GPU temperature (celsius), if available.
    pub gpu_temp: Option<f32>,
    sys: System,
    components: Components,
    streamed_turns: HashSet<Uuid>,
}

impl App {
    /// Create a new application state with defaults.
    pub fn new() -> Self {
        Self {
            agents: Vec::new(),
            sessions: Vec::new(),
            skills: Vec::new(),
            models: Vec::new(),
            selected_session: 0,
            selected_model: 0,
            active_session: None,
            active_agent: None,
            user_name: "user".to_string(),
            model_id: String::new(),
            model: String::new(),
            cwd: String::new(),
            messages: Vec::new(),
            input: String::new(),
            show_slash_commands: false,
            status: "idle".to_string(),
            pending_permissions: VecDeque::new(),
            viewer: None,
            viewer_scroll: 0,
            viewer_max_scroll: 0,
            scroll: 0,
            auto_scroll: true,
            chat_max_scroll: 0,
            cpu_usage: 0.0,
            gpu_temp: None,
            sys: System::new(),
            components: Components::new_with_refreshed_list(),
            streamed_turns: HashSet::new(),
        }
    }

    /// Update the list of available agents.
    pub fn set_agents(&mut self, agents: Vec<String>) {
        debug!("set agents (count={})", agents.len());
        self.agents = agents;
        if self.active_agent.is_none() {
            self.active_agent = self.agents.first().cloned();
        }
    }

    /// Update the list of sessions.
    pub fn set_sessions(&mut self, sessions: Vec<SessionSummary>) {
        debug!("set sessions (count={})", sessions.len());
        self.sessions = sessions;
        if self.selected_session >= self.sessions.len() {
            self.selected_session = self.sessions.len().saturating_sub(1);
        }
    }

    /// Update the list of skills.
    pub fn set_skills(&mut self, skills: Vec<SkillSummary>) {
        debug!("set skills (count={})", skills.len());
        self.skills = skills;
    }

    /// Update the list of available model ids.
    pub fn set_models(&mut self, models: Vec<String>) {
        debug!("set models (count={})", models.len());
        self.models = models;
        if self.models.is_empty() {
            self.selected_model = 0;
            return;
        }
        if let Some(idx) = self.models.iter().position(|id| id == &self.model_id) {
            self.selected_model = idx;
        } else {
            self.selected_model = 0;
            self.model_id = self.models[0].clone();
            self.model = self.model_id.clone();
        }
    }

    /// Switch active session and reset scroll state.
    pub fn set_active_session(&mut self, session_id: Uuid, agent_id: String) {
        info!("active session set (session_id={})", session_id);
        self.active_session = Some(session_id);
        self.active_agent = Some(agent_id);
        self.messages.clear();
        self.scroll = 0;
        self.auto_scroll = true;
        self.chat_max_scroll = 0;
        self.streamed_turns.clear();
        self.pending_permissions.clear();
    }

    /// Update the displayed user name.
    pub fn set_user_name(&mut self, user_name: String) {
        self.user_name = user_name;
    }

    /// Set the active model id used for future requests.
    pub fn set_active_model(&mut self, model_id: String) {
        self.model_id = model_id.clone();
        self.model = model_id;
        if let Some(idx) = self.models.iter().position(|id| id == &self.model_id) {
            self.selected_model = idx;
        }
    }

    /// Refresh CPU usage reading.
    pub fn refresh_cpu(&mut self) {
        self.sys.refresh_cpu_usage();
        let cpus = self.sys.cpus();
        if !cpus.is_empty() {
            let total: f32 = cpus.iter().map(|c| c.cpu_usage()).sum();
            self.cpu_usage = total / cpus.len() as f32;
        }
        self.components.refresh(false);
        self.gpu_temp = find_gpu_temp(&self.components);
    }

    /// Load an existing transcript into the chat view.
    pub fn load_messages(&mut self, messages: Vec<Message>) {
        debug!("loading messages (count={})", messages.len());
        self.messages = messages
            .into_iter()
            .map(|message| ChatEntry {
                role: chat_role_for(&message.role),
                content: message.content,
                color: None,
            })
            .collect();
        self.scroll = 0;
        self.auto_scroll = true;
        self.chat_max_scroll = 0;
        self.streamed_turns.clear();
    }

    /// Set the status line.
    pub fn push_status(&mut self, status: impl Into<String>) {
        self.status = status.into();
    }

    /// Append a user-authored message to the transcript.
    /// Always enables auto-scroll so the user sees their own message.
    pub fn push_user_message(&mut self, content: String) {
        self.messages.push(ChatEntry {
            role: ChatRole::User,
            content,
            color: None,
        });
        self.auto_scroll = true;
    }

    /// Append a system message to the transcript.
    pub fn push_system_message(&mut self, content: String) {
        self.messages.push(ChatEntry {
            role: ChatRole::System,
            content,
            color: None,
        });
        self.maybe_enable_auto_scroll();
    }

    /// Append a system message with a custom color.
    pub fn push_system_message_colored(&mut self, content: String, color: Color) {
        self.messages.push(ChatEntry {
            role: ChatRole::System,
            content,
            color: Some(color),
        });
        self.maybe_enable_auto_scroll();
    }

    /// Append a permission prompt message to the transcript.
    pub fn push_permission_message(&mut self, content: String) {
        self.messages.push(ChatEntry {
            role: ChatRole::Permission,
            content,
            color: Some(permission_color()),
        });
        self.maybe_enable_auto_scroll();
    }

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
                if !self.streamed_turns.remove(&turn_id) && !message.trim().is_empty() {
                    self.append_assistant_message(message);
                }
                self.status = "idle".to_string();
            }
            EventPayload::ToolCallStarted {
                tool_name,
                arguments,
                ..
            } => {
                debug!("tool call started (tool_name={})", tool_name);
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
                let command_line = command.join(" ");
                self.push_system_message_colored(
                    format!("exec: {command_line}"),
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
                    .retain(|permission| permission.request_id != request_id);
            }
            EventPayload::Error { message, .. } => {
                info!("error event received");
                self.push_system_message_colored(format!("error: {message}"), tool_error_color());
                self.status = "idle".to_string();
            }
            _ => {}
        }
    }

    /// Scroll the chat view upward by a number of lines.
    pub fn scroll_up(&mut self, lines: u16) {
        self.auto_scroll = false;
        self.scroll = self.scroll.saturating_sub(lines);
    }

    /// Scroll the chat view downward by a number of lines.
    pub fn scroll_down(&mut self, lines: u16) {
        self.scroll = min(self.scroll.saturating_add(lines), self.chat_max_scroll);
        if self.scroll >= self.chat_max_scroll {
            self.auto_scroll = true;
        }
    }

    /// Scroll to the top of the chat view.
    pub fn scroll_to_top(&mut self) {
        self.auto_scroll = false;
        self.scroll = 0;
    }

    /// Enable auto-scrolling to the bottom.
    pub fn enable_auto_scroll(&mut self) {
        self.auto_scroll = true;
        self.scroll = self.chat_max_scroll;
    }

    /// Update scroll bounds after layout changes.
    ///
    /// Only snaps to the new bottom when `auto_scroll` is on **or** the user
    /// was already pinned to the exact bottom before the update.  A strict
    /// equality check (`>=`) avoids pulling the user back down when they have
    /// scrolled even a single line upward.
    pub fn update_scroll_bounds(&mut self, max_scroll: u16) {
        let was_at_bottom = self.scroll >= self.chat_max_scroll;
        self.chat_max_scroll = max_scroll;
        if self.auto_scroll || was_at_bottom {
            self.scroll = max_scroll;
            self.auto_scroll = true;
        } else {
            self.scroll = self.scroll.min(max_scroll);
        }
    }

    /// If auto-scroll is already active, keep the scroll pinned to the
    /// current max.  Does **not** re-enable auto-scroll when the user has
    /// manually scrolled away.
    fn maybe_enable_auto_scroll(&mut self) {
        if self.auto_scroll {
            self.scroll = self.chat_max_scroll;
        }
    }

    /// Render chat messages into styled lines for the UI.
    pub fn render_lines(&self) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        if self.messages.is_empty() {
            lines.push(Line::from(Span::styled(
                " No messages yet. Type a message below to start.",
                Style::default().fg(Color::Rgb(128, 128, 128)),
            )));
            return lines;
        }

        for (idx, entry) in self.messages.iter().enumerate() {
            let (prefix, prefix_style) = match entry.role {
                ChatRole::User => (
                    " you ",
                    Style::default()
                        .fg(Color::Rgb(10, 10, 10))
                        .bg(Color::Rgb(107, 161, 230))
                        .add_modifier(ratatui::style::Modifier::BOLD),
                ),
                ChatRole::Assistant => (
                    " assistant ",
                    Style::default()
                        .fg(Color::Rgb(10, 10, 10))
                        .bg(Color::Rgb(238, 121, 72))
                        .add_modifier(ratatui::style::Modifier::BOLD),
                ),
                ChatRole::System => (
                    " system ",
                    Style::default()
                        .fg(Color::Rgb(10, 10, 10))
                        .bg(Color::Rgb(60, 60, 60))
                        .add_modifier(ratatui::style::Modifier::BOLD),
                ),
                ChatRole::Permission => (
                    " permission ",
                    Style::default()
                        .fg(Color::Rgb(10, 10, 10))
                        .bg(Color::Rgb(236, 91, 43))
                        .add_modifier(ratatui::style::Modifier::BOLD),
                ),
            };

            let content_style = match &entry.color {
                Some(color) => Style::default().fg(*color),
                None => match entry.role {
                    ChatRole::User => Style::default().fg(Color::Rgb(238, 238, 238)),
                    ChatRole::Assistant => Style::default().fg(Color::Rgb(238, 238, 238)),
                    ChatRole::System => Style::default().fg(Color::Rgb(128, 128, 128)),
                    ChatRole::Permission => Style::default().fg(Color::Rgb(236, 91, 43)),
                },
            };

            // Role badge line
            lines.push(Line::from(vec![Span::styled(prefix, prefix_style)]));

            // Content lines with left padding
            let mut content_lines = entry.content.lines();
            if let Some(first) = content_lines.next() {
                if !first.is_empty() {
                    lines.push(Line::from(Span::styled(format!(" {first}"), content_style)));
                }
                for line in content_lines {
                    lines.push(Line::from(Span::styled(format!(" {line}"), content_style)));
                }
            }

            // Add spacing between messages
            if idx + 1 < self.messages.len() {
                lines.push(Line::from(Span::raw("")));
            }
        }

        // Add trailing padding so the last message can always be scrolled
        // fully into view even if wrapped-line counting is slightly off.
        lines.push(Line::from(Span::raw("")));

        lines
    }

    /// Append a streamed assistant delta to the transcript.
    fn append_assistant_delta(&mut self, delta: String) {
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

    /// Append a full assistant message to the transcript.
    fn append_assistant_message(&mut self, message: String) {
        self.messages.push(ChatEntry {
            role: ChatRole::Assistant,
            content: message,
            color: None,
        });
        self.maybe_enable_auto_scroll();
    }

    /// Open a viewer overlay.
    pub fn open_viewer(&mut self, kind: ViewerKind) {
        self.viewer = Some(kind);
        self.viewer_scroll = 0;
        self.viewer_max_scroll = 0;
    }

    /// Close the viewer overlay.
    pub fn close_viewer(&mut self) {
        self.viewer = None;
        self.viewer_scroll = 0;
        self.viewer_max_scroll = 0;
    }

    /// Scroll viewer up by a number of lines.
    pub fn viewer_scroll_up(&mut self, lines: u16) {
        self.viewer_scroll = self.viewer_scroll.saturating_sub(lines);
    }

    /// Scroll viewer down by a number of lines.
    pub fn viewer_scroll_down(&mut self, lines: u16) {
        self.viewer_scroll = min(
            self.viewer_scroll.saturating_add(lines),
            self.viewer_max_scroll,
        );
    }

    /// Update viewer scroll bounds after layout changes.
    pub fn update_viewer_scroll_bounds(&mut self, max_scroll: u16) {
        self.viewer_max_scroll = max_scroll;
        self.viewer_scroll = self.viewer_scroll.min(max_scroll);
    }
}

/// Render a human-readable permission request summary.
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
            let command_line = argv.join(" ");
            format!("Command execution requested: {command_line}")
        }
    }
}

/// Map stored roles to chat roles.
fn chat_role_for(role: &Role) -> ChatRole {
    match role {
        Role::Assistant => ChatRole::Assistant,
        Role::User => ChatRole::User,
        Role::System => ChatRole::System,
    }
}

/// Viewer overlay types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewerKind {
    Sessions,
    Skills,
    Models,
}

fn permission_color() -> Color {
    Color::Rgb(255, 153, 51)
}

fn tool_start_color() -> Color {
    Color::Rgb(120, 190, 255)
}

fn tool_success_color() -> Color {
    Color::Rgb(120, 220, 140)
}

fn tool_error_color() -> Color {
    Color::Rgb(255, 110, 110)
}

fn exec_command_color() -> Color {
    Color::Rgb(160, 200, 255)
}

fn exec_output_color() -> Color {
    Color::Rgb(170, 170, 170)
}

fn approval_color(decision: ApprovalDecision) -> Color {
    match decision {
        ApprovalDecision::AllowOnce | ApprovalDecision::AllowAlways => tool_success_color(),
        ApprovalDecision::Deny => tool_error_color(),
    }
}

fn find_gpu_temp(components: &Components) -> Option<f32> {
    let mut best: Option<f32> = None;
    for component in components.list() {
        let label = component.label().to_lowercase();
        let id = component.id().map(|value| value.to_lowercase());
        let is_gpu = label.contains("gpu")
            || label.contains("amdgpu")
            || label.contains("nvidia")
            || label.contains("radeon")
            || id
                .as_deref()
                .is_some_and(|value| value.contains("gpu") || value == "tg0p");
        if !is_gpu {
            continue;
        }
        let Some(temp) = component.temperature() else {
            continue;
        };
        if !temp.is_finite() {
            continue;
        }
        best = Some(best.map_or(temp, |current| current.max(temp)));
    }
    best
}
