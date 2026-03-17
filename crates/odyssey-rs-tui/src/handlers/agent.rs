//! Agent selection handlers.

use crate::app::App;
use crate::client::AgentRuntimeClient;
use log::debug;
use std::sync::Arc;

/// Activate the agent currently highlighted in the agents viewer.
pub fn activate_selected_agent(app: &mut App) -> anyhow::Result<()> {
    if let Some(agent_id) = app.agents.get(app.selected_agent).cloned() {
        app.set_active_agent(agent_id.clone());
        app.push_status(format!("agent set: {agent_id}"));
    } else {
        app.push_status("no agents available");
    }
    Ok(())
}

/// Fetch the current agent list for the selected bundle.
pub async fn refresh_agents(client: &Arc<AgentRuntimeClient>, app: &mut App) -> anyhow::Result<()> {
    if app.bundle_ref.trim().is_empty() {
        app.set_agents(Vec::new());
        return Ok(());
    }
    debug!("refreshing agents");
    let agents = client.list_agents().await?;
    app.set_agents(agents);
    Ok(())
}

/// Validate and activate an agent by id.
pub async fn set_agent_by_id(
    client: &Arc<AgentRuntimeClient>,
    app: &mut App,
    agent_id: String,
) -> Result<(), String> {
    if app.bundle_ref.trim().is_empty() {
        return Err("install a local bundle first".to_string());
    }
    let agents = client.list_agents().await.map_err(|err| err.to_string())?;
    if agents.is_empty() {
        return Err("no agents registered".to_string());
    }
    if !agents.contains(&agent_id) {
        return Err(format!("unknown agent: {agent_id}"));
    }
    app.set_agents(agents);
    app.set_active_agent(agent_id.clone());
    app.push_status(format!("agent set: {agent_id}"));
    Ok(())
}
