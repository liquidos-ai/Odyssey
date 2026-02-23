//! Terminal UI for interacting with the embedded Odyssey orchestrator.
//! CLI entry point — see lib.rs for the reusable library API.

use anyhow::{Context, bail};
use autoagents_core::agent::prebuilt::executor::ReActAgent;
#[cfg(feature = "local")]
use autoagents_llamacpp::{LlamaCppProvider, ModelSource};
use autoagents_llm::LLMProvider;
use autoagents_llm::backends::openai::OpenAI;
use autoagents_llm::builder::LLMBuilder;
use clap::Parser;
use log::info;
use odyssey_rs_config::OdysseyConfig;
use odyssey_rs_core::orchestrator::prompt::PromptProfile;
use odyssey_rs_core::skills::SkillStore;
use odyssey_rs_core::{
    AgentBuilder, DEFAULT_AGENT_ID, LLMEntry, OdysseyAgent, Orchestrator, PromptBuilder,
};
use odyssey_rs_memory::FileMemoryProvider;
#[cfg(target_os = "linux")]
use odyssey_rs_sandbox::BubblewrapProvider;
#[cfg(not(target_os = "linux"))]
use odyssey_rs_sandbox::LocalSandboxProvider;
use odyssey_rs_sandbox::SandboxProvider;
use odyssey_rs_tools::builtin_tool_registry;
use odyssey_rs_tui::{EventBus, TuiConfig};
use std::path::PathBuf;
use std::sync::Arc;

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
    let sandbox: Option<Arc<dyn SandboxProvider>> = {
        #[cfg(target_os = "linux")]
        {
            Some(Arc::new(
                BubblewrapProvider::new().context("failed to init bubblewrap provider")?,
            ))
        }
        #[cfg(not(target_os = "linux"))]
        {
            // Not yet supported.
            Some(Arc::new(LocalSandboxProvider::default()))
        }
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

    let app_model_name = {
        #[cfg(not(feature = "local"))]
        {
            if openai_registered {
                model_name.clone()
            } else {
                String::new()
            }
        }
        #[cfg(feature = "local")]
        {
            if openai_registered {
                model_name.clone()
            } else if let Some(local) = local_registration.as_ref() {
                local.label.clone()
            } else {
                String::new()
            }
        }
    };

    let tui_config = TuiConfig {
        model_name: app_model_name,
        model_id: DEFAULT_LLM_ID.to_string(),
        agent_id: cli.agent.clone(),
        cwd: Some(cwd),
        ..Default::default()
    };

    odyssey_rs_tui::run(Arc::clone(&orchestrator), events, tui_config).await
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
