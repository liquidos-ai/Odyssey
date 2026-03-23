use anyhow::{Result, anyhow};
use odyssey_rs_protocol::{Session, SessionSummary};
use odyssey_rs_runtime::RunOutput;
use serde::Deserialize;
use uuid::Uuid;

#[derive(Clone)]
pub(crate) struct RemoteRuntimeClient {
    client: reqwest::Client,
    base_url: String,
}

impl RemoteRuntimeClient {
    pub(crate) fn new(base_url: impl Into<String>) -> Result<Self> {
        let base_url = base_url.into().trim_end_matches('/').to_string();
        if base_url.is_empty() {
            return Err(anyhow!("remote URL cannot be empty"));
        }
        Ok(Self {
            client: reqwest::Client::new(),
            base_url,
        })
    }

    pub(crate) async fn inspect(
        &self,
        reference: &str,
    ) -> Result<odyssey_rs_bundle::BundleMetadata> {
        self.get(&format!("/bundles/inspect?reference={reference}"))
            .await
    }

    pub(crate) async fn run(&self, bundle_ref: String, input: String) -> Result<RunOutput> {
        let session: SessionSummary = self
            .post(
                "/sessions",
                serde_json::json!({
                    "bundle_ref": bundle_ref
                }),
            )
            .await?;
        self.post(
            &format!("/sessions/{}/run-sync", session.id),
            serde_json::json!({
                "input": input
            }),
        )
        .await
    }

    pub(crate) async fn pull(
        &self,
        reference: &str,
        hub_url: &str,
    ) -> Result<odyssey_rs_bundle::BundleInstall> {
        self.post(
            "/bundles/pull",
            serde_json::json!({
                "reference": reference,
                "hub_url": hub_url
            }),
        )
        .await
    }

    pub(crate) async fn list_bundles(
        &self,
    ) -> Result<Vec<odyssey_rs_bundle::BundleInstallSummary>> {
        self.get("/bundles").await
    }

    pub(crate) async fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
        self.get("/sessions").await
    }

    pub(crate) async fn get_session(&self, id: Uuid) -> Result<Session> {
        self.get(&format!("/sessions/{id}")).await
    }

    pub(crate) async fn delete_session(&self, id: Uuid) -> Result<()> {
        let response = self
            .client
            .delete(self.url(&format!("/sessions/{id}")))
            .send()
            .await?;
        if !response.status().is_success() {
            let message = response.text().await.unwrap_or_default();
            return Err(anyhow!("remote request failed: {message}"));
        }
        Ok(())
    }

    async fn get<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T> {
        let response = self.client.get(self.url(path)).send().await?;
        Self::parse_json(response).await
    }

    async fn post<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        body: serde_json::Value,
    ) -> Result<T> {
        let response = self.client.post(self.url(path)).json(&body).send().await?;
        Self::parse_json(response).await
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    async fn parse_json<T: for<'de> Deserialize<'de>>(response: reqwest::Response) -> Result<T> {
        let status = response.status();
        if !status.is_success() {
            let message = response.text().await.unwrap_or_default();
            return Err(anyhow!("remote request failed ({status}): {message}"));
        }
        Ok(response.json::<T>().await?)
    }
}
