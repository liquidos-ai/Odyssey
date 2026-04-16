//! Session persistence for Odyssey using JSONL rollouts.

use crate::types::SessionId;
use chrono::{DateTime, Utc};
use log::{debug, info, warn};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;
use uuid::Uuid;

/// Persisted message record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MessageRecord {
    /// Role name.
    pub role: String,
    /// Message content.
    pub content: String,
    /// Timestamp for the message.
    pub created_at: DateTime<Utc>,
}

/// Persisted session record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionRecord {
    /// Session identifier.
    pub id: SessionId,
    /// Agent id for the session.
    pub agent_id: String,
    /// Session creation timestamp.
    pub created_at: DateTime<Utc>,
    /// All messages in the session.
    pub messages: Vec<MessageRecord>,
}

/// Summary record used for listing sessions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionSummaryRecord {
    /// Session identifier.
    pub id: SessionId,
    /// Agent id for the session.
    pub agent_id: String,
    /// Total number of messages.
    pub message_count: usize,
    /// Session creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Timestamp of the most recent message.
    pub updated_at: DateTime<Utc>,
}

/// Persistent store abstraction for sessions and messages.
pub trait StateStore: Send + Sync {
    /// Record a new session creation.
    fn record_session(
        &self,
        session_id: SessionId,
        agent_id: &str,
        created_at: DateTime<Utc>,
    ) -> Result<(), StateError>;
    /// Append a message to a session.
    fn append_message(
        &self,
        session_id: SessionId,
        message: &MessageRecord,
    ) -> Result<(), StateError>;
    /// Load a session record by id.
    fn load_session(&self, session_id: SessionId) -> Result<Option<SessionRecord>, StateError>;
    /// List all session summaries.
    fn list_sessions(&self) -> Result<Vec<SessionSummaryRecord>, StateError>;
    /// Delete a session and its backing storage.
    fn delete_session(&self, session_id: SessionId) -> Result<bool, StateError>;
}

/// Errors returned by the state store.
#[derive(Debug, Error)]
pub enum StateError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("unsupported schema version: {0}")]
    UnsupportedSchema(u32),
    #[error("missing session metadata")]
    MissingMetadata,
    #[error("session already exists: {0}")]
    SessionExists(SessionId),
}

/// Internal JSONL event representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum RolloutEvent {
    //TODO: The rollout should also contain tool call and tool results
    SchemaVersion {
        version: u32,
    },
    SessionCreated {
        session_id: SessionId,
        agent_id: String,
        created_at: DateTime<Utc>,
    },
    Message {
        session_id: SessionId,
        role: String,
        content: String,
        created_at: DateTime<Utc>,
    },
}

#[derive(Default)]
struct RolloutState {
    version: Option<u32>,
    agent_id: Option<String>,
    created_at: Option<DateTime<Utc>>,
    messages: Vec<MessageRecord>,
}

impl RolloutState {
    fn apply(&mut self, event: RolloutEvent) -> Result<(), StateError> {
        match event {
            RolloutEvent::SchemaVersion { version } => {
                self.version = Some(version);
                if version > 1 {
                    return Err(StateError::UnsupportedSchema(version));
                }
            }
            RolloutEvent::SessionCreated {
                agent_id,
                created_at,
                ..
            } => {
                self.agent_id = Some(agent_id);
                self.created_at = Some(created_at);
            }
            RolloutEvent::Message {
                role,
                content,
                created_at,
                ..
            } => {
                self.messages.push(MessageRecord {
                    role,
                    content,
                    created_at,
                });
            }
        }
        Ok(())
    }

    fn finish(self, session_id: SessionId) -> Result<SessionRecord, StateError> {
        let _ = self.version.ok_or(StateError::MissingMetadata)?;
        let agent_id = self.agent_id.ok_or(StateError::MissingMetadata)?;
        let created_at = self.created_at.ok_or(StateError::MissingMetadata)?;
        Ok(SessionRecord {
            id: session_id,
            agent_id,
            created_at,
            messages: self.messages,
        })
    }
}

/// JSONL-backed state store implementation.
pub struct JsonlStateStore {
    /// Root directory for session rollouts.
    root: PathBuf,
    /// Serialize write access to rollout files.
    write_lock: Mutex<()>,
}

