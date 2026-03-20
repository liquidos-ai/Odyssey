use crate::app::AppState;
use crate::models::{
    ApprovalResolution, BuildRequest, CreateSessionRequest, ExportRequest, ImportRequest,
    PlaceholderRequest, PublishRequest, ResolveApprovalRequest, RunRequest, TurnAccepted,
};
use crate::sse::stream_events;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use odyssey_rs_protocol::{AgentRef, ExecutionRequest, SessionSpec};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/bundles", get(list_bundles))
        .route("/bundles/build", post(build_bundle))
        .route("/bundles/inspect", get(inspect_bundle))
        .route("/bundles/export", post(export_bundle))
        .route("/bundles/import", post(import_bundle))
        .route("/bundles/publish", post(publish_bundle))
        .route("/bundles/pull", post(pull_bundle))
        .route("/sessions", get(list_sessions).post(create_session))
        .route("/sessions/{id}", get(get_session).delete(delete_session))
        .route("/sessions/{id}/run", post(run_session))
        .route("/sessions/{id}/run-sync", post(run_session_sync))
        .route("/sessions/{id}/events", get(session_events))
        .route("/approvals/{id}", post(resolve_approval))
        .with_state(state)
}

async fn list_bundles(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let bundles = state.bundles.list_installed().map_err(internal)?;
    Ok(Json(
        serde_json::to_value(bundles).map_err(|err| internal(err.to_string()))?,
    ))
}

async fn build_bundle(
    State(state): State<AppState>,
    Json(request): Json<BuildRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let install = state
        .bundles
        .build_and_install(&request.project_path)
        .map_err(internal)?;
    Ok(Json(json!({
        "id": install.metadata.id,
        "version": install.metadata.version,
        "digest": install.metadata.digest,
        "path": install.path,
    })))
}

#[derive(Debug, Deserialize)]
struct InspectQuery {
    reference: String,
}

async fn inspect_bundle(
    State(state): State<AppState>,
    Query(query): Query<InspectQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let metadata = state
        .bundles
        .resolve(&query.reference)
        .map(|install| install.metadata)
        .map_err(internal)?;
    Ok(Json(
        serde_json::to_value(metadata).map_err(|err| internal(err.to_string()))?,
    ))
}

async fn publish_bundle(
    State(state): State<AppState>,
    Json(request): Json<PublishRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    match state
        .bundles
        .publish(&request.source, &request.target, &state.hub_url)
        .await
    {
        Ok(metadata) => Ok(Json(json!({
            "status": "ok",
            "digest": metadata.digest,
            "id": metadata.id,
            "version": metadata.version,
        }))),
        Err(err) => Err((StatusCode::NOT_IMPLEMENTED, err.to_string())),
    }
}

async fn pull_bundle(
    State(state): State<AppState>,
    Json(request): Json<PlaceholderRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    match state.bundles.pull(&request.reference, &state.hub_url).await {
        Ok(install) => Ok(Json(json!({
            "status": "ok",
            "namespace": install.metadata.namespace,
            "id": install.metadata.id,
            "version": install.metadata.version,
            "digest": install.metadata.digest,
            "path": install.path,
        }))),
        Err(err) => Err((StatusCode::BAD_GATEWAY, err.to_string())),
    }
}

async fn export_bundle(
    State(state): State<AppState>,
    Json(request): Json<ExportRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let path = state
        .bundles
        .export(&request.reference, &request.output_path)
        .map_err(internal)?;
    Ok(Json(json!({
        "status": "ok",
        "path": path,
    })))
}

async fn import_bundle(
    State(state): State<AppState>,
    Json(request): Json<ImportRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let install = state
        .bundles
        .import(&request.archive_path)
        .map_err(internal)?;
    Ok(Json(json!({
        "status": "ok",
        "namespace": install.metadata.namespace,
        "id": install.metadata.id,
        "version": install.metadata.version,
        "digest": install.metadata.digest,
        "path": install.path,
    })))
}

async fn create_session(
    State(state): State<AppState>,
    Json(request): Json<CreateSessionRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let session = state
        .runtime
        .create_session(SessionSpec {
            agent_ref: AgentRef::from(request.agent_ref),
            model: request.model,
            metadata: json!({}),
        })
        .map_err(internal)?;
    Ok(Json(
        serde_json::to_value(session).map_err(|err| internal(err.to_string()))?,
    ))
}

async fn list_sessions(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let sessions = state.runtime.list_sessions(None);
    Ok(Json(
        serde_json::to_value(sessions).map_err(|err| internal(err.to_string()))?,
    ))
}

