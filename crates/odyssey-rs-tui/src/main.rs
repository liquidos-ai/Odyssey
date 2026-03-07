//! Terminal UI for interacting with the embedded Odyssey orchestrator.

mod app;
mod cli;
mod client;
mod event;
mod event_bus;
mod handlers;
mod llm;
mod spawn;
mod terminal;
mod tui_config;
mod ui;

use anyhow::{Context, bail};
use app::App;
use autoagents_core::agent::prebuilt::executor::ReActAgent;
use clap::Parser;
use cli::{Cli, local_enabled};
use client::AgentRuntimeClient;
use event::AppEvent;
use event_bus::EventBus;
use llm::{DEFAULT_LLM_ID, build_openai_provider, register_llm};
use log::{debug, info, warn};
use odyssey_rs_config::OdysseyConfig;
use odyssey_rs_core::agent_runtime::prompt::{PromptBuilder, PromptProfile};
use odyssey_rs_core::memory::FileMemoryProvider;
use odyssey_rs_core::skills::SkillStore;
use odyssey_rs_core::{AgentBuilder, AgentRuntimeBuilder, DEFAULT_AGENT_ID, OdysseyAgent};
#[cfg(target_os = "linux")]
use odyssey_rs_sandbox::BubblewrapProvider;
#[cfg(not(target_os = "linux"))]
use odyssey_rs_sandbox::LocalSandboxProvider;
use odyssey_rs_sandbox::SandboxProvider;
use odyssey_rs_tools::builtin_tool_registry;
use spawn::{spawn_input_handler, spawn_tick};
use std::fs::{File, OpenOptions, create_dir_all};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tui_config::TuiConfig;

/// Entry point for the Odyssey TUI client.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_logging()?;

    let cli = Cli::parse();
    info!(
        "starting TUI (config_set={}, model_set={}, default_agent_set={})",
        cli.config.is_some(),
        cli.model.is_some(),
        cli.agent.is_some()
    );

    let config = load_config(&cli)?;
    let (client, app) = build_orchestrator_and_app(&cli, config).await?;

    run(cli, client, app).await
}

fn init_logging() -> anyhow::Result<()> {
    let log_path = resolve_persistent_log_path()?;
    if let Some(parent) = log_path.parent() {
        create_dir_all(parent)
            .with_context(|| format!("failed to create log directory at {}", parent.display()))?;
    }
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("failed to open log file {}", log_path.display()))?;
    let tee = TeeWriter::new(file);
    let _ = env_logger::builder()
        .format_timestamp_millis()
        .parse_default_env()
        .target(env_logger::Target::Pipe(Box::new(tee)))
        .try_init();
    Ok(())
}

fn resolve_persistent_log_path() -> anyhow::Result<PathBuf> {
    let cwd = std::env::current_dir().context("failed to resolve current working directory")?;
    Ok(cwd
        .join(".odyssey")
        .join("sessions")
        .join("errors")
        .join("core-errors.log"))
}

struct TeeWriter {
    file: Mutex<File>,
}

impl TeeWriter {
    fn new(file: File) -> Self {
        Self {
            file: Mutex::new(file),
        }
    }
}

impl Write for TeeWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        std::io::stdout().write_all(buf)?;
        if let Ok(mut file) = self.file.lock() {
            file.write_all(buf)?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        std::io::stdout().flush()?;
        if let Ok(mut file) = self.file.lock() {
            file.flush()?;
        }
        Ok(())
    }
}

// ── AgentRuntime bootstrap ────────────────────────────────────────────────────

/// Load the `OdysseyConfig` from the CLI flag or via layered discovery.
fn load_config(cli: &Cli) -> anyhow::Result<OdysseyConfig> {
    if let Some(path) = cli.config.as_ref() {
        info!("loading config from path: {}", path.display());
        OdysseyConfig::load_from_path(path).context("failed to load config")
    } else {
        let cwd = std::env::current_dir().context("cwd")?;
        info!("loading layered config from cwd: {}", cwd.display());
        let layered = OdysseyConfig::load_layered(&cwd).context("failed to load layered config")?;
        debug!("layered config loaded (layers={})", layered.layers.len());
        Ok(layered.config)
    }
}

