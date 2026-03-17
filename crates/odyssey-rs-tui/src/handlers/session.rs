//! Session lifecycle: create, join, activate, refresh, and send messages.

use crate::app::App;
use crate::client::AgentRuntimeClient;
use crate::event::AppEvent;
use crate::spawn::{spawn_send_message, spawn_stream};
use log::{debug, info, warn};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use uuid::Uuid;

/// Create a new session and start streaming its events.
pub async fn create_session(
    client: &Arc<AgentRuntimeClient>,
    app: &mut App,
    sender: mpsc::Sender<AppEvent>,
    stream_handle: &mut Option<JoinHandle<()>>,
) -> anyhow::Result<()> {
    if app.bundle_ref.trim().is_empty() {
        app.push_status("install a local bundle first");
        return Ok(());
    }
    let agent_id = app
        .active_agent
        .clone()
        .or_else(|| app.agents.first().cloned());
    info!(
        "creating session (agent_id={})",
        agent_id.as_deref().unwrap_or("default")
    );
    let session_id = client.create_session(agent_id.clone()).await?;
    refresh_sessions(client, app).await?;
    app.select_session(session_id);
    if let Some(agent_id) = agent_id {
        app.set_active_session(session_id, agent_id);
    } else if let Ok(session) = client.get_session(session_id).await {
        app.set_active_session(session.id, session.agent_id);
    }
    app.push_status("session created");
    spawn_stream(client.clone(), session_id, sender, stream_handle);
    Ok(())
}

/// Join an existing session by id and load its full transcript.
pub async fn join_session(
    client: &Arc<AgentRuntimeClient>,
    app: &mut App,
    session_id: Uuid,
    sender: mpsc::Sender<AppEvent>,
    stream_handle: &mut Option<JoinHandle<()>>,
) -> anyhow::Result<()> {
    info!("joining session (session_id={})", session_id);
    refresh_sessions(client, app).await?;
    app.select_session(session_id);
    let session = client.get_session(session_id).await?;
    app.set_active_session(session.id, session.agent_id);
    app.load_messages(session.messages);
    app.push_status("session joined");
    spawn_stream(client.clone(), session_id, sender, stream_handle);
    Ok(())
}

/// Activate the session currently highlighted in the sessions viewer.
pub async fn activate_selected_session(
    client: &Arc<AgentRuntimeClient>,
    app: &mut App,
    sender: mpsc::Sender<AppEvent>,
    stream_handle: &mut Option<JoinHandle<()>>,
) -> anyhow::Result<()> {
    if let Some(session) = app.sessions.get(app.selected_session).cloned() {
        let session_id = session.id;
        let agent_id = session.agent_id;
        info!("activating session (session_id={})", session_id);
        app.select_session(session_id);
        app.set_active_session(session_id, agent_id);
        if let Ok(session_detail) = client.get_session(session_id).await {
            app.load_messages(session_detail.messages);
        }
        app.push_status("session selected");
        spawn_stream(client.clone(), session_id, sender, stream_handle);
    }
    Ok(())
}

/// Fetch the current session list from the orchestrator and update the app.
pub async fn refresh_sessions(
    client: &Arc<AgentRuntimeClient>,
    app: &mut App,
) -> anyhow::Result<()> {
    debug!("refreshing sessions");
    match client.list_sessions().await {
        Ok(sessions) => app.set_sessions(sessions),
        Err(err) => warn!("failed to refresh sessions: {err}"),
    }
    Ok(())
}

/// Take the current input, send it to the active session, and spawn a task.
pub async fn send_message(
    client: &Arc<AgentRuntimeClient>,
    app: &mut App,
    sender: mpsc::Sender<AppEvent>,
) -> anyhow::Result<()> {
    let session_id = match app.active_session {
        Some(id) => id,
        None => {
            app.push_status("no active session");
            return Ok(());
        }
    };
    let prompt = std::mem::take(&mut app.input);
    info!(
        "sending message (session_id={}, prompt_len={})",
        session_id,
        prompt.len()
    );
    app.push_user_message(prompt.clone());
    app.enable_auto_scroll();
    let agent_id = app.active_agent.clone();
    let llm_id = app.model_id.clone();
    app.push_status("running");
    spawn_send_message(client.clone(), session_id, prompt, agent_id, llm_id, sender);
    Ok(())
}
