use crate::build::BundleMetadata;
use crate::layout::{BundleConfig, OCI_INDEX_MEDIA_TYPE, OCI_MANIFEST_MEDIA_TYPE, REF_ANNOTATION};
use base64::Engine;
use reqwest::{StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::env;
use thiserror::Error;

const HUB_TOKEN_ENV: &str = "ODYSSEY_HUB_TOKEN";

#[derive(Debug, Error)]
pub enum HubClientError {
    #[error("invalid hub URL: {0}")]
    InvalidHubUrl(String),
    #[error(transparent)]
    Transport(#[from] reqwest::Error),
    #[error("hub request failed with {status}: {message}")]
    HttpStatus { status: StatusCode, message: String },
}

#[derive(Clone)]
pub(crate) struct HubClient {
    base_url: Url,
    token: Option<String>,
    http: reqwest::Client,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PublishBundleRequest {
    pub repository: String,
    pub tag: String,
    pub manifest_digest: String,
    #[serde(with = "base64_bytes")]
    pub manifest_bytes: Vec<u8>,
    #[serde(with = "base64_bytes")]
    pub config_bytes: Vec<u8>,
    pub layers: Vec<BlobPayload>,
    pub metadata: BundleMetadata,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PullBundleRequest {
    pub repository: String,
    pub tag: Option<String>,
    pub digest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BlobPayload {
    pub digest: String,
    #[serde(with = "base64_bytes")]
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PullBundleResponse {
    pub manifest_bytes: Vec<u8>,
    pub index_bytes: Vec<u8>,
    pub config_bytes: Vec<u8>,
    pub metadata: BundleMetadata,
    pub layers: Vec<BlobPayload>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HubPullBundleResponse {
    #[serde(with = "base64_bytes")]
    manifest_bytes: Vec<u8>,
    #[serde(with = "base64_bytes")]
    config_bytes: Vec<u8>,
    layers: Vec<BlobPayload>,
    version: HubArtifactVersion,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HubArtifactVersion {
    version: String,
    manifest_digest: String,
}

impl HubClient {
    pub(crate) fn from_hub_url(hub_url: &str) -> Result<Self, HubClientError> {
        let mut url =
            Url::parse(hub_url).map_err(|err| HubClientError::InvalidHubUrl(err.to_string()))?;
        if url.path() != "/" && !url.path().is_empty() {
            return Err(HubClientError::InvalidHubUrl(
                "hub URL must not include a path".to_string(),
            ));
        }

        if !url.username().is_empty() {
            return Err(HubClientError::InvalidHubUrl(
                "hub URL must not include a username".to_string(),
            ));
        }
        if url.password().is_some() {
            return Err(HubClientError::InvalidHubUrl(
                "hub URL must not include a password".to_string(),
            ));
        }
        url.set_path("");

        Ok(Self {
            base_url: url,
            token: hub_token(),
            http: reqwest::Client::new(),
        })
    }

    pub(crate) async fn publish_bundle(
        &self,
        request: PublishBundleRequest,
    ) -> Result<BundleMetadata, HubClientError> {
        let metadata = request.metadata.clone();
        let response = self
            .with_auth(
                self.http
                    .post(self.endpoint("api/v1/artifacts/publish")?)
                    .json(&request),
            )
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(http_status(response).await);
        }
        Ok(metadata)
    }

    pub(crate) async fn pull_bundle(
        &self,
        request: PullBundleRequest,
    ) -> Result<PullBundleResponse, HubClientError> {
        let response = self
            .with_auth(
                self.http
                    .post(self.endpoint("api/v1/artifacts/pull")?)
                    .json(&request),
            )
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(http_status(response).await);
        }
        let response = response.json::<HubPullBundleResponse>().await?;
        let config: BundleConfig = serde_json::from_slice(&response.config_bytes)
            .map_err(|err| HubClientError::InvalidHubUrl(err.to_string()))?;
        let metadata = BundleMetadata {
            namespace: config.namespace.clone(),
            id: config.id.clone(),
            version: config.version.clone(),
            digest: response.version.manifest_digest.clone(),
            bundle_manifest: config.bundle_manifest,
            agent_spec: config.agent_spec,
        };
        Ok(PullBundleResponse {
            index_bytes: build_index_bytes(
                &request.repository,
                request.tag.as_deref().unwrap_or(&response.version.version),
                &response.version.manifest_digest,
                response.manifest_bytes.len(),
                request.digest.is_some(),
            )?,
            manifest_bytes: response.manifest_bytes,
            config_bytes: response.config_bytes,
            metadata,
            layers: response.layers,
        })
    }

    fn endpoint(&self, path: &str) -> Result<Url, HubClientError> {
        self.base_url
            .join(path)
            .map_err(|err| HubClientError::InvalidHubUrl(err.to_string()))
    }

    fn with_auth(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.token {
            Some(token) => request.bearer_auth(token),
            None => request,
        }
    }
}

async fn http_status(response: reqwest::Response) -> HubClientError {
    let status = response.status();
    let message = response.text().await.unwrap_or_default();
    HubClientError::HttpStatus { status, message }
}

fn hub_token() -> Option<String> {
    env::var(HUB_TOKEN_ENV)
        .ok()
        .filter(|value| !value.is_empty())
}

fn build_index_bytes(
    repository: &str,
    version: &str,
    manifest_digest: &str,
    manifest_size: usize,
    pull_by_digest: bool,
) -> Result<Vec<u8>, HubClientError> {
    let reference_name = if pull_by_digest {
        format!("{repository}@{manifest_digest}")
    } else {
        format!("{repository}:{version}")
    };
    serde_json::to_vec_pretty(&json!({
        "schemaVersion": 2,
        "mediaType": OCI_INDEX_MEDIA_TYPE,
        "manifests": [{
            "mediaType": OCI_MANIFEST_MEDIA_TYPE,
            "digest": manifest_digest,
            "size": manifest_size,
            "annotations": {
                REF_ANNOTATION: reference_name,
            }
        }]
    }))
    .map_err(|err| HubClientError::InvalidHubUrl(err.to_string()))
}

mod base64_bytes {
    use super::*;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S>(value: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&base64::engine::general_purpose::STANDARD.encode(value))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = String::deserialize(deserializer)?;
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::{HUB_TOKEN_ENV, HubClient, HubClientError, hub_token};
    use pretty_assertions::assert_eq;
    use reqwest::Url;
    use std::env;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn hub_url_parsing_supports_http_and_https() {
        let client = HubClient::from_hub_url("http://localhost:8473").expect("client");

        assert_eq!(
            client
                .endpoint("api/v1/artifacts/publish")
                .expect("endpoint"),
            "http://localhost:8473/api/v1/artifacts/publish"
                .parse::<Url>()
                .expect("url")
        );
    }

    #[test]
    fn hub_url_rejects_non_root_paths() {
        let error = HubClient::from_hub_url("http://localhost:8473/api")
            .err()
            .expect("path should fail");
        assert_eq!(
            error.to_string(),
            HubClientError::InvalidHubUrl("hub URL must not include a path".to_string())
                .to_string()
        );
    }

    #[test]
    fn hub_url_rejects_embedded_credentials() {
        let error = HubClient::from_hub_url("http://user:pass@localhost:8473")
            .err()
            .expect("credentials should fail");
        assert_eq!(
            error.to_string(),
            HubClientError::InvalidHubUrl("hub URL must not include a username".to_string())
                .to_string()
        );
    }

    #[test]
    fn hub_token_uses_non_empty_env_value() {
        let _guard = env_lock().lock().expect("env lock");
        unsafe {
            env::remove_var(HUB_TOKEN_ENV);
        }
        assert_eq!(hub_token(), None);

        unsafe {
            env::set_var(HUB_TOKEN_ENV, "");
        }
        assert_eq!(hub_token(), None);

        unsafe {
            env::set_var(HUB_TOKEN_ENV, "token-value");
        }
        assert_eq!(hub_token(), Some("token-value".to_string()));

        unsafe {
            env::remove_var(HUB_TOKEN_ENV);
        }
    }
}
