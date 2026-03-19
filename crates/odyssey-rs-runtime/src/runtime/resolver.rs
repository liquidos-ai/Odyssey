use super::bundle_loader::load_bundle;
use crate::RuntimeError;
use odyssey_rs_bundle::BundleStore;
use odyssey_rs_manifest::{AgentSpec, BundleManifest};
use odyssey_rs_protocol::{AgentRef, ModelSpec};
use std::{path::PathBuf, sync::Arc};

#[derive(Clone)]
pub(crate) struct ResolvedAgentSpec {
    pub install_path: PathBuf,
    pub manifest: BundleManifest,
    pub agent: AgentSpec,
    pub default_model: ModelSpec,
}

pub(crate) fn resolve_agent(
    store: &BundleStore,
    agent_ref: &AgentRef,
) -> Result<ResolvedAgentSpec, RuntimeError> {
    let loaded = load_bundle(store, agent_ref.as_str())?;
    Ok(ResolvedAgentSpec {
        install_path: loaded.install.path,
        manifest: loaded.manifest,
        default_model: ModelSpec {
            provider: loaded.agent.model.provider.clone(),
            name: loaded.agent.model.name.clone(),
        },
        agent: loaded.agent,
    })
}

pub(crate) struct LLMResolver<'a> {
    model_spec: &'a ModelSpec,
}

impl<'a> LLMResolver<'a> {
    pub fn new(model_spec: &'a ModelSpec) -> Self {
        Self { model_spec }
    }

    pub fn build_llm(&self) -> Result<Arc<dyn autoagents_llm::LLMProvider>, RuntimeError> {
        match self.model_spec.provider.as_str() {
            "openai" => {
                let key = std::env::var("OPENAI_API_KEY").map_err(|_| {
                    RuntimeError::Unsupported(
                        "OPENAI_API_KEY is required for provider openai".to_string(),
                    )
                })?;
                let llm: Arc<dyn autoagents_llm::LLMProvider> = autoagents_llm::builder::LLMBuilder::<
                autoagents_llm::backends::openai::OpenAI,
            >::new()
            .api_key(key)
            .model(&self.model_spec.name)
            .build()
            .map_err(|err| RuntimeError::Executor(err.to_string()))?;
                Ok(llm)
            }
            other => Err(RuntimeError::Unsupported(format!(
                "unsupported model provider: {other}"
            ))),
        }
    }
}