async fn build_orchestrator_and_app(
    cli: &Cli,
    config: OdysseyConfig,
) -> anyhow::Result<(Arc<AgentRuntimeClient>, App)> {
    let use_local = local_enabled(cli);

    // ── LLM providers ──────────────────────────────────────────────────────
    let model_name = cli
        .model
        .as_ref()
        .cloned()
        .or_else(|| std::env::var("OPENAI_MODEL").ok())
        .unwrap_or_else(|| "gpt-5.2".to_string());

    let api_key = std::env::var("OPENAI_API_KEY").ok();
    let openai_provider = if let Some(key) = api_key {
        Some(build_openai_provider(&model_name, key)?)
    } else {
        None
    };

    if openai_provider.is_none() && !use_local {
        bail!("OPENAI_API_KEY is required to run the TUI");
    }

    // ── Core services ──────────────────────────────────────────────────────
    let tools = builtin_tool_registry();
    let events = EventBus::new(2048);
    let cwd = std::env::current_dir().context("failed to resolve current working directory")?;
    let skill_store =
        Arc::new(SkillStore::load(&config.skills, &cwd).context("failed to load skills")?);
    let memory = Arc::new(
        FileMemoryProvider::new(resolve_tui_memory_root(&cwd, &config, DEFAULT_AGENT_ID))
            .context("failed to create memory provider")?,
    );
    let system_prompt = PromptBuilder::new(memory, Some(skill_store.clone()))
        .build_system_prompt("", &config.memory, PromptProfile::OrchestratorDefault)
        .await
        .context("failed to build Odyssey system prompt")?;
    let odyssey_agent = AgentBuilder::new(
        DEFAULT_AGENT_ID.to_string(),
        ReActAgent::new(OdysseyAgent::new(system_prompt, Vec::new())),
    )
    .with_description("Odyssey runtime agent");

    let sandbox: Arc<dyn SandboxProvider> = {
        #[cfg(target_os = "linux")]
        {
            Arc::new(BubblewrapProvider::new().context("failed to init bubblewrap provider")?)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Arc::new(LocalSandboxProvider::default())
        }
    };

    let orchestrator = Arc::new(
        AgentRuntimeBuilder::new(config, tools)
            .with_sandbox_provider(sandbox)
            .with_skill_store(skill_store)
            .with_event_sink(Arc::new(events.clone()))
            .register_agent(odyssey_agent)
            .build()
            .await?,
    );
    orchestrator.set_default_agent_id(DEFAULT_AGENT_ID)?;

    // ── Register LLM providers ─────────────────────────────────────────────
    let mut openai_registered = false;
    if let Some(provider) = openai_provider {
        register_llm(&orchestrator, DEFAULT_LLM_ID, provider)?;
        openai_registered = true;
    }

    #[cfg(feature = "local")]
    let local_registration = if use_local {
        Some(llm::register_local_llm(cli, &orchestrator, openai_registered).await?)
    } else {
        None
    };

    // ── Build initial App state ────────────────────────────────────────────
    let client = Arc::new(AgentRuntimeClient::new(orchestrator.clone(), events));

    let app_model = {
        #[cfg(not(feature = "local"))]
        {
            if openai_registered {
                format!("{DEFAULT_LLM_ID} ({model_name})")
            } else {
                DEFAULT_LLM_ID.to_string()
            }
        }
        #[cfg(feature = "local")]
        {
            if openai_registered {
                format!("{DEFAULT_LLM_ID} ({model_name})")
            } else if let Some(local) = local_registration.as_ref() {
                format!("{DEFAULT_LLM_ID} ({})", local.label)
            } else {
                DEFAULT_LLM_ID.to_string()
            }
        }
    };

    let mut app = App::new();
    app.model_id = DEFAULT_LLM_ID.to_string();
    app.model = app_model;
    app.active_agent = Some(DEFAULT_AGENT_ID.to_string());
    app.mcp_status = orchestrator.mcp_status().clone();
    if let odyssey_rs_core::McpStatus::Failed(message) = &app.mcp_status {
        app.push_status(format!("mcp failed: {message}"));
    }

    // Apply the persisted theme before first render (no save round-trip).
    let tui_config = TuiConfig::load();
    app.init_theme(&tui_config.theme.clone());
    app.tui_config = tui_config;

    populate_app(&client, &mut app, cli, &cwd).await;

    Ok((client, app))
}

fn resolve_tui_memory_root(cwd: &Path, config: &OdysseyConfig, agent_id: &str) -> PathBuf {
    if let Some(path) = config.memory.path.as_ref() {
        let path = PathBuf::from(path);
        if path.is_absolute() {
            return path;
        }
        return cwd.join(path);
    }
    cwd.join(".odyssey").join("memory").join(agent_id)
}

/// Populate the initial app state from the orchestrator (agents, sessions, etc.).
async fn populate_app(
    client: &Arc<AgentRuntimeClient>,
    app: &mut App,
    cli: &Cli,
    cwd: &std::path::Path,
) {
    if let Ok(mut agents) = client.list_agents().await {
        debug!("loaded agents (count={})", agents.len());
        agents.sort();
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

    app.set_user_name(terminal::resolve_user_name());
    app.cwd = cwd.display().to_string();
}

// ── Main event loop ───────────────────────────────────────────────────────────

async fn run(cli: Cli, client: Arc<AgentRuntimeClient>, mut app: App) -> anyhow::Result<()> {
    let mut terminal = terminal::setup_terminal()?;
    let (tx, mut rx) = mpsc::channel::<AppEvent>(256);

    spawn_input_handler(tx.clone());
    spawn_tick(tx.clone());

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

    // Suppress unused variable warning for cli when no local feature is active
    let _ = cli;

    Ok(())
}
