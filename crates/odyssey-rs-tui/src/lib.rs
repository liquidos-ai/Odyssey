//! Library entry point for the Odyssey TUI.
//!
//! Provides a reusable [`run`] function that launches the Ratatui terminal UI
//! against a pre-configured [`Orchestrator`].

mod app;
mod client;
mod event;
mod event_bus;
mod ui;

pub use event_bus::EventBus;

use anyhow::anyhow;
use app::{App, PendingPermission, ViewerKind};
use client::OrchestratorClient;
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event as CrosstermEvent, KeyCode, KeyEvent,
    KeyModifiers, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use event::AppEvent;
use log::{debug, info, warn};
use odyssey_rs_core::Orchestrator;
use odyssey_rs_protocol::ApprovalDecision;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io::{self, Stdout};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use uuid::Uuid;

const ENV_USER: &str = "USER";
const ENV_USERNAME: &str = "USERNAME";

/// Supported slash commands in the TUI input box.
enum SlashCommand {
    New,
    Join(Uuid),
    Sessions,
    Skills,
    Models,
    Model(String),
}

/// Configuration for the Odyssey TUI session.
#[derive(Debug, Clone, Default)]
pub struct TuiConfig {
    /// Display label for the active model (shown in header).
    pub model_name: String,
    /// LLM provider ID registered in the orchestrator.
    pub model_id: String,
    /// Default agent ID to use when creating sessions.
    pub agent_id: Option<String>,
    /// Display name for the current user.
    pub user_name: Option<String>,
    /// Current working directory (shown in header).
    pub cwd: Option<std::path::PathBuf>,
}

/// Launch the Odyssey TUI against a pre-configured orchestrator.
///
/// The caller is responsible for:
/// - Creating the [`Orchestrator`] with `Some(Arc::new(events.clone()))` injected
/// - Registering all LLM providers and agents before calling `run`
/// - Initializing logging (e.g. `env_logger`) before calling `run`
///
/// # Errors
/// Returns an error if terminal setup, session creation, or the event loop fails.
pub async fn run(
    orchestrator: Arc<Orchestrator>,
    events: EventBus,
    config: TuiConfig,
) -> anyhow::Result<()> {
    let cwd = config
        .cwd
        .clone()
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| anyhow::anyhow!("cannot determine working directory"))?;

    let client = Arc::new(OrchestratorClient::new(orchestrator, events));

    let mut app = App::new();

    // Load and validate agents
    let agents = client.list_agents().await?;
    if agents.is_empty() {
        return Err(anyhow!("no agents registered in orchestrator"));
    }
    debug!("loaded agents (count={})", agents.len());
    app.set_agents(agents);

    if let Some(agent_id) = config.agent_id.clone() {
        app.active_agent = Some(agent_id);
    }

    // Load and validate models
    let mut models = client.list_models().await?;
    models.sort();
    if !models.contains(&config.model_id) {
        return Err(anyhow!(
            "model '{}' not registered; available: {:?}",
            config.model_id,
            models
        ));
    }
    debug!("loaded models (count={})", models.len());
    app.set_models(models);

    app.model_id.clone_from(&config.model_id);
    let app_model = if config.model_name.is_empty() {
        config.model_id.clone()
    } else {
        format!("{} ({})", config.model_id, config.model_name)
    };
    app.model = app_model;

    if let Ok(sessions) = client.list_sessions().await {
        debug!("loaded sessions (count={})", sessions.len());
        app.set_sessions(sessions);
    } else {
        warn!("failed to load sessions");
        app.push_status("failed to load sessions");
    }

    if let Ok(skills) = client.list_skills().await {
        debug!("loaded skills (count={})", skills.len());
        app.set_skills(skills);
    }

    let user_name = config.user_name.clone().unwrap_or_else(resolve_user_name);
    app.set_user_name(user_name);
    app.cwd = cwd.display().to_string();

    let mut terminal = setup_terminal()?;
    let (tx, mut rx) = mpsc::channel(256);
    spawn_input_handler(tx.clone());
    spawn_tick(tx.clone());

    let mut stream_handle: Option<JoinHandle<()>> = None;
    if app.active_session.is_none()
        && let Err(err) = create_session(&client, &mut app, tx.clone(), &mut stream_handle).await
    {
        app.push_status(format!("failed to create session: {err}"));
    }

    loop {
        terminal.draw(|frame| ui::draw(frame, &mut app))?;
        let event = rx
            .recv()
            .await
            .ok_or_else(|| anyhow!("event channel closed unexpectedly"))?;
        if handle_app_event(event, &client, &mut app, tx.clone(), &mut stream_handle).await? {
            break;
        }
    }

    restore_terminal(&mut terminal)?;
    Ok(())
}

