//! Terminal UI for interacting with the embedded Odyssey orchestrator.

mod app;
mod client;
mod event;
mod event_bus;
mod ui;

use anyhow::{Context, bail};
use app::{App, PendingPermission, ViewerKind};
use autoagents_core::agent::prebuilt::executor::ReActAgent;
#[cfg(feature = "local")]
use autoagents_llamacpp::{LlamaCppProvider, ModelSource};
use autoagents_llm::LLMProvider;
use autoagents_llm::backends::openai::OpenAI;
use autoagents_llm::builder::LLMBuilder;
use clap::Parser;
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
use event_bus::EventBus;
use log::{debug, info, warn};
use odyssey_rs_config::OdysseyConfig;
use odyssey_rs_core::orchestrator::prompt::PromptProfile;
use odyssey_rs_core::skills::SkillStore;
use odyssey_rs_core::{
    AgentBuilder, DEFAULT_AGENT_ID, LLMEntry, OdysseyAgent, Orchestrator, PromptBuilder,
};
use odyssey_rs_memory::FileMemoryProvider;
use odyssey_rs_protocol::ApprovalDecision;
use odyssey_rs_sandbox::{BubblewrapProvider, LocalSandboxProvider, SandboxProvider};
use odyssey_rs_tools::builtin_tool_registry;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io::{self, Stdout};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use uuid::Uuid;

/// Supported slash commands in the TUI input box.
enum SlashCommand {
    New,
    Join(Uuid),
    Sessions,
    Skills,
    Models,
    Model(String),
}

/// Command-line options for the TUI client.
#[derive(Parser)]
#[command(name = "odyssey-rs-tui", version)]
struct Cli {
    /// Optional path to an odyssey.json5 config file
    #[arg(long)]
    config: Option<PathBuf>,
    /// OpenAI model name for the default agent
    #[arg(long)]
    model: Option<String>,
    /// Default agent id
    #[arg(long)]
    agent: Option<String>,
    /// Enable the local llama.cpp provider
    #[cfg(feature = "local")]
    #[arg(long)]
    local: bool,
    /// Local GGUF model path (mutually exclusive with --local-hf-repo)
    #[cfg(feature = "local")]
    #[arg(long)]
    local_gguf: Option<PathBuf>,
    /// HuggingFace repo id for a GGUF model
    #[cfg(feature = "local")]
    #[arg(long)]
    local_hf_repo: Option<String>,
    /// Optional HuggingFace GGUF filename
    #[cfg(feature = "local")]
    #[arg(long)]
    local_hf_filename: Option<String>,
    /// Optional HuggingFace mmproj filename
    #[cfg(feature = "local")]
    #[arg(long)]
    local_hf_mmproj: Option<String>,
    /// Optional chat template name or inline template
    #[cfg(feature = "local")]
    #[arg(long)]
    local_chat_template: Option<String>,
    /// Context size override
    #[cfg(feature = "local")]
    #[arg(long)]
    local_n_ctx: Option<u32>,
    /// Thread count override
    #[cfg(feature = "local")]
    #[arg(long)]
    local_n_threads: Option<i32>,
    /// Max tokens to generate
    #[cfg(feature = "local")]
    #[arg(long, default_value_t = 2048)]
    local_max_tokens: u32,
    /// Sampling temperature
    #[cfg(feature = "local")]
    #[arg(long)]
    local_temperature: Option<f32>,
    /// GPU layers to offload
    #[cfg(feature = "local")]
    #[arg(long)]
    local_n_gpu_layers: Option<u32>,
    /// Main GPU index
    #[cfg(feature = "local")]
    #[arg(long)]
    local_main_gpu: Option<i32>,
}

const DEFAULT_LLM_ID: &str = "default_LLM";
#[cfg(feature = "local")]
const LOCAL_LLM_ID: &str = "local-llama-cpp";

