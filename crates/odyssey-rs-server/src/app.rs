use crate::routes::router;
use axum::Router;
use odyssey_rs_runtime::{RuntimeConfig, RuntimeEngine, RuntimeError};
use std::sync::Arc;
use tower_http::{cors::CorsLayer, trace::TraceLayer};

#[derive(Clone)]
pub struct AppState {
    pub runtime: Arc<RuntimeEngine>,
}

pub fn build_app(runtime: Arc<RuntimeEngine>) -> Router {
    router(AppState { runtime })
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
}

pub async fn serve(config: RuntimeConfig) -> Result<(), RuntimeError> {
    let runtime = Arc::new(RuntimeEngine::new(config.clone())?);
    let app = build_app(runtime);
    let listener = tokio::net::TcpListener::bind(&config.bind_addr)
        .await
        .map_err(|err| RuntimeError::Io {
            path: config.bind_addr.clone(),
            message: err.to_string(),
        })?;
    axum::serve(listener, app)
        .await
        .map_err(|err| RuntimeError::Executor(err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::build_app;
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use odyssey_rs_protocol::SandboxMode;
    use odyssey_rs_runtime::{RuntimeConfig, RuntimeEngine};
    use pretty_assertions::assert_eq;
    use std::sync::Arc;
    use tempfile::tempdir;
    use tower::util::ServiceExt;

    fn runtime_config(root: &std::path::Path) -> RuntimeConfig {
        RuntimeConfig {
            cache_root: root.join("bundles"),
            session_root: root.join("sessions"),
            sandbox_root: root.join("sandbox"),
            bind_addr: "127.0.0.1:0".to_string(),
            sandbox_mode_override: Some(SandboxMode::DangerFullAccess),
            hub_url: "http://127.0.0.1:8473".to_string(),
        }
    }

    #[tokio::test]
    async fn app_builder_wraps_router() {
        let temp = tempdir().expect("tempdir");
        let runtime = Arc::new(RuntimeEngine::new(runtime_config(temp.path())).expect("runtime"));
        let app = build_app(runtime);

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/does-not-exist")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