/// Dispatch a UI event and return true when the app should exit.
async fn handle_app_event(
    event: AppEvent,
    client: &Arc<OrchestratorClient>,
    app: &mut App,
    sender: mpsc::Sender<AppEvent>,
    stream_handle: &mut Option<JoinHandle<()>>,
) -> anyhow::Result<bool> {
    match event {
        AppEvent::Input(key) => handle_input(key, client, app, sender, stream_handle).await,
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

/// Handle keyboard input while a viewer panel is open.
async fn handle_viewer_input(
    key: KeyEvent,
    kind: ViewerKind,
    client: &Arc<OrchestratorClient>,
    app: &mut App,
    sender: mpsc::Sender<AppEvent>,
    stream_handle: &mut Option<JoinHandle<()>>,
) -> anyhow::Result<bool> {
    match key.code {
        KeyCode::Up => match kind {
            ViewerKind::Sessions => {
                if app.selected_session > 0 {
                    app.selected_session -= 1;
                }
            }
            ViewerKind::Skills => app.viewer_scroll_up(1),
            ViewerKind::Models => {
                if app.selected_model > 0 {
                    app.selected_model -= 1;
                }
            }
        },
        KeyCode::Down => match kind {
            ViewerKind::Sessions => {
                if app.selected_session + 1 < app.sessions.len() {
                    app.selected_session += 1;
                }
            }
            ViewerKind::Skills => app.viewer_scroll_down(1),
            ViewerKind::Models => {
                if app.selected_model + 1 < app.models.len() {
                    app.selected_model += 1;
                }
            }
        },
        KeyCode::PageUp => app.viewer_scroll_up(5),
        KeyCode::PageDown => app.viewer_scroll_down(5),
        KeyCode::Home => app.viewer_scroll_up(u16::MAX),
        KeyCode::End => app.viewer_scroll_down(u16::MAX),
        KeyCode::Enter => {
            if matches!(kind, ViewerKind::Sessions) {
                activate_selected_session(client, app, sender, stream_handle).await?;
                app.close_viewer();
            } else if matches!(kind, ViewerKind::Models) {
                activate_selected_model(app)?;
                app.close_viewer();
            }
        }
        _ => {}
    }
    Ok(false)
}

/// Handle keyboard input in the default (non-viewer) state.
async fn handle_default_input(
    key: KeyEvent,
    client: &Arc<OrchestratorClient>,
    app: &mut App,
    sender: mpsc::Sender<AppEvent>,
    stream_handle: &mut Option<JoinHandle<()>>,
) -> anyhow::Result<bool> {
    match key.code {
        KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            create_session(client, app, sender, stream_handle).await?;
        }
        KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            refresh_sessions(client, app).await?;
        }
        KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            activate_selected_session(client, app, sender, stream_handle).await?;
        }
        KeyCode::PageUp => {
            app.scroll_up(5);
        }
        KeyCode::PageDown => {
            app.scroll_down(5);
        }
        KeyCode::Up => {
            app.scroll_up(1);
        }
        KeyCode::Down => {
            app.scroll_down(1);
        }
        KeyCode::Home => {
            app.scroll_to_top();
        }
        KeyCode::End => {
            app.enable_auto_scroll();
        }
        KeyCode::Enter => {
            if app.input.trim().is_empty() {
                app.show_slash_commands = false;
                return Ok(false);
            }
            if app.input.trim_start().starts_with('/') {
                let command = std::mem::take(&mut app.input);
                app.show_slash_commands = false;
                if let Err(err) =
                    handle_slash_command(client, app, sender, stream_handle, command).await
                {
                    app.push_system_message(err);
                }
            } else {
                app.show_slash_commands = false;
                send_message(client, app, sender.clone()).await?;
            }
        }
        KeyCode::Backspace => {
            app.input.pop();
            app.show_slash_commands = app.input.trim_start().starts_with('/');
        }
        KeyCode::Char(ch) => {
            if !key.modifiers.contains(KeyModifiers::CONTROL) {
                app.input.push(ch);
                app.show_slash_commands = app.input.trim_start().starts_with('/');
            }
        }
        _ => {}
    }

    Ok(false)
}

