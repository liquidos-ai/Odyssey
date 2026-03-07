//! LLM provider registration helpers.

#[cfg(feature = "local")]
use crate::cli::Cli;
use anyhow::Context;
#[cfg(feature = "local")]
use autoagents_llamacpp::{LlamaCppProvider, ModelSource};
use autoagents_llm::backends::openai::OpenAI;
use autoagents_llm::builder::LLMBuilder;
use autoagents_llm::{LLMProvider, chat::ReasoningEffort};
use log::info;
use odyssey_rs_core::{AgentRuntime, LLMEntry};
use std::sync::Arc;

pub const DEFAULT_LLM_ID: &str = "default_LLM";
#[cfg(feature = "local")]
pub const LOCAL_LLM_ID: &str = "local-llama-cpp";

/// Metadata returned after registering the local llama.cpp provider.
#[cfg(feature = "local")]
pub struct LocalLlmRegistration {
    /// Human-readable label shown in the header.
    pub label: String,
}

/// Build and register the OpenAI provider.
///
/// Returns `None` when `OPENAI_API_KEY` is not set.
pub fn build_openai_provider(
    model_name: &str,
    api_key: String,
) -> anyhow::Result<Arc<dyn LLMProvider>> {
    info!("building default LLM provider (model={})", model_name);
    let provider = LLMBuilder::<OpenAI>::new()
        .api_key(api_key)
        .model(model_name.to_string())
        .reasoning(true)
        .reasoning_effort(ReasoningEffort::Medium)
        .build()
        .context("failed to build OpenAI LLM provider")?;
    // LLMBuilder::build() returns Arc<OpenAI>; coerce to trait object.
    Ok(provider as Arc<dyn LLMProvider>)
}

/// Register an LLM provider with the orchestrator.
pub fn register_llm(
    orchestrator: &Arc<AgentRuntime>,
    id: impl Into<String>,
    provider: Arc<dyn LLMProvider>,
) -> anyhow::Result<()> {
    orchestrator.register_llm_provider(LLMEntry {
        id: id.into(),
        provider,
    })?;
    Ok(())
}

/// Build and register the local llama.cpp provider.
#[cfg(feature = "local")]
pub async fn register_local_llm(
    cli: &Cli,
    orchestrator: &Arc<AgentRuntime>,
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

    let provider: Arc<dyn LLMProvider> = Arc::new(
        builder
            .build()
            .await
            .context("failed to build llama.cpp provider")?,
    );

    let llm_id = if openai_registered {
        LOCAL_LLM_ID.to_string()
    } else {
        DEFAULT_LLM_ID.to_string()
    };

    register_llm(orchestrator, &llm_id, provider)?;
    info!("registered llama.cpp provider (llm_id={llm_id})");
    Ok(LocalLlmRegistration { label })
}

#[cfg(feature = "local")]
fn resolve_local_model_source(cli: &Cli) -> anyhow::Result<ModelSource> {
    use anyhow::bail;
    if cli.local_gguf.is_some() && cli.local_hf_repo.is_some() {
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
    // Sensible default
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
                .and_then(|n| n.to_str())
                .unwrap_or(model_path);
            format!("gguf:{name}")
        }
        ModelSource::HuggingFace {
            repo_id,
            filename,
            mmproj_filename: _,
        } => filename
            .as_ref()
            .map(|f| format!("hf:{repo_id}/{f}"))
            .unwrap_or_else(|| format!("hf:{repo_id}")),
    }
}
