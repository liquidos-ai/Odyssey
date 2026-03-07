//! Reusable library API for launching the Odyssey Ratatui client.
//!
//! This lets external binaries embed the TUI with an already configured
//! [`AgentRuntime`], while reusing the same event loop and handlers as the
//! built-in `odyssey-rs-tui` binary.

mod app;
mod client;
mod event;
mod event_bus;
mod handlers;
mod spawn;
mod terminal;
mod tui_config;
mod ui;

pub use event_bus::EventBus;

use anyhow::anyhow;
use app::App;
use client::AgentRuntimeClient;
use event::AppEvent;
use log::debug;
use odyssey_rs_core::AgentRuntime;
use odyssey_rs_core::McpStatus;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Configuration for a reusable TUI session.
#[derive(Debug, Clone, Default)]
pub struct TuiRunConfig {
    /// Human-readable model name shown in the header.
    pub model_name: String,
    /// LLM provider id registered in the runtime.
    pub model_id: String,
    /// Optional default agent id.
    pub agent_id: Option<String>,
    /// Optional user name shown in the header.
    pub user_name: Option<String>,
    /// Optional working directory shown in the header.
    pub cwd: Option<PathBuf>,
}

/// Launch the Odyssey TUI against a pre-configured [`AgentRuntime`].
///
/// The caller is responsible for:
/// - Injecting `events` as the runtime event sink
/// - Registering agents and LLM providers before calling `run`
pub async fn run(
    orchestrator: Arc<AgentRuntime>,
    events: EventBus,
    config: TuiRunConfig,
) -> anyhow::Result<()> {
    let cwd = config
        .cwd
        .clone()
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| anyhow!("cannot determine working directory"))?;

    let client = Arc::new(AgentRuntimeClient::new(orchestrator.clone(), events));
    let mut app = App::new();
    app.model_id = config.model_id.clone();
    app.model = if config.model_name.is_empty() {
        config.model_id.clone()
    } else {
        format!("{} ({})", config.model_id, config.model_name)
    };
    app.active_agent = config.agent_id.clone();
    app.mcp_status = orchestrator.mcp_status().clone();
    if let McpStatus::Failed(message) = &app.mcp_status {
        app.push_status(format!("mcp failed: {message}"));
    }

    // Apply persisted theme before first render.
    let persisted_tui_config = tui_config::TuiConfig::load();
    app.init_theme(&persisted_tui_config.theme);
    app.tui_config = persisted_tui_config;

    let agents = client.list_agents().await?;
    if agents.is_empty() {
        return Err(anyhow!("no agents registered in runtime"));
    }
    debug!("loaded agents (count={})", agents.len());
    app.set_agents(agents);
    if let Some(agent_id) = config.agent_id {
        app.active_agent = Some(agent_id);
    }

    if let Ok(sessions) = client.list_sessions().await {
        debug!("loaded sessions (count={})", sessions.len());
        app.set_sessions(sessions);
    } else {
        app.push_status("failed to load sessions");
    }

    if let Ok(skills) = client.list_skills().await {
        debug!("loaded skills (count={})", skills.len());
        app.set_skills(skills);
    }

    let mut models = client.list_models().await?;
    models.sort();
    if !models.contains(&config.model_id) {
        return Err(anyhow!(
            "model '{}' not registered; available: {:?}",
            config.model_id,
            models
        ));
    }
    app.set_models(models);
    app.model_id = config.model_id;

    app.set_user_name(config.user_name.unwrap_or_else(terminal::resolve_user_name));
    app.cwd = cwd.display().to_string();

    let mut terminal = terminal::setup_terminal()?;
    let (tx, mut rx) = mpsc::channel::<AppEvent>(256);
    spawn::spawn_input_handler(tx.clone());
    spawn::spawn_tick(tx.clone());

    let mut stream_handle: Option<JoinHandle<()>> = None;
    if app.active_session.is_none()
        && let Err(err) =
            handlers::session::create_session(&client, &mut app, tx.clone(), &mut stream_handle)
                .await
    {
        app.push_status(format!("failed to create session: {err}"));
    }

    loop {
        terminal.draw(|frame| ui::draw(frame, &mut app))?;
        let Some(event) = rx.recv().await else {
            break;
        };
        if handlers::handle_app_event(event, &client, &mut app, tx.clone(), &mut stream_handle)
            .await?
        {
            break;
        }
    }

    terminal::restore_terminal(&mut terminal)?;
    Ok(())
}
