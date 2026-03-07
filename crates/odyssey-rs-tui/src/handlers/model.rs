//! Model selection handlers.

use crate::app::App;
use crate::client::AgentRuntimeClient;
use log::debug;
use std::sync::Arc;

/// Activate the model currently highlighted in the models viewer.
pub fn activate_selected_model(app: &mut App) -> anyhow::Result<()> {
    if let Some(model_id) = app.models.get(app.selected_model).cloned() {
        app.set_active_model(model_id.clone());
        app.push_status(format!("model set: {model_id}"));
    } else {
        app.push_status("no models available");
    }
    Ok(())
}

/// Fetch the current model list from the orchestrator and update the app.
pub async fn refresh_models(client: &Arc<AgentRuntimeClient>, app: &mut App) -> anyhow::Result<()> {
    debug!("refreshing models");
    let mut models = client.list_models().await?;
    models.sort();
    app.set_models(models);
    Ok(())
}

/// Look up a model by id, validate it exists, then make it active.
pub async fn set_model_by_id(
    client: &Arc<AgentRuntimeClient>,
    app: &mut App,
    model_id: String,
) -> Result<(), String> {
    let mut models = client.list_models().await.map_err(|e| e.to_string())?;
    if models.is_empty() {
        return Err("no models registered".to_string());
    }
    models.sort();
    if !models.contains(&model_id) {
        return Err(format!("unknown model: {model_id}"));
    }
    app.set_models(models);
    app.set_active_model(model_id.clone());
    app.push_status(format!("model set: {model_id}"));
    Ok(())
}
