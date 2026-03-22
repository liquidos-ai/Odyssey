use crate::BundleError;
use crate::build::BundleMetadata;
use crate::client::{
    BlobPayload, HubClient, HubClientError, PublishBundleRequest, PullBundleRequest,
};
use crate::layout::{blob_path, read_blob, read_config, read_manifest};
use odyssey_rs_manifest::{BundleRef, BundleRefKind};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct PulledLayout {
    pub manifest_bytes: Vec<u8>,
    pub index_bytes: Vec<u8>,
    pub config_bytes: Vec<u8>,
    pub metadata: BundleMetadata,
    pub layers: Vec<(String, Vec<u8>)>,
}

pub async fn publish_layout(
    hub_url: &str,
    bundle_path: &Path,
    target: &BundleRef,
) -> Result<BundleMetadata, BundleError> {
    if target.kind != BundleRefKind::Remote {
        return Err(BundleError::Invalid(
            "publish target must be a namespaced remote reference".to_string(),
        ));
    }
    let repository = target
        .repository()
        .ok_or_else(|| BundleError::Invalid("publish target missing repository".to_string()))?;
    let client = HubClient::from_hub_url(hub_url).map_err(hub_err)?;
    let (_, manifest, manifest_digest) = read_manifest(bundle_path)?;
    let config = read_config(bundle_path, &manifest)?;
    let config_bytes = read_blob(bundle_path, &manifest.config.digest)?;
    let metadata = BundleMetadata {
        namespace: target
            .namespace
            .clone()
            .unwrap_or_else(|| config.namespace.clone()),
        id: target.id.clone().unwrap_or_else(|| config.id.clone()),
        version: target
            .version
            .clone()
            .unwrap_or_else(|| config.version.clone()),
        digest: manifest_digest.clone(),
        readme: config.readme.clone(),
        bundle_manifest: config.bundle_manifest.clone(),
        agent_spec: config.agent_spec.clone(),
    };
    let mut layers = Vec::with_capacity(manifest.layers.len());
    for layer in &manifest.layers {
        let bytes = read_blob(bundle_path, &layer.digest)?;
        layers.push(BlobPayload {
            digest: layer.digest.clone(),
            bytes,
        });
    }

    let manifest_path = blob_path(bundle_path, &manifest_digest)?;
    let manifest_bytes = std::fs::read(&manifest_path).map_err(|err| BundleError::Io {
        path: manifest_path.display().to_string(),
        message: err.to_string(),
    })?;
    client
        .publish_bundle(PublishBundleRequest {
            repository,
            tag: metadata.version.clone(),
            manifest_digest,
            manifest_bytes,
            config_bytes,
            layers,
            metadata,
        })
        .await
        .map_err(hub_err)
}

pub async fn pull_layout(
    hub_url: &str,
    reference: &BundleRef,
) -> Result<PulledLayout, BundleError> {
    let repository = reference
        .repository()
        .ok_or_else(|| BundleError::Invalid("remote reference missing repository".to_string()))?;
    let client = HubClient::from_hub_url(hub_url).map_err(hub_err)?;
    let pulled = client
        .pull_bundle(PullBundleRequest {
            repository,
            tag: reference.version.clone(),
            digest: reference.digest.clone(),
        })
        .await
        .map_err(hub_err)?;
    let layers = pulled
        .layers
        .into_iter()
        .map(|layer| (layer.digest, layer.bytes))
        .collect();

    Ok(PulledLayout {
        manifest_bytes: pulled.manifest_bytes,
        index_bytes: pulled.index_bytes,
        config_bytes: pulled.config_bytes,
        metadata: pulled.metadata,
        layers,
    })
}

fn hub_err(err: HubClientError) -> BundleError {
    match err {
        HubClientError::InvalidHubUrl(message) => BundleError::Invalid(message),
        HubClientError::Transport(source) => BundleError::Http(source.to_string()),
        HubClientError::HttpStatus { status, message } => {
            BundleError::Http(format!("hub returned {status}: {message}"))
        }
    }
}
