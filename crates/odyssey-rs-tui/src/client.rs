//! Local orchestrator client for the Odyssey TUI.

use crate::event::AppEvent;
use crate::event_bus::EventBus;
use anyhow::Result;
use log::{debug, info};
use odyssey_rs_core::Orchestrator;
use odyssey_rs_core::types::{Session, SessionSummary};
use odyssey_rs_protocol::{ApprovalDecision, SkillSummary};
use std::sync::Arc;
use tokio::sync::broadcast;
use uuid::Uuid;

/// Local client that wraps an embedded orchestrator.
#[derive(Clone)]
pub struct OrchestratorClient {
    orchestrator: Arc<Orchestrator>,
    events: EventBus,
}

impl OrchestratorClient {
    /// Create a new local client.
    pub fn new(orchestrator: Arc<Orchestrator>, events: EventBus) -> Self {
        Self {
            orchestrator,
            events,
        }
    }

    /// List available agent ids.
    pub async fn list_agents(&self) -> Result<Vec<String>> {
        Ok(self.orchestrator.list_agents())
    }

    /// List available sessions.
    pub async fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
        Ok(self.orchestrator.list_sessions()?)
    }

    /// Create a session, optionally for a specific agent.
    pub async fn create_session(&self, agent_id: Option<String>) -> Result<Uuid> {
        Ok(self.orchestrator.create_session(agent_id)?)
    }

    /// Fetch a session by id.
    pub async fn get_session(&self, session_id: Uuid) -> Result<Session> {
        Ok(self.orchestrator.resume_session(session_id)?)
    }

    /// Send a prompt to a session using the streaming path so that
    /// incremental deltas are emitted to the event bus in real time.
    pub async fn send_message(
        &self,
        session_id: Uuid,
        prompt: String,
        agent_id: Option<String>,
        llm_id: String,
    ) -> Result<odyssey_rs_core::RunResult> {
        if prompt.trim().is_empty() {
            anyhow::bail!("prompt cannot be empty");
        }
        let session = self.orchestrator.resume_session(session_id)?;
        let agent_id = if let Some(agent_id) = agent_id {
            if session.agent_id != agent_id {
                anyhow::bail!("agent_id does not match session agent");
            }
            agent_id
        } else {
            session.agent_id
        };

        debug!(
            "streaming session turn (session_id={}, agent_id={}, prompt_len={})",
            session_id,
            agent_id,
            prompt.len()
        );
        let run_stream = self
            .orchestrator
            .run_stream_in_session(session_id, &agent_id, &llm_id, prompt)
            .await?;
        Ok(run_stream.finish().await?)
    }

    /// Resolve a permission request.
    pub async fn resolve_permission(
        &self,
        request_id: Uuid,
        decision: ApprovalDecision,
    ) -> Result<bool> {
        Ok(self.orchestrator.resolve_approval(request_id, decision))
    }

    /// List skill summaries.
    pub async fn list_skills(&self) -> Result<Vec<SkillSummary>> {
        Ok(self.orchestrator.list_skill_summaries())
    }

    /// List registered model ids.
    pub async fn list_models(&self) -> Result<Vec<String>> {
        Ok(self.orchestrator.list_llm_ids())
    }

    /// Stream events for a session.
    pub async fn stream_events(
        &self,
        session_id: Uuid,
        sender: tokio::sync::mpsc::Sender<AppEvent>,
    ) -> Result<()> {
        let mut receiver = self.events.subscribe();
        info!(
            "subscribing to local event stream (session_id={})",
            session_id
        );
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    if event.session_id != session_id {
                        continue;
                    }
                    let _ = sender.send(AppEvent::Server(event)).await;
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
        info!("event stream closed (session_id={})", session_id);
        Ok(())
    }
}
