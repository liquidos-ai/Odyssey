use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, response::IntoResponse};
use base64::Engine;
use odyssey_rs_bundle::{BundleMetadata, BundleStore};
use pretty_assertions::assert_eq;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tempfile::tempdir;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio::time::{Duration, sleep};

fn write_bundle_project(root: &Path) {
    fs::create_dir_all(root.join("skills").join("repo-hygiene")).expect("create skill dir");
    fs::create_dir_all(root.join("data")).expect("create data dir");
    fs::write(
        root.join("odyssey.bundle.json5"),
        r#"{
            id: "demo",
            version: "0.1.0",
            agent_spec: "agent.yaml",
            executor: { type: "prebuilt", id: "react" },
            memory: { provider: { type: "prebuilt", id: "sliding_window" } },
            resources: ["data"],
            skills: [{ name: "repo-hygiene", path: "skills/repo-hygiene" }],
            tools: [{ name: "Read", source: "builtin" }],
            server: { enable_http: true },
            sandbox: {
                permissions: {
                    filesystem: { exec: [], mounts: { read: [], write: [] } },
                    network: [],
                    tools: { mode: "default", rules: [] }
                },
                system_tools: [],
                resources: {}
            }
        }"#,
    )
    .expect("write manifest");
    fs::write(
        root.join("agent.yaml"),
        r#"id: demo
description: test bundle
prompt: keep responses concise
model:
  provider: openai
  name: gpt-4.1-mini
tools:
  allow: ["Read", "Skill"]
  deny: []
"#,
    )
    .expect("write agent");
    fs::write(
        root.join("skills").join("repo-hygiene").join("SKILL.md"),
        "# Repo Hygiene\n",
    )
    .expect("write skill");
    fs::write(root.join("data").join("notes.txt"), "hello world\n").expect("write resource");
}

#[test]
fn export_and_import_round_trip_bundle_layout() {
    let temp = tempdir().expect("tempdir");
    let project_root = temp.path().join("project");
    fs::create_dir_all(&project_root).expect("create project");
    write_bundle_project(&project_root);

    let store = BundleStore::new(temp.path().join("store"));
    let install = store
        .build_and_install(&project_root)
        .expect("build install");

    let archive_path = store
        .export("demo@0.1.0", temp.path().join("exports"))
        .expect("export archive");
    let imported_store = BundleStore::new(temp.path().join("import-store"));
    let imported = imported_store.import(archive_path).expect("import archive");

    assert_eq!(imported.metadata.id, install.metadata.id);
    assert_eq!(imported.metadata.version, install.metadata.version);
    assert_eq!(imported.metadata.digest, install.metadata.digest);
}

#[test]
fn export_to_directory_uses_bundle_identity_in_filename() {
    let temp = tempdir().expect("tempdir");
    let project_root = temp.path().join("project");
    fs::create_dir_all(&project_root).expect("create project");
    write_bundle_project(&project_root);

    let store = BundleStore::new(temp.path().join("local-store"));
    let install = store
        .build_and_install(&project_root)
        .expect("build install");
    let export_dir = temp.path().join("exports");
    fs::create_dir_all(&export_dir).expect("create export dir");

    let export_path = store
        .export(install.path.to_str().expect("install path"), &export_dir)
        .expect("export to directory");

    assert_eq!(
        export_path.file_name().and_then(|value| value.to_str()),
        Some("demo-0.1.0.odyssey")
    );
}

