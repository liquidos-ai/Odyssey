use crate::build::BundleMetadata;
use base64::Engine;
use reqwest::{StatusCode, Url};
use serde::{Deserialize, Serialize};
use std::env;
use thiserror::Error;

const HUB_TOKEN_ENV: &str = "ODYSSEY_HUB_TOKEN";
const HUB_USERNAME_ENV: &str = "ODYSSEY_HUB_USERNAME";
const HUB_PASSWORD_ENV: &str = "ODYSSEY_HUB_PASSWORD";

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
    auth: HubAuth,
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
    #[serde(with = "base64_bytes")]
    pub manifest_bytes: Vec<u8>,
    #[serde(with = "base64_bytes")]
    pub index_bytes: Vec<u8>,
    #[serde(with = "base64_bytes")]
    pub config_bytes: Vec<u8>,
    pub metadata: BundleMetadata,
    pub layers: Vec<BlobPayload>,
}

#[derive(Debug, Deserialize)]
struct PublishBundleResponse {
    metadata: BundleMetadata,
}

#[derive(Debug, Clone)]
enum HubAuth {
    Anonymous,
    Basic(String, String),
    Bearer(String),
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

        let auth = auth_from_url(&url);
        if !url.username().is_empty() {
            url.set_username("")
                .map_err(|_| HubClientError::InvalidHubUrl("invalid username".to_string()))?;
        }
        if url.password().is_some() {
            url.set_password(None)
                .map_err(|_| HubClientError::InvalidHubUrl("invalid password".to_string()))?;
        }
        url.set_path("");

        Ok(Self {
            base_url: url,
            auth,
            http: reqwest::Client::new(),
        })
    }

    pub(crate) async fn publish_bundle(
        &self,
        request: PublishBundleRequest,
    ) -> Result<BundleMetadata, HubClientError> {
        let response = self
            .with_auth(
                self.http
                    .post(self.endpoint("api/v1/bundles/publish")?)
                    .json(&request),
            )
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(http_status(response).await);
        }
        Ok(response.json::<PublishBundleResponse>().await?.metadata)
    }

    pub(crate) async fn pull_bundle(
        &self,
        request: PullBundleRequest,
    ) -> Result<PullBundleResponse, HubClientError> {
        let response = self
            .with_auth(
                self.http
                    .post(self.endpoint("api/v1/bundles/pull")?)
                    .json(&request),
            )
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(http_status(response).await);
        }
        response
            .json::<PullBundleResponse>()
            .await
            .map_err(Into::into)
    }

    fn endpoint(&self, path: &str) -> Result<Url, HubClientError> {
        self.base_url
            .join(path)
            .map_err(|err| HubClientError::InvalidHubUrl(err.to_string()))
    }

    fn with_auth(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.auth {
            HubAuth::Anonymous => request,
            HubAuth::Basic(username, password) => request.basic_auth(username, Some(password)),
            HubAuth::Bearer(token) => request.bearer_auth(token),
        }
    }
}

async fn http_status(response: reqwest::Response) -> HubClientError {
    let status = response.status();
    let message = response.text().await.unwrap_or_default();
    HubClientError::HttpStatus { status, message }
}

fn auth_from_url(url: &Url) -> HubAuth {
    if !url.username().is_empty() {
        return HubAuth::Basic(
            url.username().to_string(),
            url.password().unwrap_or_default().to_string(),
        );
    }
    if let Some(token) = env::var(HUB_TOKEN_ENV)
        .ok()
        .filter(|value| !value.is_empty())
    {
        return HubAuth::Bearer(token);
    }
    match (
        env::var(HUB_USERNAME_ENV).ok(),
        env::var(HUB_PASSWORD_ENV).ok(),
    ) {
        (Some(username), Some(password)) if !username.is_empty() => {
            HubAuth::Basic(username, password)
        }
        _ => HubAuth::Anonymous,
    }
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
    use super::{HubClient, HubClientError, auth_from_url};
    use pretty_assertions::assert_eq;
    use reqwest::Url;

    #[test]
    fn hub_url_parsing_supports_http_https_and_basic_auth() {
        let client = HubClient::from_hub_url("http://user:pass@localhost:8473").expect("client");

        assert_eq!(
            client.endpoint("api/v1/bundles/publish").expect("endpoint"),
            "http://localhost:8473/api/v1/bundles/publish"
                .parse::<Url>()
                .expect("url")
        );

        let auth = auth_from_url(&"https://registry.example.com".parse::<Url>().expect("url"));
        assert_eq!(format!("{auth:?}"), "Anonymous");
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
}