async fn get_session(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let session = state.runtime.get_session(id).map_err(internal)?;
    Ok(Json(
        serde_json::to_value(session).map_err(|err| internal(err.to_string()))?,
    ))
}

async fn delete_session(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, String)> {
    state.runtime.delete_session(id).map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn run_session(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(request): Json<RunRequest>,
) -> Result<Json<TurnAccepted>, (StatusCode, String)> {
    let turn_id = state
        .runtime
        .submit(ExecutionRequest {
            request_id: Uuid::new_v4(),
            session_id: id,
            input: request.input,
            turn_context: request.turn_context,
        })
        .await
        .map_err(internal)?;
    Ok(Json(TurnAccepted {
        session_id: id,
        turn_id: turn_id.turn_id,
    }))
}

async fn run_session_sync(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(request): Json<RunRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let result = state
        .runtime
        .run(ExecutionRequest {
            request_id: Uuid::new_v4(),
            session_id: id,
            input: request.input,
            turn_context: request.turn_context,
        })
        .await
        .map_err(internal)?;
    Ok(Json(
        serde_json::to_value(result).map_err(|err| internal(err.to_string()))?,
    ))
}

async fn session_events(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<
    axum::response::Sse<
        impl futures_util::Stream<Item = Result<axum::response::sse::Event, axum::Error>>,
    >,
    (StatusCode, String),
> {
    let receiver = state.runtime.subscribe_session(id).map_err(internal)?;
    Ok(stream_events(receiver))
}

async fn resolve_approval(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(request): Json<ResolveApprovalRequest>,
) -> Result<Json<ApprovalResolution>, (StatusCode, String)> {
    let resolved = state
        .runtime
        .resolve_approval(id, request.decision)
        .map_err(internal)?;
    Ok(Json(ApprovalResolution { resolved }))
}

fn internal(error: impl ToString) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::router;
    use crate::app::AppState;
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request, StatusCode};
    use odyssey_rs_protocol::SandboxMode;
    use odyssey_rs_runtime::{RuntimeConfig, RuntimeEngine};
    use pretty_assertions::assert_eq;
    use serde_json::Value;
    use std::fs;
    use std::path::Path;
    use std::sync::Arc;
    use tempfile::tempdir;
    use tower::util::ServiceExt;
    use uuid::Uuid;

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

    fn runtime_config(root: &Path) -> RuntimeConfig {
        RuntimeConfig {
            cache_root: root.join("bundles"),
            session_root: root.join("sessions"),
            sandbox_root: root.join("sandbox"),
            bind_addr: "127.0.0.1:0".to_string(),
            sandbox_mode_override: Some(SandboxMode::DangerFullAccess),
            hub_url: "http://127.0.0.1:8473".to_string(),
            worker_count: 2,
            queue_capacity: 32,
        }
    }

    async fn json_response(
        app: axum::Router,
        request: Request<Body>,
        expected_status: StatusCode,
    ) -> Value {
        let response = app.oneshot(request).await.expect("router response");
        assert_eq!(response.status(), expected_status);
        serde_json::from_slice(
            &to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("response body"),
        )
        .expect("json response")
    }

    #[tokio::test]
    async fn bundle_and_session_routes_round_trip() {
        let temp = tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).expect("create project root");
        write_bundle_project(&project_root);

        let runtime = Arc::new(RuntimeEngine::new(runtime_config(temp.path())).expect("runtime"));
        let app = router(AppState {
            runtime: runtime.clone(),
            bundles: runtime.bundle_store(),
            hub_url: runtime.config().hub_url.clone(),
        });

        let built = json_response(
            app.clone(),
            Request::builder()
                .method(Method::POST)
                .uri("/bundles/build")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "project_path": project_root
                    }))
                    .expect("serialize build request"),
                ))
                .expect("build request"),
            StatusCode::OK,
        )
        .await;
        assert_eq!(built["id"], "demo");
        assert_eq!(built["version"], "0.1.0");

        let inspected = json_response(
            app.clone(),
            Request::builder()
                .method(Method::GET)
                .uri("/bundles/inspect?reference=demo@0.1.0")
                .body(Body::empty())
                .expect("inspect request"),
            StatusCode::OK,
        )
        .await;
        assert_eq!(inspected["id"], "demo");
        assert_eq!(inspected["version"], "0.1.0");

        let export_dir = temp.path().join("exports");
        fs::create_dir_all(&export_dir).expect("create export dir");
        let exported = json_response(
            app.clone(),
            Request::builder()
                .method(Method::POST)
                .uri("/bundles/export")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "reference": "demo@0.1.0",
                        "output_path": export_dir
                    }))
                    .expect("serialize export request"),
                ))
                .expect("export request"),
            StatusCode::OK,
        )
        .await;
        let archive_path = exported["path"].as_str().expect("archive path");
        assert!(archive_path.ends_with(".odyssey"));

        let imported = json_response(
            app.clone(),
            Request::builder()
                .method(Method::POST)
                .uri("/bundles/import")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "archive_path": archive_path
                    }))
                    .expect("serialize import request"),
                ))
                .expect("import request"),
            StatusCode::OK,
        )
        .await;
        assert_eq!(imported["id"], "demo");
        assert_eq!(imported["version"], "0.1.0");

        let created = json_response(
            app.clone(),
            Request::builder()
                .method(Method::POST)
                .uri("/sessions")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "agent_ref": "demo@0.1.0"
                    }))
                    .expect("serialize session request"),
                ))
                .expect("create session request"),
            StatusCode::OK,
        )
        .await;
        let session_id = created["id"].as_str().expect("session id");
        assert_eq!(created["agent_id"], "demo");

        let fetched = json_response(
            app.clone(),
            Request::builder()
                .method(Method::GET)
                .uri(format!("/sessions/{session_id}"))
                .body(Body::empty())
                .expect("get session request"),
            StatusCode::OK,
        )
        .await;
        assert_eq!(fetched["id"], session_id);
        assert_eq!(fetched["agent_ref"], "demo@0.1.0");

        let resolved = json_response(
            app.clone(),
            Request::builder()
                .method(Method::POST)
                .uri(format!("/approvals/{}", Uuid::new_v4()))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&serde_json::json!({
                        "decision": "deny"
                    }))
                    .expect("serialize approval request"),
                ))
                .expect("approval request"),
            StatusCode::OK,
        )
        .await;
        assert_eq!(resolved["resolved"], serde_json::json!(false));

        let deleted = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri(format!("/sessions/{session_id}"))
                    .body(Body::empty())
                    .expect("delete session request"),
            )
            .await
            .expect("delete session response");
        assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn distribution_routes_map_publish_and_pull_failures() {
        let temp = tempdir().expect("tempdir");
        let project_root = temp.path().join("project");
        fs::create_dir_all(&project_root).expect("create project root");
        write_bundle_project(&project_root);

        let runtime = Arc::new(RuntimeEngine::new(runtime_config(temp.path())).expect("runtime"));
        let app = router(AppState {
            runtime: runtime.clone(),
            bundles: runtime.bundle_store(),
            hub_url: "http://127.0.0.1:9".to_string(),
        });

        let publish = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/bundles/publish")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "source": project_root,
                            "target": "demo:0.1.0"
                        }))
                        .expect("serialize publish request"),
                    ))
                    .expect("publish request"),
            )
            .await
            .expect("publish response");
        assert_eq!(publish.status(), StatusCode::NOT_IMPLEMENTED);
        assert_eq!(
            String::from_utf8(
                to_bytes(publish.into_body(), usize::MAX)
                    .await
                    .expect("publish body")
                    .to_vec()
            )
            .expect("publish text"),
            "invalid bundle: publish target must be a namespaced remote reference"
        );

        let pull = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/bundles/pull")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "reference": "team/:0.1.0"
                        }))
                        .expect("serialize pull request"),
                    ))
                    .expect("pull request"),
            )
            .await
            .expect("pull response");
        assert_eq!(pull.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(
            String::from_utf8(
                to_bytes(pull.into_body(), usize::MAX)
                    .await
                    .expect("pull body")
                    .to_vec()
            )
            .expect("pull text"),
            "invalid bundle: remote reference missing repository"
        );
    }

    #[tokio::test]
    async fn list_routes_return_empty_arrays_for_fresh_runtime() {
        let temp = tempdir().expect("tempdir");
        let runtime = Arc::new(RuntimeEngine::new(runtime_config(temp.path())).expect("runtime"));
        let app = router(AppState {
            runtime: runtime.clone(),
            bundles: runtime.bundle_store(),
            hub_url: runtime.config().hub_url.clone(),
        });

        let bundles = json_response(
            app.clone(),
            Request::builder()
                .method(Method::GET)
                .uri("/bundles")
                .body(Body::empty())
                .expect("bundles request"),
            StatusCode::OK,
        )
        .await;
        let sessions = json_response(
            app,
            Request::builder()
                .method(Method::GET)
                .uri("/sessions")
                .body(Body::empty())
                .expect("sessions request"),
            StatusCode::OK,
        )
        .await;

        assert_eq!(bundles, serde_json::json!([]));
        assert_eq!(sessions, serde_json::json!([]));
    }
}
