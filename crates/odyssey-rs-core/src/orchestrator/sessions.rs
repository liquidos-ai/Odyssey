//! In-memory session store with optional persistence via StateStore.

use crate::error::OdysseyCoreError;
use crate::state::{MessageRecord, StateStore};
use crate::types::{Message, Session, SessionId, SessionSummary};
use log::{debug, info};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// Session storage facade used by orchestrator and subagents.
#[derive(Clone)]
pub(crate) struct SessionStore {
    /// In-memory session cache.
    sessions: Arc<RwLock<HashMap<SessionId, Session>>>,
    /// Optional persistent store for sessions.
    state_store: Option<Arc<dyn StateStore>>,
}

impl SessionStore {
    /// Create a new session store with an optional backing store.
    pub(crate) fn new(state_store: Option<Arc<dyn StateStore>>) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            state_store,
        }
    }

    /// Expose the in-memory session map for internal handlers.
    pub(crate) fn sessions(&self) -> Arc<RwLock<HashMap<SessionId, Session>>> {
        self.sessions.clone()
    }

    /// Return the optional persistent store handle.
    pub(crate) fn state_store(&self) -> Option<Arc<dyn StateStore>> {
        self.state_store.clone()
    }

    /// Create a new session and persist it if configured.
    pub(crate) fn create_session(&self, agent_id: String) -> Result<SessionId, OdysseyCoreError> {
        let session = Session {
            id: Uuid::new_v4(),
            agent_id: agent_id.clone(),
            messages: Vec::new(),
            created_at: chrono::Utc::now(),
        };
        info!(
            "created session (session_id={}, agent_id={})",
            session.id, agent_id
        );

        if let Some(store) = &self.state_store {
            store
                .record_session(session.id, &session.agent_id, session.created_at)
                .map_err(|err| OdysseyCoreError::State(err.to_string()))?;
        }

        let session_id = session.id;
        self.sessions.write().insert(session.id, session);
        Ok(session_id)
    }

    /// Resume a session from cache or persistent store.
    pub(crate) fn resume_session(
        &self,
        session_id: SessionId,
    ) -> Result<Session, OdysseyCoreError> {
        if let Some(session) = self.sessions.read().get(&session_id).cloned() {
            return Ok(session);
        }

        if let Some(store) = &self.state_store
            && let Some(record) = store
                .load_session(session_id)
                .map_err(|err| OdysseyCoreError::State(err.to_string()))?
        {
            debug!("loaded session from store (session_id={})", session_id);
            let session = Session::from(record);
            self.sessions.write().insert(session_id, session.clone());
            return Ok(session);
        }

        Err(OdysseyCoreError::UnknownSession(session_id))
    }

    /// List all session summaries, using persistence when configured.
    pub(crate) fn list_sessions(&self) -> Result<Vec<SessionSummary>, OdysseyCoreError> {
        if let Some(store) = &self.state_store {
            let records = store
                .list_sessions()
                .map_err(|err| OdysseyCoreError::State(err.to_string()))?;
            return Ok(records.into_iter().map(SessionSummary::from).collect());
        }

        let mut summaries: Vec<SessionSummary> = self
            .sessions
            .read()
            .values()
            .map(|session| SessionSummary {
                id: session.id,
                agent_id: session.agent_id.clone(),
                message_count: session.messages.len(),
                created_at: session.created_at,
            })
            .collect();
        summaries.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(summaries)
    }

    /// Delete a session from cache and persistence.
    pub(crate) fn delete_session(&self, session_id: SessionId) -> Result<bool, OdysseyCoreError> {
        info!("deleting session (session_id={})", session_id);
        let mut removed = self.sessions.write().remove(&session_id).is_some();
        if let Some(store) = &self.state_store {
            let deleted = store
                .delete_session(session_id)
                .map_err(|err| OdysseyCoreError::State(err.to_string()))?;
            removed = removed || deleted;
        }
        Ok(removed)
    }

    /// Append a message to a session and persist it if configured.
    pub(crate) fn append_message(
        &self,
        session_id: SessionId,
        message: &Message,
    ) -> Result<(), OdysseyCoreError> {
        let mut sessions = self.sessions.write();
        let session = sessions
            .get_mut(&session_id)
            .ok_or(OdysseyCoreError::UnknownSession(session_id))?;
        debug!(
            "appending message (session_id={}, role={}, content_len={})",
            session_id,
            message.role.as_str(),
            message.content.len()
        );
        session.messages.push(message.clone());

        if let Some(store) = &self.state_store {
            let record = MessageRecord {
                role: message.role.as_str().to_string(),
                content: message.content.clone(),
                created_at: message.created_at,
            };
            store
                .append_message(session_id, &record)
                .map_err(|err| OdysseyCoreError::State(err.to_string()))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::SessionStore;
    use crate::state::JsonlStateStore;
    use crate::types::{Message, Role, Session};
    use pretty_assertions::assert_eq;
    use std::sync::Arc;
    use tempfile::tempdir;

    #[test]
    fn session_store_in_memory_lists_sessions() {
        let store = SessionStore::new(None);
        let session_id = store.create_session("agent".to_string()).expect("create");
        let summaries = store.list_sessions().expect("list");
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].id, session_id);
        assert_eq!(summaries[0].agent_id, "agent".to_string());
    }

    #[test]
    fn session_store_persists_and_resumes_sessions() {
        let root = tempdir().expect("root");
        let state = JsonlStateStore::new(root.path()).expect("state");
        let store = SessionStore::new(Some(Arc::new(state)));

        let session_id = store.create_session("agent".to_string()).expect("create");
        let message = Message {
            role: Role::User,
            content: "hello".to_string(),
            created_at: chrono::Utc::now(),
        };
        store.append_message(session_id, &message).expect("append");

        let store = SessionStore::new(Some(Arc::new(
            JsonlStateStore::new(root.path()).expect("state"),
        )));
        let session = store.resume_session(session_id).expect("resume");
        assert_eq!(
            session,
            Session {
                id: session_id,
                agent_id: "agent".to_string(),
                created_at: session.created_at,
                messages: vec![message],
            }
        );

        let summaries = store.list_sessions().expect("list");
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].message_count, 1);

        assert_eq!(store.delete_session(session_id).expect("delete"), true);
        let err = store.resume_session(session_id).expect_err("missing");
        match err {
            crate::error::OdysseyCoreError::UnknownSession(id) => assert_eq!(id, session_id),
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
