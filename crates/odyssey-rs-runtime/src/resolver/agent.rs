use crate::RuntimeError;
use crate::bundle::load_bundle;
use odyssey_rs_bundle::BundleStore;
use odyssey_rs_manifest::{AgentSpec, BundleManifest};
use odyssey_rs_protocol::{AgentRef, ModelSpec};
use std::path::PathBuf;

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
        default_model: loaded.agent.model.clone(),
        agent: loaded.agent,
    })
}