/// Handle keyboard input and dispatch actions.
async fn handle_input(
    key: KeyEvent,
    client: &Arc<OrchestratorClient>,
    app: &mut App,
    sender: mpsc::Sender<AppEvent>,
    stream_handle: &mut Option<JoinHandle<()>>,
) -> anyhow::Result<bool> {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Ok(true);
    }
    if key.code == KeyCode::Esc {
        if app.viewer.is_some() {
            app.close_viewer();
            return Ok(false);
        }
        if app.show_slash_commands {
            app.show_slash_commands = false;
            app.input.clear();
            return Ok(false);
        }
        return Ok(true);
    }

    if let Some(permission) = app.pending_permissions.front().cloned()
        && matches!(
            key.code,
            KeyCode::Char('y') | KeyCode::Char('a') | KeyCode::Char('n')
        )
    {
        return handle_permission_input(key, client, app, permission).await;
    }

    if let Some(kind) = app.viewer {
        return handle_viewer_input(key, kind, client, app, sender, stream_handle).await;
    }

    handle_default_input(key, client, app, sender, stream_handle).await
}

/// Handle keyboard input for a pending permission prompt.
async fn handle_permission_input(
    key: KeyEvent,
    client: &Arc<OrchestratorClient>,
    app: &mut App,
    permission: PendingPermission,
) -> anyhow::Result<bool> {
    let decision = match key.code {
        KeyCode::Char('y') => Some(ApprovalDecision::AllowOnce),
        KeyCode::Char('a') => Some(ApprovalDecision::AllowAlways),
        KeyCode::Char('n') => Some(ApprovalDecision::Deny),
        KeyCode::Esc => {
            app.pending_permissions.pop_front();
            return Ok(false);
        }
        _ => None,
    };
    if let Some(decision) = decision {
        info!(
            "sending permission decision (request_id={}, decision={:?})",
            permission.request_id, decision
        );
        match client
            .resolve_permission(permission.request_id, decision)
            .await
        {
            Ok(resolved) => {
                app.pending_permissions.pop_front();
                if resolved {
                    app.push_status("permission sent");
                } else {
                    app.push_status("permission request not found");
                }
            }
            Err(err) => {
                app.push_status(format!("failed to resolve permission: {err}"));
                app.pending_permissions.pop_front();
            }
        }
    }
    Ok(false)
}

/// Refresh the session list from the orchestrator.
async fn refresh_sessions(client: &Arc<OrchestratorClient>, app: &mut App) -> anyhow::Result<()> {
    debug!("refreshing sessions");
    let sessions = client.list_sessions().await?;
    app.set_sessions(sessions);
    Ok(())
}

/// Refresh the model list from the orchestrator.
async fn refresh_models(client: &Arc<OrchestratorClient>, app: &mut App) -> anyhow::Result<()> {
    debug!("refreshing models");
    let mut models = client.list_models().await?;
    models.sort();
    app.set_models(models);
    Ok(())
}

/// Create a new session and start streaming its events.
async fn create_session(
    client: &Arc<OrchestratorClient>,
    app: &mut App,
    sender: mpsc::Sender<AppEvent>,
    stream_handle: &mut Option<JoinHandle<()>>,
) -> anyhow::Result<()> {
    let agent_id = app
        .active_agent
        .clone()
        .or_else(|| app.agents.first().cloned());
    info!(
        "creating session (agent_id={})",
        agent_id.as_deref().unwrap_or("default")
    );
    let session_id = client.create_session(agent_id.clone()).await?;
    if let Some(agent_id) = agent_id {
        app.set_active_session(session_id, agent_id);
    } else if let Ok(session) = client.get_session(session_id).await {
        app.set_active_session(session.id, session.agent_id);
    }
    app.push_status("session created");
    spawn_stream(client.clone(), session_id, sender, stream_handle);
    Ok(())
}

/// Activate the selected session and load its messages.
async fn activate_selected_session(
    client: &Arc<OrchestratorClient>,
    app: &mut App,
    sender: mpsc::Sender<AppEvent>,
    stream_handle: &mut Option<JoinHandle<()>>,
) -> anyhow::Result<()> {
    if let Some(session) = app.sessions.get(app.selected_session).cloned() {
        let session_id = session.id;
        let agent_id = session.agent_id;
        info!("activating session (session_id={})", session_id);
        app.set_active_session(session_id, agent_id);
        if let Ok(session_detail) = client.get_session(session_id).await {
            app.load_messages(session_detail.messages);
        }
        app.push_status("session selected");
        spawn_stream(client.clone(), session_id, sender, stream_handle);
    }
    Ok(())
}