#[cfg_attr(tarpaulin, ignore)]
#[tokio::test]
async fn publish_and_pull_round_trip_through_hub_api() {
    let temp = tempdir().expect("tempdir");
    let project_root = temp.path().join("project");
    fs::create_dir_all(&project_root).expect("create project");
    write_bundle_project(&project_root);

    let publisher_store = BundleStore::new(temp.path().join("publisher-store"));
    let install = publisher_store
        .build_and_install(&project_root)
        .expect("build install");

    let hub_url = start_hub().await;
    let published = publisher_store
        .publish(
            project_root.to_str().expect("project root path"),
            "team/demo:0.1.0",
            &hub_url,
        )
        .await
        .expect("publish");

    let consumer_store = BundleStore::new(temp.path().join("consumer-store"));
    let pulled = consumer_store
        .pull("team/demo:0.1.0", &hub_url)
        .await
        .expect("pull");

    assert_eq!(pulled.metadata.namespace, "team");
    assert_eq!(pulled.metadata.id, install.metadata.id);
    assert_eq!(pulled.metadata.version, install.metadata.version);
    assert_eq!(pulled.metadata.digest, published.digest);
    assert_eq!(
        fs::read_to_string(pulled.path.join("agent.yaml")).expect("read pulled agent"),
        fs::read_to_string(install.path.join("agent.yaml")).expect("read built agent")
    );

    let by_digest = consumer_store
        .pull(&format!("team/demo@{}", published.digest), &hub_url)
        .await
        .expect("pull by digest");
    assert_eq!(by_digest.metadata.digest, published.digest);
}

#[tokio::test]
async fn publish_requires_namespaced_remote_target_for_project_sources() {
    let temp = tempdir().expect("tempdir");
    let project_root = temp.path().join("project");
    fs::create_dir_all(&project_root).expect("create project");
    write_bundle_project(&project_root);

    let store = BundleStore::new(temp.path().join("store"));
    let error = store
        .publish(
            project_root.to_str().expect("project root path"),
            "demo:0.1.0",
            "http://127.0.0.1:8473",
        )
        .await
        .expect_err("publish without namespace should fail");

    assert_eq!(
        error.to_string(),
        "invalid bundle: publish target must be a namespaced remote reference"
    );
}

#[tokio::test]
async fn publish_surfaces_hub_http_failures() {
    let temp = tempdir().expect("tempdir");
    let project_root = temp.path().join("project");
    fs::create_dir_all(&project_root).expect("create project");
    write_bundle_project(&project_root);

    let store = BundleStore::new(temp.path().join("store"));
    let hub_url =
        start_failing_hub(StatusCode::BAD_REQUEST, json!({ "error": "bad bundle" })).await;
    let error = store
        .publish(
            project_root.to_str().expect("project root path"),
            "team/demo:0.1.0",
            &hub_url,
        )
        .await
        .expect_err("publish should fail");

    assert_eq!(
        error.to_string(),
        "http error: hub returned 400 Bad Request: {\"error\":\"bad bundle\"}"
    );
}

#[tokio::test]
async fn pull_rejects_non_remote_references() {
    let temp = tempdir().expect("tempdir");
    let store = BundleStore::new(temp.path().join("store"));

    let error = store
        .pull("team/:0.1.0", "http://127.0.0.1:8473")
        .await
        .expect_err("invalid remote reference should fail");

    assert_eq!(
        error.to_string(),
        "invalid bundle: remote reference missing repository"
    );
}

#[tokio::test]
async fn pull_surfaces_hub_http_failures() {
    let temp = tempdir().expect("tempdir");
    let store = BundleStore::new(temp.path().join("store"));
    let hub_url = start_failing_hub(
        StatusCode::NOT_FOUND,
        json!({ "error": "bundle not found" }),
    )
    .await;

    let error = store
        .pull("team/demo:0.1.0", &hub_url)
        .await
        .expect_err("pull should fail");

    assert_eq!(
        error.to_string(),
        "http error: hub returned 404 Not Found: {\"error\":\"bundle not found\"}"
    );
}

async fn start_hub() -> String {
    let app = hub_app();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("listener addr");
    let server: JoinHandle<()> = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve hub");
    });
    sleep(Duration::from_millis(25)).await;
    drop(server);
    format!("http://{addr}")
}

async fn start_failing_hub(status: StatusCode, body: serde_json::Value) -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("listener addr");
    let server: JoinHandle<()> = tokio::spawn(async move {
        let app = Router::new()
            .route("/api/v1/bundles/publish", post(fail_publish_bundle))
            .route("/api/v1/bundles/pull", post(fail_pull_bundle))
            .with_state(FailingHubState { status, body });
        axum::serve(listener, app).await.expect("serve failing hub");
    });
    sleep(Duration::from_millis(25)).await;
    drop(server);
    format!("http://{addr}")
}

