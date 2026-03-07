//! Application state struct and its basic setters / accessors.

use crate::app::types::{ChatEntry, ChatRole, PendingPermission, ViewerKind};
use crate::tui_config::TuiConfig;
use crate::ui::theme::{AVAILABLE_THEMES, ODYSSEY, Theme};
use log::{debug, info};
use odyssey_rs_core::McpStatus;
use odyssey_rs_core::types::{Message, Role, SessionSummary};
use odyssey_rs_protocol::SkillSummary;
use std::collections::{HashSet, VecDeque};
use sysinfo::{Components, System};
use uuid::Uuid;

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
    /// Index of the selected session in the viewer list.
    pub selected_session: usize,
    /// Index of the selected model in the viewer list.
    pub selected_model: usize,
    /// Active session id.
    pub active_session: Option<Uuid>,
    /// Active agent id.
    pub active_agent: Option<String>,
    /// Current user name shown in the header.
    pub user_name: String,
    /// Active model id used for LLM requests.
    pub model_id: String,
    /// Human-readable model label shown in the header.
    pub model: String,
    /// Current working directory shown in the header.
    pub cwd: String,
    /// Chat transcript entries.
    pub messages: Vec<ChatEntry>,
    /// Current input buffer.
    pub input: String,
    /// Whether to show the slash command palette overlay.
    pub show_slash_commands: bool,
    /// Index of the highlighted command in the slash palette.
    pub slash_selected: usize,
    /// Active UI theme.
    pub theme: Theme,
    /// Index of the highlighted theme in the themes viewer.
    pub selected_theme: usize,
    /// Persistent TUI configuration (theme name, etc.).
    pub tui_config: TuiConfig,
    /// Status line text.
    pub status: String,
    /// MCP connection status observed during startup.
    pub mcp_status: McpStatus,
    /// Pending permission requests, shown as a queue in the header.
    pub pending_permissions: VecDeque<PendingPermission>,
    /// Current viewer mode, if any.
    pub viewer: Option<ViewerKind>,
    /// Current viewer scroll offset.
    pub viewer_scroll: u16,
    /// Maximum viewer scroll offset (updated every frame).
    pub viewer_max_scroll: u16,
    /// Current chat scroll offset.
    pub scroll: u16,
    /// Whether to auto-scroll to the bottom on new messages.
    pub auto_scroll: bool,
    /// Maximum chat scroll offset (updated every frame).
    pub chat_max_scroll: u16,
    /// Current CPU usage percentage (0.0–100.0).
    pub cpu_usage: f32,
    /// Current GPU temperature in °C, if a GPU sensor is available.
    pub gpu_temp: Option<f32>,
    pub(crate) sys: System,
    pub(crate) components: Components,
    pub(crate) streamed_turns: HashSet<Uuid>,
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
            slash_selected: 0,
            theme: ODYSSEY,
            selected_theme: 0,
            tui_config: TuiConfig::default(),
            status: "idle".to_string(),
            mcp_status: McpStatus::Disabled,
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

    // ── List setters ─────────────────────────────────────────────────────────

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
    ///
    /// Tries to preserve the currently selected model; falls back to the first
    /// entry if the active model is not in the new list.
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

    // ── Session / model / user setters ──────────────────────────────────────

    /// Switch active session and reset all session-scoped state.
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

    // ── Theme ─────────────────────────────────────────────────────────────────

    /// Apply the theme at `index` in `AVAILABLE_THEMES` and persist the choice.
    pub fn apply_theme_at(&mut self, index: usize) {
        if let Some(&t) = AVAILABLE_THEMES.get(index) {
            self.theme = t;
            self.selected_theme = index;
            self.persist_theme(t.name);
        }
    }

    /// Apply a theme by name (case-insensitive) and persist. Returns false if not found.
    pub fn apply_theme_by_name(&mut self, name: &str) -> bool {
        let name_lc = name.to_lowercase();
        if let Some((idx, &t)) = AVAILABLE_THEMES
            .iter()
            .enumerate()
            .find(|(_, t)| t.name == name_lc)
        {
            self.theme = t;
            self.selected_theme = idx;
            self.persist_theme(t.name);
            return true;
        }
        false
    }

    /// Apply a theme without persisting to disk (used at startup).
    pub fn init_theme(&mut self, name: &str) {
        let name_lc = name.to_lowercase();
        if let Some((idx, &t)) = AVAILABLE_THEMES
            .iter()
            .enumerate()
            .find(|(_, t)| t.name == name_lc)
        {
            self.theme = t;
            self.selected_theme = idx;
        }
    }

    fn persist_theme(&mut self, name: &'static str) {
        self.tui_config.theme = name.to_string();
        if let Err(e) = self.tui_config.save() {
            log::warn!("failed to save tui config: {e}");
        }
    }

    // ── Viewer ───────────────────────────────────────────────────────────────

    /// Open a viewer overlay and reset its scroll state.
    pub fn open_viewer(&mut self, kind: ViewerKind) {
        self.viewer = Some(kind);
        self.viewer_scroll = 0;
        self.viewer_max_scroll = 0;
    }

    /// Close the viewer overlay and reset its scroll state.
    pub fn close_viewer(&mut self) {
        self.viewer = None;
        self.viewer_scroll = 0;
        self.viewer_max_scroll = 0;
    }

    // ── Messages ─────────────────────────────────────────────────────────────

    /// Load an existing transcript into the chat view.
    pub fn load_messages(&mut self, messages: Vec<Message>) {
        debug!("loading messages (count={})", messages.len());
        self.messages = messages
            .into_iter()
            .map(|msg| ChatEntry {
                role: chat_role_for(&msg.role),
                content: msg.content,
                color: None,
            })
            .collect();
        self.scroll = 0;
        self.auto_scroll = true;
        self.chat_max_scroll = 0;
        self.streamed_turns.clear();
    }

    /// Set the status line text.
    pub fn push_status(&mut self, status: impl Into<String>) {
        self.status = status.into();
    }

    /// Append a user-authored message to the transcript and re-enable auto-scroll.
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

    /// Append a system message with a custom text color.
    pub fn push_system_message_colored(&mut self, content: String, color: ratatui::style::Color) {
        self.messages.push(ChatEntry {
            role: ChatRole::System,
            content,
            color: Some(color),
        });
        self.maybe_enable_auto_scroll();
    }

    /// Append a permission prompt message to the transcript.
    pub fn push_permission_message(&mut self, content: String) {
        use crate::app::types::permission_color;
        self.messages.push(ChatEntry {
            role: ChatRole::Permission,
            content,
            color: Some(permission_color()),
        });
        self.maybe_enable_auto_scroll();
    }

    // ── System metrics ───────────────────────────────────────────────────────

    /// Refresh CPU usage and GPU temperature readings.
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

    // ── Private helpers ───────────────────────────────────────────────────────

    /// Keep scroll pinned to the current bottom when auto-scroll is active.
    pub(crate) fn maybe_enable_auto_scroll(&mut self) {
        if self.auto_scroll {
            self.scroll = self.chat_max_scroll;
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn chat_role_for(role: &Role) -> ChatRole {
    match role {
        Role::Assistant => ChatRole::Assistant,
        Role::User => ChatRole::User,
        Role::System => ChatRole::System,
    }
}

fn find_gpu_temp(components: &Components) -> Option<f32> {
    let mut best: Option<f32> = None;
    for component in components.list() {
        let label = component.label().to_lowercase();
        let id = component.id().map(|v| v.to_lowercase());
        let is_gpu = label.contains("gpu")
            || label.contains("amdgpu")
            || label.contains("nvidia")
            || label.contains("radeon")
            || id
                .as_deref()
                .is_some_and(|v| v.contains("gpu") || v == "tg0p");
        if !is_gpu {
            continue;
        }
        let Some(temp) = component.temperature() else {
            continue;
        };
        if !temp.is_finite() {
            continue;
        }
        best = Some(best.map_or(temp, |cur| cur.max(temp)));
    }
    best
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_app() -> App {
        App::new()
    }

    #[test]
    fn default_state_is_idle() {
        let app = make_app();
        assert_eq!(app.status, "idle");
        assert!(app.active_session.is_none());
        assert!(app.messages.is_empty());
    }

    #[test]
    fn set_agents_picks_first_as_active() {
        let mut app = make_app();
        app.set_agents(vec!["alpha".into(), "beta".into()]);
        assert_eq!(app.agents.len(), 2);
        assert_eq!(app.active_agent.as_deref(), Some("alpha"));
    }

    #[test]
    fn set_agents_does_not_override_existing_active() {
        let mut app = make_app();
        app.active_agent = Some("beta".into());
        app.set_agents(vec!["alpha".into(), "beta".into()]);
        assert_eq!(app.active_agent.as_deref(), Some("beta"));
    }

    #[test]
    fn set_sessions_clamps_selection() {
        let mut app = make_app();
        app.selected_session = 5;
        app.set_sessions(vec![]);
        assert_eq!(app.selected_session, 0);
    }

    #[test]
    fn set_models_preserves_current_selection() {
        let mut app = make_app();
        app.model_id = "gpt-4".into();
        app.set_models(vec!["gpt-3.5".into(), "gpt-4".into()]);
        assert_eq!(app.selected_model, 1);
    }

    #[test]
    fn set_models_falls_back_to_first_when_not_found() {
        let mut app = make_app();
        app.model_id = "old-model".into();
        app.set_models(vec!["gpt-3.5".into(), "gpt-4".into()]);
        assert_eq!(app.selected_model, 0);
        assert_eq!(app.model_id, "gpt-3.5");
    }

    #[test]
    fn set_active_session_clears_messages_and_permissions() {
        let mut app = make_app();
        app.messages.push(ChatEntry {
            role: ChatRole::User,
            content: "hello".into(),
            color: None,
        });
        app.pending_permissions.push_back(PendingPermission {
            request_id: Uuid::new_v4(),
            summary: "test".into(),
        });
        let id = Uuid::new_v4();
        app.set_active_session(id, "agent-1".into());
        assert!(app.messages.is_empty());
        assert!(app.pending_permissions.is_empty());
        assert_eq!(app.active_session, Some(id));
    }

    #[test]
    fn set_active_model_updates_selection_index() {
        let mut app = make_app();
        app.set_models(vec!["a".into(), "b".into(), "c".into()]);
        app.set_active_model("c".into());
        assert_eq!(app.selected_model, 2);
        assert_eq!(app.model_id, "c");
    }

    #[test]
    fn push_user_message_enables_auto_scroll() {
        let mut app = make_app();
        app.auto_scroll = false;
        app.push_user_message("hello".into());
        assert!(app.auto_scroll);
        assert_eq!(app.messages.len(), 1);
    }

    #[test]
    fn push_status_updates_status_line() {
        let mut app = make_app();
        app.push_status("running");
        assert_eq!(app.status, "running");
    }

    #[test]
    fn open_close_viewer_resets_scroll() {
        let mut app = make_app();
        app.open_viewer(ViewerKind::Sessions);
        assert_eq!(app.viewer, Some(ViewerKind::Sessions));
        assert_eq!(app.viewer_scroll, 0);
        app.viewer_scroll = 10;
        app.close_viewer();
        assert!(app.viewer.is_none());
        assert_eq!(app.viewer_scroll, 0);
    }
}