/// Activate the selected model for future runs.
fn activate_selected_model(app: &mut App) -> anyhow::Result<()> {
    if let Some(model_id) = app.models.get(app.selected_model).cloned() {
        app.set_active_model(model_id.clone());
        app.push_status(format!("model set: {model_id}"));
    } else {
        app.push_status("no models available");
    }
    Ok(())
}

/// Join a session by id and load its transcript.
async fn join_session(
    client: &Arc<OrchestratorClient>,
    app: &mut App,
    session_id: Uuid,
    sender: mpsc::Sender<AppEvent>,
    stream_handle: &mut Option<JoinHandle<()>>,
) -> anyhow::Result<()> {
    info!("joining session (session_id={})", session_id);
    let session = client.get_session(session_id).await?;
    app.set_active_session(session.id, session.agent_id);
    app.load_messages(session.messages);
    app.push_status("session joined");
    spawn_stream(client.clone(), session_id, sender, stream_handle);
    Ok(())
}

/// Handle slash commands entered in the input box.
async fn handle_slash_command(
    client: &Arc<OrchestratorClient>,
    app: &mut App,
    sender: mpsc::Sender<AppEvent>,
    stream_handle: &mut Option<JoinHandle<()>>,
    input: String,
) -> Result<(), String> {
    let command = parse_slash_command(&input)?;
    let Some(command) = command else {
        return Ok(());
    };
    debug!("handling slash command");
    match command {
        SlashCommand::New => create_session(client, app, sender, stream_handle)
            .await
            .map_err(|err| err.to_string())?,
        SlashCommand::Join(session_id) => {
            join_session(client, app, session_id, sender, stream_handle)
                .await
                .map_err(|err| err.to_string())?
        }
        SlashCommand::Sessions => {
            app.open_viewer(ViewerKind::Sessions);
        }
        SlashCommand::Skills => {
            app.open_viewer(ViewerKind::Skills);
        }
        SlashCommand::Models => {
            refresh_models(client, app)
                .await
                .map_err(|err| err.to_string())?;
            app.open_viewer(ViewerKind::Models);
        }
        SlashCommand::Model(model_id) => {
            set_model_by_id(client, app, model_id).await?;
        }
    }
    Ok(())
}

/// Parse a slash command from the input line.
fn parse_slash_command(input: &str) -> Result<Option<SlashCommand>, String> {
    let trimmed = input.trim();
    if !trimmed.starts_with('/') {
        return Ok(None);
    }
    let mut parts = trimmed.trim_start_matches('/').split_whitespace();
    let Some(command) = parts.next() else {
        return Ok(None);
    };
    match command.to_lowercase().as_str() {
        "new" => Ok(Some(SlashCommand::New)),
        "skills" => Ok(Some(SlashCommand::Skills)),
        "sessions" => Ok(Some(SlashCommand::Sessions)),
        "models" => Ok(Some(SlashCommand::Models)),
        "model" => match parts.next() {
            None => Ok(Some(SlashCommand::Models)),
            Some("list") => Ok(Some(SlashCommand::Models)),
            Some(id) => Ok(Some(SlashCommand::Model(id.to_string()))),
        },
        "join" => {
            let Some(id) = parts.next() else {
                return Err("usage: /join <session_id>".to_string());
            };
            let session_id = Uuid::parse_str(id).map_err(|_| "invalid session id".to_string())?;
            Ok(Some(SlashCommand::Join(session_id)))
        }
        "session" => match parts.next() {
            Some("new") => Ok(Some(SlashCommand::New)),
            Some("join") => {
                let Some(id) = parts.next() else {
                    return Err("usage: /session join <session_id>".to_string());
                };
                let session_id =
                    Uuid::parse_str(id).map_err(|_| "invalid session id".to_string())?;
                Ok(Some(SlashCommand::Join(session_id)))
            }
            Some("list") => Ok(Some(SlashCommand::Sessions)),
            Some("skills") => Ok(Some(SlashCommand::Skills)),
            Some(id) => {
                let session_id =
                    Uuid::parse_str(id).map_err(|_| "invalid session id".to_string())?;
                Ok(Some(SlashCommand::Join(session_id)))
            }
            None => Err("usage: /session <id>|new|join <id>".to_string()),
        },
        _ => Err(format!("unknown command: {command}")),
    }
}