#[derive(Clone, Default)]
struct HubState {
    by_digest: Arc<Mutex<HashMap<String, StoredBundle>>>,
    by_tag: Arc<Mutex<HashMap<String, StoredBundle>>>,
}

#[derive(Clone)]
struct StoredBundle {
    repository: String,
    tag: String,
    manifest_digest: String,
    manifest_bytes: Vec<u8>,
    config_bytes: Vec<u8>,
    metadata: BundleMetadata,
    layers: Vec<BlobPayload>,
}

#[derive(Clone)]
struct FailingHubState {
    status: StatusCode,
    body: serde_json::Value,
}

fn hub_app() -> Router {
    Router::new()
        .route("/api/v1/bundles/publish", post(publish_bundle))
        .route("/api/v1/bundles/pull", post(pull_bundle))
        .with_state(HubState::default())
}

async fn publish_bundle(
    State(state): State<HubState>,
    Json(request): Json<PublishBundleRequest>,
) -> impl IntoResponse {
    let stored = StoredBundle {
        repository: request.repository.clone(),
        tag: request.tag.clone(),
        manifest_digest: request.manifest_digest.clone(),
        manifest_bytes: request.manifest_bytes,
        config_bytes: request.config_bytes,
        metadata: request.metadata.clone(),
        layers: request.layers,
    };

    state.by_digest.lock().expect("digest lock").insert(
        format!("{}/{}", request.repository, request.manifest_digest),
        stored.clone(),
    );
    state.by_tag.lock().expect("tag lock").insert(
        format!("{}/{}", request.repository, request.tag),
        stored.clone(),
    );

    (
        StatusCode::OK,
        Json(json!({
            "metadata": stored.metadata,
        })),
    )
}

async fn pull_bundle(
    State(state): State<HubState>,
    Json(request): Json<PullBundleRequest>,
) -> impl IntoResponse {
    let key = if let Some(ref digest) = request.digest {
        format!("{}/{}", request.repository, digest)
    } else if let Some(ref tag) = request.tag {
        format!("{}/{}", request.repository, tag)
    } else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "missing tag or digest" })),
        );
    };

    let bundle = state
        .by_digest
        .lock()
        .expect("digest lock")
        .get(&key)
        .cloned()
        .or_else(|| state.by_tag.lock().expect("tag lock").get(&key).cloned());

    let Some(bundle) = bundle else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "bundle not found" })),
        );
    };

    let reference_name = if request.digest.is_some() {
        format!("{}@{}", bundle.repository, bundle.manifest_digest)
    } else {
        format!("{}:{}", bundle.repository, bundle.tag)
    };
    let index_bytes = serde_json::to_vec_pretty(&json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.index.v1+json",
        "manifests": [{
            "mediaType": "application/vnd.oci.image.manifest.v1+json",
            "digest": bundle.manifest_digest,
            "size": bundle.manifest_bytes.len(),
            "annotations": {
                "org.opencontainers.image.ref.name": reference_name
            }
        }]
    }))
    .expect("index bytes");

    (
        StatusCode::OK,
        Json(json!({
            "manifestBytes": encode(&bundle.manifest_bytes),
            "indexBytes": encode(&index_bytes),
            "configBytes": encode(&bundle.config_bytes),
            "metadata": bundle.metadata,
            "layers": bundle.layers,
        })),
    )
}

async fn fail_publish_bundle(State(state): State<FailingHubState>) -> impl IntoResponse {
    (state.status, Json(state.body))
}

async fn fail_pull_bundle(State(state): State<FailingHubState>) -> impl IntoResponse {
    (state.status, Json(state.body))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublishBundleRequest {
    repository: String,
    tag: String,
    manifest_digest: String,
    #[serde(with = "base64_bytes")]
    manifest_bytes: Vec<u8>,
    #[serde(with = "base64_bytes")]
    config_bytes: Vec<u8>,
    layers: Vec<BlobPayload>,
    metadata: BundleMetadata,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PullBundleRequest {
    repository: String,
    tag: Option<String>,
    digest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BlobPayload {
    digest: String,
    #[serde(with = "base64_bytes")]
    bytes: Vec<u8>,
}

fn encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

mod base64_bytes {
    use super::*;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S>(value: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&encode(value))
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
