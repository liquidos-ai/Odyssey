//! Bundle install and selection handlers.

use crate::app::App;
use crate::client::AgentRuntimeClient;
use crate::event::AppEvent;
use crate::handlers::{agent, model, session};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

fn resolve_bundle_path(app: &App, raw: &str) -> PathBuf {
    let candidate = Path::new(raw);
    if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        Path::new(&app.cwd).join(candidate)
    }
}

/// Install a bundle project from `path`, switch to it, and activate a session.
pub async fn install_bundle(
    client: &Arc<AgentRuntimeClient>,
    app: &mut App,
    sender: mpsc::Sender<AppEvent>,
    stream_handle: &mut Option<JoinHandle<()>>,
    path: String,
) -> Result<(), String> {
    let install_root = resolve_bundle_path(app, &path);
    if !install_root.exists() {
        return Err(format!(
            "bundle project not found at {}",
            install_root.display()
        ));
    }
    let bundle_ref = client
        .install_bundle(&install_root)
        .await
        .map_err(|err| err.to_string())?;
    switch_bundle(client, app, sender, stream_handle, bundle_ref).await
}

/// Refresh the list of installed bundles in the local bundle store.
pub async fn refresh_bundles(
    client: &Arc<AgentRuntimeClient>,
    app: &mut App,
) -> Result<(), String> {
    let bundles = client.list_bundles().await.map_err(|err| err.to_string())?;
    app.set_bundles(bundles);
    Ok(())
}

/// Activate the currently highlighted installed bundle.
pub async fn activate_selected_bundle(
    client: &Arc<AgentRuntimeClient>,
    app: &mut App,
    sender: mpsc::Sender<AppEvent>,
    stream_handle: &mut Option<JoinHandle<()>>,
) -> Result<(), String> {
    let Some(bundle) = app.bundles.get(app.selected_bundle) else {
        return Ok(());
    };
    let bundle_ref = format!("{}/{}@{}", bundle.namespace, bundle.id, bundle.version);
    switch_bundle(client, app, sender, stream_handle, bundle_ref).await
}

/// Switch the active bundle, refresh dependent state, and activate a session.
pub async fn switch_bundle(
    client: &Arc<AgentRuntimeClient>,
    app: &mut App,
    sender: mpsc::Sender<AppEvent>,
    stream_handle: &mut Option<JoinHandle<()>>,
    bundle_ref: String,
) -> Result<(), String> {
    let metadata = client
        .select_bundle(bundle_ref.clone())
        .await
        .map_err(|err| err.to_string())?;
    refresh_bundles(client, app).await?;
    app.set_bundle_ref(bundle_ref.clone());
    app.set_active_model(metadata.agent_spec.model.name.clone());
    agent::refresh_agents(client, app)
        .await
        .map_err(|err| err.to_string())?;
    model::refresh_models(client, app)
        .await
        .map_err(|err| err.to_string())?;
    session::refresh_sessions(client, app)
        .await
        .map_err(|err| err.to_string())?;
    let skills = client.list_skills().await.map_err(|err| err.to_string())?;
    app.set_skills(skills);
    if !app.sessions.is_empty() {
        session::activate_selected_session(client, app, sender, stream_handle)
            .await
            .map_err(|err| err.to_string())?;
    } else {
        session::create_session(client, app, sender, stream_handle)
            .await
            .map_err(|err| err.to_string())?;
    }
    app.push_status(format!("bundle set: {bundle_ref}"));
    Ok(())
}
