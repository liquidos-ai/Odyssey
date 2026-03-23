use crate::RuntimeError;
use crate::bundle::load_bundle;
use odyssey_rs_bundle::BundleStore;
use odyssey_rs_manifest::{AgentSpec, BundleManifest};
use odyssey_rs_protocol::{BundleRef, ModelSpec};
use std::path::PathBuf;

#[derive(Clone)]
pub(crate) struct ResolvedBundle {
    pub install_path: PathBuf,
    pub manifest: BundleManifest,
    pub default_agent: AgentSpec,
    pub model: ModelSpec,
}

pub(crate) fn resolve_bundle_from_ref(
    store: &BundleStore,
    bundle_ref: &BundleRef,
) -> Result<ResolvedBundle, RuntimeError> {
    let loaded = load_bundle(store, bundle_ref.as_str())?;
    Ok(ResolvedBundle {
        install_path: loaded.install.path,
        manifest: loaded.manifest,
        model: loaded.agent.model.clone(),
        default_agent: loaded.agent,
    })
}