async fn set_model_by_id(
    client: &Arc<OrchestratorClient>,
    app: &mut App,
    model_id: String,
) -> Result<(), String> {
    let mut models = client.list_models().await.map_err(|err| err.to_string())?;
    if models.is_empty() {
        return Err("no models registered".to_string());
    }
    models.sort();
    if !models.contains(&model_id) {
        return Err(format!("unknown model: {model_id}"));
    }
    app.set_models(models);
    app.set_active_model(model_id.clone());
    app.push_status(format!("model set: {model_id}"));
    Ok(())
}

/// Send a message to the active session.
async fn send_message(
    client: &Arc<OrchestratorClient>,
    app: &mut App,
    sender: mpsc::Sender<AppEvent>,
) -> anyhow::Result<()> {
    let session_id = match app.active_session {
        Some(id) => id,
        None => {
            app.push_status("no active session");
            return Ok(());
        }
    };
    let prompt = std::mem::take(&mut app.input);
    info!(
        "sending message (session_id={}, prompt_len={})",
        session_id,
        prompt.len()
    );
    app.push_user_message(prompt.clone());
    app.enable_auto_scroll();
    let agent_id = app.active_agent.clone();
    let llm_id = app.model_id.clone();
    app.push_status("running");
    spawn_send_message(client.clone(), session_id, prompt, agent_id, llm_id, sender);
    Ok(())
}

/// Spawn a task to stream events for a session.
fn spawn_stream(
    client: Arc<OrchestratorClient>,
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

/// Spawn a task to send a message asynchronously.
fn spawn_send_message(
    client: Arc<OrchestratorClient>,
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
        if let Err(err) = client
            .send_message(session_id, prompt, agent_id, llm_id)
            .await
        {
            let _ = sender
                .send(AppEvent::ActionError(format!("send message failed: {err}")))
                .await;
        }
    });
}

/// Spawn a task to poll for input events.
fn spawn_input_handler(sender: mpsc::Sender<AppEvent>) {
    tokio::spawn(async move {
        const MOUSE_SCROLL_LINES: i16 = 3;
        loop {
            if matches!(crossterm::event::poll(Duration::from_millis(30)), Ok(true)) {
                while matches!(crossterm::event::poll(Duration::from_millis(0)), Ok(true)) {
                    let event = match crossterm::event::read() {
                        Ok(event) => event,
                        Err(_) => break,
                    };
                    match event {
                        CrosstermEvent::Key(key) => {
                            let _ = sender.send(AppEvent::Input(key)).await;
                        }
                        CrosstermEvent::Mouse(mouse) => match mouse.kind {
                            MouseEventKind::ScrollUp => {
                                let lines = if mouse.modifiers.contains(KeyModifiers::SHIFT) {
                                    MOUSE_SCROLL_LINES.saturating_mul(2)
                                } else {
                                    MOUSE_SCROLL_LINES
                                };
                                let _ = sender.send(AppEvent::Scroll(-lines)).await;
                            }
                            MouseEventKind::ScrollDown => {
                                let lines = if mouse.modifiers.contains(KeyModifiers::SHIFT) {
                                    MOUSE_SCROLL_LINES.saturating_mul(2)
                                } else {
                                    MOUSE_SCROLL_LINES
                                };
                                let _ = sender.send(AppEvent::Scroll(lines)).await;
                            }
                            _ => {}
                        },
                        _ => {}
                    }
                }
            }
        }
    });
}

/// Spawn a periodic tick event generator.
fn spawn_tick(sender: mpsc::Sender<AppEvent>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(250));
        loop {
            interval.tick().await;
            let _ = sender.send(AppEvent::Tick).await;
        }
    });
}

fn resolve_user_name() -> String {
    std::env::var(ENV_USER)
        .or_else(|_| std::env::var(ENV_USERNAME))
        .unwrap_or_else(|_| "user".to_string())
}

/// Configure terminal in raw mode with alternate screen.
fn setup_terminal() -> anyhow::Result<Terminal<CrosstermBackend<Stdout>>> {
    debug!("setting up terminal");
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

/// Restore terminal state on exit.
fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> anyhow::Result<()> {
    debug!("restoring terminal");
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}