impl JsonlStateStore {
    /// Create a new JSONL store under the given root.
    pub fn new(root: impl AsRef<Path>) -> Result<Self, StateError> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)?;
        info!("initialized JSONL state store (root={})", root.display());
        Ok(Self {
            root,
            write_lock: Mutex::new(()),
        })
    }

    /// Build the rollout file path for a session.
    fn rollout_path(&self, session_id: SessionId) -> PathBuf {
        self.root.join(format!("{session_id}.jsonl"))
    }

    /// Append an event to an existing rollout file.
    fn write_event(&self, session_id: SessionId, event: &RolloutEvent) -> Result<(), StateError> {
        let _guard = self.write_lock.lock();
        let path = self.rollout_path(session_id);
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        let line = serde_json::to_string(event)?;
        writeln!(file, "{line}")?;
        Ok(())
    }

    /// Create a new rollout file and write the initial event.
    fn write_new_rollout(
        &self,
        session_id: SessionId,
        event: &RolloutEvent,
    ) -> Result<(), StateError> {
        let _guard = self.write_lock.lock();
        let path = self.rollout_path(session_id);
        if path.exists() {
            return Err(StateError::SessionExists(session_id));
        }
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)?;
        let header = serde_json::to_string(&RolloutEvent::SchemaVersion { version: 1 })?;
        writeln!(file, "{header}")?;
        let line = serde_json::to_string(event)?;
        writeln!(file, "{line}")?;
        Ok(())
    }

    /// Read and reconstruct a session from its rollout file.
    fn read_rollout(&self, session_id: SessionId) -> Result<Option<SessionRecord>, StateError> {
        let path = self.rollout_path(session_id);
        if !path.exists() {
            return Ok(None);
        }
        let file = OpenOptions::new().read(true).open(&path)?;
        let reader = BufReader::new(file);
        let mut rollout = RolloutState::default();

        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let event: RolloutEvent = serde_json::from_str(&line)?;
            rollout.apply(event)?;
        }
        Ok(Some(rollout.finish(session_id)?))
    }
}

impl StateStore for JsonlStateStore {
    /// Record session creation as a rollout event.
    fn record_session(
        &self,
        session_id: SessionId,
        agent_id: &str,
        created_at: DateTime<Utc>,
    ) -> Result<(), StateError> {
        info!(
            "recording session creation (session_id={}, agent_id={})",
            session_id, agent_id
        );
        let event = RolloutEvent::SessionCreated {
            session_id,
            agent_id: agent_id.to_string(),
            created_at,
        };
        self.write_new_rollout(session_id, &event)
    }

    /// Append a message event to a session rollout.
    fn append_message(
        &self,
        session_id: SessionId,
        message: &MessageRecord,
    ) -> Result<(), StateError> {
        debug!(
            "appending message event (session_id={}, role={}, content_len={})",
            session_id,
            message.role,
            message.content.len()
        );
        let event = RolloutEvent::Message {
            session_id,
            role: message.role.clone(),
            content: message.content.clone(),
            created_at: message.created_at,
        };
        self.write_event(session_id, &event)
    }

    /// Load a session from the rollout file.
    fn load_session(&self, session_id: SessionId) -> Result<Option<SessionRecord>, StateError> {
        self.read_rollout(session_id)
    }

    /// List all sessions by scanning rollout files.
    fn list_sessions(&self) -> Result<Vec<SessionSummaryRecord>, StateError> {
        let mut summaries = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
                continue;
            }
            let file_name = match path.file_stem().and_then(|stem| stem.to_str()) {
                Some(name) => name,
                None => continue,
            };
            let session_id = match Uuid::parse_str(file_name) {
                Ok(id) => id,
                Err(_) => continue,
            };
            if let Some(record) = self.read_rollout(session_id)? {
                let updated_at = record
                    .messages
                    .last()
                    .map(|msg| msg.created_at)
                    .unwrap_or(record.created_at);
                summaries.push(SessionSummaryRecord {
                    id: record.id,
                    agent_id: record.agent_id,
                    message_count: record.messages.len(),
                    created_at: record.created_at,
                    updated_at,
                });
            }
        }
        summaries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(summaries)
    }

    /// Delete the rollout file for a session.
    fn delete_session(&self, session_id: SessionId) -> Result<bool, StateError> {
        let path = self.rollout_path(session_id);
        if path.exists() {
            info!("deleting session rollout (session_id={})", session_id);
            fs::remove_file(path)?;
            Ok(true)
        } else {
            warn!("session rollout not found (session_id={})", session_id);
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{JsonlStateStore, MessageRecord, SessionRecord, SessionSummaryRecord, StateStore};
    use chrono::Utc;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;
    use uuid::Uuid;

    #[test]
    fn jsonl_state_store_round_trip() {
        let temp = tempdir().expect("tempdir");
        let store = JsonlStateStore::new(temp.path()).expect("store");
        let session_id = Uuid::new_v4();
        let created_at = Utc::now();
        store
            .record_session(session_id, "agent", created_at)
            .expect("record session");

        let message = MessageRecord {
            role: "user".to_string(),
            content: "hello".to_string(),
            created_at,
        };
        store
            .append_message(session_id, &message)
            .expect("append message");

        let record = store
            .load_session(session_id)
            .expect("load")
            .expect("record");
        let expected = SessionRecord {
            id: session_id,
            agent_id: "agent".to_string(),
            created_at,
            messages: vec![message.clone()],
        };
        assert_eq!(record, expected);

        let summaries = store.list_sessions().expect("summaries");
        let expected_summary = SessionSummaryRecord {
            id: session_id,
            agent_id: "agent".to_string(),
            message_count: 1,
            created_at,
            updated_at: created_at,
        };
        assert_eq!(summaries, vec![expected_summary]);

        assert_eq!(store.delete_session(session_id).expect("delete"), true);
        assert_eq!(
            store.load_session(session_id).expect("load after delete"),
            None
        );
    }
}