/// Entry point for the Odyssey TUI client.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = env_logger::builder()
        .format_timestamp_millis()
        .parse_default_env()
        .try_init();

    let cli = Cli::parse();
    info!(
        "starting TUI (config_set={}, model_set={}, default_agent_set={})",
        cli.config.is_some(),
        cli.model.is_some(),
        cli.agent.is_some()
    );
    let config = if let Some(path) = cli.config.as_ref() {
        info!("loading config from path: {}", path.display());
        OdysseyConfig::load_from_path(path).context("failed to load config")?
    } else {
        let cwd = std::env::current_dir().context("cwd")?;
        info!("loading layered config from cwd: {}", cwd.display());
        let layered = OdysseyConfig::load_layered(&cwd).context("failed to load layered config")?;
        debug!("layered config loaded (layers={})", layered.layers.len());
        layered.config
    };

    let local_enabled = local_enabled(&cli);
    let model_name = cli
        .model
        .as_ref()
        .cloned()
        .or_else(|| std::env::var("OPENAI_MODEL").ok())
        .unwrap_or_else(|| "gpt-5.2".to_string());
    let api_key = std::env::var("OPENAI_API_KEY").ok();
    let mut openai_llm: Option<Arc<dyn LLMProvider>> = None;
    if let Some(api_key) = api_key {
        info!("building default LLM provider (model={})", model_name);
        let llm: Arc<dyn LLMProvider> = LLMBuilder::<OpenAI>::new()
            .api_key(api_key)
            .model(model_name.clone())
            .build()
            .context("failed to build OpenAI LLM provider")?;
        openai_llm = Some(llm);
    } else if !local_enabled {
        bail!("OPENAI_API_KEY is required to run the TUI");
    }

    let tools = builtin_tool_registry();
    let events = EventBus::new(2048);
    let memory_root = config
        .memory
        .path
        .clone()
        .unwrap_or_else(|| ".odyssey/memory".to_string());
    let memory = Arc::new(
        FileMemoryProvider::new(PathBuf::from(memory_root))
            .context("failed to create memory provider")?,
    );
    let cwd = std::env::current_dir().context("failed to resolve current working directory")?;
    let sandbox: Option<Arc<dyn SandboxProvider>> = if std::env::consts::OS == "linux" {
        Some(Arc::new(BubblewrapProvider::new().unwrap()))
    } else {
        //Not yet supported
        Some(Arc::new(LocalSandboxProvider::default()))
    };
    let skill_store =
        Arc::new(SkillStore::load(&config.skills, &cwd).context("failed to load skills")?);
    let system_prompt = PromptBuilder::new(memory.clone(), Some(skill_store.clone()))
        .build_system_prompt("", &config.memory, PromptProfile::OrchestratorDefault)
        .await
        .context("failed to build system prompt")?;
    let orchestrator = Arc::new(Orchestrator::new(
        config,
        tools,
        sandbox,
        None,
        Some(skill_store.clone()),
        Some(Arc::new(events.clone())),
    )?);
    let mut openai_registered = false;
    if let Some(llm) = openai_llm.as_ref() {
        orchestrator.register_llm_provider(LLMEntry {
            id: DEFAULT_LLM_ID.to_string(),
            provider: llm.clone(),
        })?;
        openai_registered = true;
    }
    #[cfg(feature = "local")]
    let local_registration = if local_enabled {
        Some(register_local_llm(&cli, &orchestrator, openai_registered).await?)
    } else {
        None
    };
    let default_agent = AgentBuilder::new(
        DEFAULT_AGENT_ID.to_string(),
        ReActAgent::new(OdysseyAgent::new(system_prompt, Vec::new())),
        memory,
    );
    orchestrator.register_agent(default_agent)?;

    let client = Arc::new(OrchestratorClient::new(orchestrator, events));

    let mut app = App::new();
    app.model_id = DEFAULT_LLM_ID.to_string();
    #[cfg(not(feature = "local"))]
    let app_model = if openai_registered {
        format!("{DEFAULT_LLM_ID} ({model_name})")
    } else {
        DEFAULT_LLM_ID.to_string()
    };
    #[cfg(feature = "local")]
    let app_model = if openai_registered {
        format!("{DEFAULT_LLM_ID} ({model_name})")
    } else if let Some(local) = local_registration.as_ref() {
        format!("{DEFAULT_LLM_ID} ({})", local.label)
    } else {
        DEFAULT_LLM_ID.to_string()
    };
    app.model = app_model;
    if let Ok(agents) = client.list_agents().await {
        debug!("loaded agents (count={})", agents.len());
        app.set_agents(agents);
    } else {
        warn!("failed to load agents");
        app.push_status("failed to load agents");
    }
    if let Some(agent) = cli.agent.clone() {
        app.active_agent = Some(agent);
    }
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
    if let Ok(mut models) = client.list_models().await {
        debug!("loaded models (count={})", models.len());
        models.sort();
        app.set_models(models);
    } else {
        warn!("failed to load models");
        app.push_status("failed to load models");
    }
    app.set_user_name(resolve_user_name());
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

        let Some(event) = rx.recv().await else { break };
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
        return Ok(false);
    }

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
        let resolved = client
            .resolve_permission(permission.request_id, decision)
            .await
            .unwrap_or(false);
        app.pending_permissions.pop_front();
        if resolved {
            app.push_status("permission sent");
        } else {
            app.push_status("permission request not found");
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

fn local_enabled(cli: &Cli) -> bool {
    #[cfg(feature = "local")]
    {
        cli.local
    }
    #[cfg(not(feature = "local"))]
    {
        let _ = cli;
        false
    }
}

#[cfg(feature = "local")]
struct LocalLlmRegistration {
    label: String,
}

#[cfg(feature = "local")]
async fn register_local_llm(
    cli: &Cli,
    orchestrator: &Arc<Orchestrator>,
    openai_registered: bool,
) -> anyhow::Result<LocalLlmRegistration> {
    let source = resolve_local_model_source(cli)?;
    let label = local_label_from_source(&source);
    info!("building llama.cpp provider (source={label})");
    let mut builder = LlamaCppProvider::builder().model_source(source);
    if let Some(template) = cli.local_chat_template.as_ref() {
        builder = builder.chat_template(template.clone());
    }
    if let Some(n_ctx) = cli.local_n_ctx {
        builder = builder.n_ctx(n_ctx);
    }
    if let Some(n_threads) = cli.local_n_threads {
        builder = builder.n_threads(n_threads);
    }

    builder = builder.max_tokens(cli.local_max_tokens);

    if let Some(temperature) = cli.local_temperature {
        builder = builder.temperature(temperature);
    }
    if let Some(n_gpu_layers) = cli.local_n_gpu_layers {
        builder = builder.n_gpu_layers(n_gpu_layers);
    }
    if let Some(main_gpu) = cli.local_main_gpu {
        builder = builder.main_gpu(main_gpu);
    }
    let provider = builder
        .build()
        .await
        .context("failed to build llama.cpp provider")?;
    let provider: Arc<dyn LLMProvider> = Arc::new(provider);
    let llm_id = if openai_registered {
        LOCAL_LLM_ID.to_string()
    } else {
        DEFAULT_LLM_ID.to_string()
    };
    orchestrator.register_llm_provider(LLMEntry {
        id: llm_id.clone(),
        provider,
    })?;
    info!("registered llama.cpp provider (llm_id={llm_id})");
    Ok(LocalLlmRegistration { label })
}

#[cfg(feature = "local")]
fn resolve_local_model_source(cli: &Cli) -> anyhow::Result<ModelSource> {
    let has_gguf = cli.local_gguf.is_some();
    let has_hf = cli.local_hf_repo.is_some();
    if has_gguf && has_hf {
        bail!("use only one of --local-gguf or --local-hf-repo");
    }
    if let Some(path) = cli.local_gguf.as_ref() {
        return Ok(ModelSource::Gguf {
            model_path: path.display().to_string(),
        });
    }
    if let Some(repo_id) = cli.local_hf_repo.as_ref() {
        return Ok(ModelSource::HuggingFace {
            repo_id: repo_id.clone(),
            filename: cli.local_hf_filename.clone(),
            mmproj_filename: cli.local_hf_mmproj.clone(),
        });
    }
    Ok(ModelSource::HuggingFace {
        repo_id: "Qwen/Qwen2.5-Coder-7B-Instruct-GGUF".to_string(),
        filename: Some("qwen2.5-coder-7b-instruct-q8_0.gguf".to_string()),
        mmproj_filename: None,
    })
}

#[cfg(feature = "local")]
fn local_label_from_source(source: &ModelSource) -> String {
    match source {
        ModelSource::Gguf { model_path } => {
            let name = std::path::Path::new(model_path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(model_path);
            format!("gguf:{name}")
        }
        ModelSource::HuggingFace {
            repo_id,
            filename,
            mmproj_filename: _,
        } => filename
            .as_ref()
            .map(|file| format!("hf:{repo_id}/{file}"))
            .unwrap_or_else(|| format!("hf:{repo_id}")),
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
            if let Ok(true) = crossterm::event::poll(Duration::from_millis(30)) {
                while let Ok(true) = crossterm::event::poll(Duration::from_millis(0)) {
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
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
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
