use crate::RuntimeError;
use autoagents_llm::chat::{ChatMessage, ChatRole, MessageType};
use autoagents_llm::{FunctionCall, ToolCall};
use chrono::{DateTime, Utc};
use log::warn;
use odyssey_rs_protocol::EventMsg;
use odyssey_rs_protocol::Task;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::NamedTempFile;
use tokio::sync::broadcast;
use uuid::Uuid;

#[derive(Clone)]
pub struct SessionStore {
    root: PathBuf,
    sessions: Arc<RwLock<HashMap<Uuid, SessionState>>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct SessionRecord {
    pub id: Uuid,
    pub bundle_ref: String,
    pub agent_id: String,
    #[serde(default = "default_model_provider")]
    pub model_provider: String,
    pub model_id: String,
    #[serde(default)]
    pub model_config: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub turns: Vec<TurnRecord>,
}

fn default_model_provider() -> String {
    "openai".to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct TurnRecord {
    pub turn_id: Uuid,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prompt: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub response: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chat_history: Vec<TurnChatMessageRecord>,
    pub created_at: DateTime<Utc>,
}

impl TurnRecord {
    pub(crate) fn from_history(
        turn_id: Uuid,
        task: &Task,
        response: impl Into<String>,
        chat_history: Vec<TurnChatMessageRecord>,
        created_at: DateTime<Utc>,
    ) -> Self {
        let mut record = Self {
            turn_id,
            prompt: task.prompt.clone(),
            response: response.into(),
            chat_history,
            created_at,
        };
        let _ = record.normalize();
        record
    }

    pub(crate) fn normalize(&mut self) -> bool {
        if self.chat_history.is_empty() {
            return false;
        }
        let changed = !self.prompt.is_empty() || !self.response.is_empty();
        self.prompt.clear();
        self.response.clear();
        changed
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TurnChatMessageKind {
    #[default]
    Text,
    ToolUse,
    ToolResult,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct TurnToolCallRecord {
    pub id: String,
    #[serde(default = "default_tool_call_type")]
    pub call_type: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct TurnChatMessageRecord {
    pub role: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub kind: TurnChatMessageKind,
    #[serde(default)]
    pub tool_calls: Vec<TurnToolCallRecord>,
}

fn default_tool_call_type() -> String {
    "function".to_string()
}

impl TurnChatMessageRecord {
    pub(crate) fn from_text(role: ChatRole, content: impl Into<String>) -> Self {
        Self {
            role: role.to_string(),
            content: content.into(),
            kind: TurnChatMessageKind::Text,
            tool_calls: Vec::new(),
        }
    }

    pub(crate) fn from_tool_calls(
        role: ChatRole,
        kind: TurnChatMessageKind,
        calls: Vec<ToolCall>,
    ) -> Self {
        Self {
            role: role.to_string(),
            content: String::default(),
            kind,
            tool_calls: calls
                .into_iter()
                .map(|call| TurnToolCallRecord {
                    id: call.id,
                    call_type: call.call_type,
                    name: call.function.name,
                    arguments: call.function.arguments,
                })
                .collect(),
        }
    }

    pub(crate) fn into_chat_message(self) -> ChatMessage {
        let role = match self.role.as_str() {
            "assistant" => ChatRole::Assistant,
            "system" => ChatRole::System,
            "tool" => ChatRole::Tool,
            _ => ChatRole::User,
        };
        let tool_calls = self
            .tool_calls
            .into_iter()
            .map(|call| ToolCall {
                id: call.id,
                call_type: call.call_type,
                function: FunctionCall {
                    name: call.name,
                    arguments: call.arguments,
                },
            })
            .collect::<Vec<_>>();
        let message_type = match self.kind {
            TurnChatMessageKind::Text => MessageType::Text,
            TurnChatMessageKind::ToolUse => MessageType::ToolUse(tool_calls),
            TurnChatMessageKind::ToolResult => MessageType::ToolResult(tool_calls),
        };
        let content = if matches!(message_type, MessageType::Text) {
            self.content
        } else {
            String::default()
        };
        ChatMessage {
            role,
            message_type,
            content,
        }
    }
}

#[derive(Clone)]
struct SessionState {
    record: SessionRecord,
    sender: broadcast::Sender<EventMsg>,
}

impl SessionStore {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, RuntimeError> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(|err| RuntimeError::Io {
            path: root.display().to_string(),
            message: err.to_string(),
        })?;
        let sessions = load_sessions(&root)?;
        Ok(Self {
            root,
            sessions: Arc::new(RwLock::new(sessions)),
        })
    }

    pub fn create(
        &self,
        bundle_ref: String,
        agent_id: String,
        model_provider: String,
        model_id: String,
        model_config: Option<Value>,
    ) -> Result<SessionRecord, RuntimeError> {
        let id = Uuid::new_v4();
        let record = SessionRecord {
            id,
            bundle_ref,
            agent_id,
            model_provider,
            model_id,
            model_config,
            created_at: Utc::now(),
            turns: Vec::new(),
        };
        self.persist(&record)?;
        let (sender, _) = broadcast::channel(512);
        self.sessions.write().insert(
            id,
            SessionState {
                record: record.clone(),
                sender,
            },
        );
        Ok(record)
    }

    pub fn get(&self, id: Uuid) -> Result<SessionRecord, RuntimeError> {
        self.sessions
            .read()
            .get(&id)
            .map(|state| state.record.clone())
            .ok_or_else(|| RuntimeError::UnknownSession(id.to_string()))
    }

    pub fn list(&self) -> Vec<SessionRecord> {
        let mut records = self
            .sessions
            .read()
            .values()
            .map(|state| state.record.clone())
            .collect::<Vec<_>>();
        records.sort_by_key(|record| record.created_at);
        records.reverse();
        records
    }

    pub fn subscribe(&self, id: Uuid) -> Result<broadcast::Receiver<EventMsg>, RuntimeError> {
        self.sessions
            .read()
            .get(&id)
            .map(|state| state.sender.subscribe())
            .ok_or_else(|| RuntimeError::UnknownSession(id.to_string()))
    }

    pub fn sender(&self, id: Uuid) -> Result<broadcast::Sender<EventMsg>, RuntimeError> {
        self.sessions
            .read()
            .get(&id)
            .map(|state| state.sender.clone())
            .ok_or_else(|| RuntimeError::UnknownSession(id.to_string()))
    }

    pub fn append_turn(&self, id: Uuid, turn: TurnRecord) -> Result<(), RuntimeError> {
        let mut sessions = self.sessions.write();
        let state = sessions
            .get_mut(&id)
            .ok_or_else(|| RuntimeError::UnknownSession(id.to_string()))?;
        let mut updated = state.record.clone();
        updated.turns.push(turn);
        self.persist(&updated)?;
        state.record = updated;
        Ok(())
    }

    pub fn delete(&self, id: Uuid) -> Result<(), RuntimeError> {
        let record = self
            .sessions
            .read()
            .get(&id)
            .map(|state| state.record.clone())
            .ok_or_else(|| RuntimeError::UnknownSession(id.to_string()))?;
        let path = self.root.join(format!("{}.json", record.id));
        if path.exists() {
            fs::remove_file(&path).map_err(|err| RuntimeError::Io {
                path: path.display().to_string(),
                message: err.to_string(),
            })?;
            if let Some(parent) = path.parent() {
                sync_directory(parent)?;
            }
        }
        self.sessions.write().remove(&id);
        Ok(())
    }

    fn persist(&self, record: &SessionRecord) -> Result<(), RuntimeError> {
        let path = self.root.join(format!("{}.json", record.id));
        persist_record(&path, record)
    }
}

fn persist_record(path: &Path, record: &SessionRecord) -> Result<(), RuntimeError> {
    let parent = path.parent().ok_or_else(|| RuntimeError::Io {
        path: path.display().to_string(),
        message: "session path must have a parent directory".to_string(),
    })?;
    let bytes =
        serde_json::to_vec_pretty(record).map_err(|err| RuntimeError::Executor(err.to_string()))?;

    let mut temp = NamedTempFile::new_in(parent).map_err(|err| RuntimeError::Io {
        path: parent.display().to_string(),
        message: err.to_string(),
    })?;
    temp.write_all(&bytes).map_err(|err| RuntimeError::Io {
        path: temp.path().display().to_string(),
        message: err.to_string(),
    })?;
    temp.as_file_mut()
        .sync_all()
        .map_err(|err| RuntimeError::Io {
            path: temp.path().display().to_string(),
            message: err.to_string(),
        })?;
    temp.persist(path).map_err(|err| RuntimeError::Io {
        path: path.display().to_string(),
        message: err.error.to_string(),
    })?;
    sync_directory(parent)
}

fn normalize_session_record(record: &mut SessionRecord) -> bool {
    let mut changed = false;
    for turn in &mut record.turns {
        if turn.normalize() {
            changed = true;
        }
    }
    changed
}

fn load_sessions(root: &Path) -> Result<HashMap<Uuid, SessionState>, RuntimeError> {
    let mut sessions = HashMap::new();
    for entry in fs::read_dir(root).map_err(|err| RuntimeError::Io {
        path: root.display().to_string(),
        message: err.to_string(),
    })? {
        let entry = entry.map_err(|err| RuntimeError::Io {
            path: root.display().to_string(),
            message: err.to_string(),
        })?;
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Some(record) = load_session_record(&path)? else {
            continue;
        };
        let (sender, _) = broadcast::channel(512);
        sessions.insert(record.id, SessionState { record, sender });
    }
    Ok(sessions)
}

fn load_session_record(path: &Path) -> Result<Option<SessionRecord>, RuntimeError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) => {
            warn!(
                "skipping unreadable session record {}: {}",
                path.display(),
                err
            );
            if let Err(quarantine_error) = quarantine_corrupt_session(path) {
                warn!(
                    "failed to quarantine unreadable session record {}: {}",
                    path.display(),
                    quarantine_error
                );
            }
            return Ok(None);
        }
    };

    let mut record: SessionRecord = match serde_json::from_slice(&bytes) {
        Ok(record) => record,
        Err(err) => {
            warn!(
                "skipping corrupt session record {}: {}",
                path.display(),
                err
            );
            if let Err(quarantine_error) = quarantine_corrupt_session(path) {
                warn!(
                    "failed to quarantine corrupt session record {}: {}",
                    path.display(),
                    quarantine_error
                );
            }
            return Ok(None);
        }
    };

    if normalize_session_record(&mut record)
        && let Err(err) = persist_record(path, &record)
    {
        warn!(
            "failed to persist normalized session record {}: {}",
            path.display(),
            err
        );
    }

    Ok(Some(record))
}

fn quarantine_corrupt_session(path: &Path) -> Result<(), RuntimeError> {
    let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
        return Ok(());
    };
    let quarantine = path.with_file_name(format!("{file_name}.corrupt-{}", Uuid::new_v4()));
    fs::rename(path, &quarantine).map_err(|err| RuntimeError::Io {
        path: quarantine.display().to_string(),
        message: err.to_string(),
    })
}

fn sync_directory(path: &Path) -> Result<(), RuntimeError> {
    #[cfg(unix)]
    {
        let directory = fs::File::open(path).map_err(|err| RuntimeError::Io {
            path: path.display().to_string(),
            message: err.to_string(),
        })?;
        directory.sync_all().map_err(|err| RuntimeError::Io {
            path: path.display().to_string(),
            message: err.to_string(),
        })?;
    }

    #[cfg(not(unix))]
    {
        let _ = path;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{SessionRecord, SessionStore, TurnChatMessageRecord, TurnRecord};
    use autoagents_llm::chat::ChatRole;
    use chrono::Utc;
    use odyssey_rs_protocol::Task;
    use pretty_assertions::assert_eq;
    use serde_json::{Value, json};
    use std::fs;
    use tempfile::tempdir;
    use uuid::Uuid;

    #[test]
    fn new_loads_persisted_sessions_from_disk() {
        let temp = tempdir().expect("tempdir");
        let record = SessionRecord {
            id: Uuid::new_v4(),
            bundle_ref: "odyssey-cowork@latest".to_string(),
            agent_id: "odyssey-cowork".to_string(),
            model_provider: "openai".to_string(),
            model_id: "gpt-4.1-mini".to_string(),
            model_config: None,
            created_at: Utc::now(),
            turns: vec![TurnRecord {
                turn_id: Uuid::new_v4(),
                prompt: "hello".to_string(),
                response: "world".to_string(),
                chat_history: vec![
                    TurnChatMessageRecord::from_text(ChatRole::User, "hello"),
                    TurnChatMessageRecord::from_text(ChatRole::Assistant, "world"),
                ],
                created_at: Utc::now(),
            }],
        };
        fs::write(
            temp.path().join(format!("{}.json", record.id)),
            serde_json::to_vec_pretty(&record).expect("serialize"),
        )
        .expect("write session");

        let store = SessionStore::new(temp.path()).expect("store");
        let loaded = store.get(record.id).expect("load session");

        assert_eq!(loaded.id, record.id);
        assert_eq!(loaded.bundle_ref, record.bundle_ref);
        assert_eq!(loaded.turns.len(), 1);
        assert_eq!(loaded.turns[0].prompt, "");
        assert_eq!(loaded.turns[0].response, "");
        assert_eq!(store.list().len(), 1);

        let persisted = fs::read_to_string(temp.path().join(format!("{}.json", record.id)))
            .expect("read session");
        let json: Value = serde_json::from_str(&persisted).expect("parse session json");
        let turn = &json["turns"][0];
        assert_eq!(turn.get("prompt"), None);
        assert_eq!(turn.get("response"), None);
    }

    #[test]
    fn append_turn_omits_duplicated_prompt_and_response_when_history_exists() {
        let temp = tempdir().expect("tempdir");
        let store = SessionStore::new(temp.path()).expect("store");
        let session = store
            .create(
                "odyssey-cowork@latest".to_string(),
                "odyssey-cowork".to_string(),
                "openai".to_string(),
                "gpt-4.1-mini".to_string(),
                None,
            )
            .expect("session");
        let turn = TurnRecord::from_history(
            Uuid::new_v4(),
            &Task::new("hello"),
            "world",
            vec![
                TurnChatMessageRecord::from_text(ChatRole::User, "hello"),
                TurnChatMessageRecord::from_text(ChatRole::Assistant, "world"),
            ],
            Utc::now(),
        );

        store.append_turn(session.id, turn).expect("append");

        let persisted = fs::read_to_string(temp.path().join(format!("{}.json", session.id)))
            .expect("read session");
        let json: Value = serde_json::from_str(&persisted).expect("parse session json");
        let turn = &json["turns"][0];
        assert_eq!(turn.get("prompt"), None);
        assert_eq!(turn.get("response"), None);
        assert_eq!(turn["chat_history"][0]["content"], "hello");
        assert_eq!(turn["chat_history"][1]["content"], "world");
    }

    #[test]
    fn create_persists_model_config() {
        let temp = tempdir().expect("tempdir");
        let store = SessionStore::new(temp.path()).expect("store");
        let session = store
            .create(
                "odyssey-cowork@latest".to_string(),
                "odyssey-cowork".to_string(),
                "openai".to_string(),
                "gpt-5".to_string(),
                Some(json!({ "reasoning_effort": "high" })),
            )
            .expect("session");

        let loaded = store.get(session.id).expect("load session");

        assert_eq!(
            loaded.model_config,
            Some(json!({ "reasoning_effort": "high" }))
        );
    }

    #[test]
    fn new_skips_and_quarantines_corrupt_session_files() {
        let temp = tempdir().expect("tempdir");
        let good = SessionRecord {
            id: Uuid::new_v4(),
            bundle_ref: "odyssey-cowork@latest".to_string(),
            agent_id: "odyssey-cowork".to_string(),
            model_provider: "openai".to_string(),
            model_id: "gpt-4.1-mini".to_string(),
            model_config: None,
            created_at: Utc::now(),
            turns: Vec::new(),
        };
        fs::write(
            temp.path().join(format!("{}.json", good.id)),
            serde_json::to_vec_pretty(&good).expect("serialize"),
        )
        .expect("write good session");
        fs::write(temp.path().join("broken.json"), "{not valid json").expect("write corrupt");

        let store = SessionStore::new(temp.path()).expect("store");

        assert_eq!(store.list().len(), 1);
        assert_eq!(store.get(good.id).expect("good session").id, good.id);
        assert!(!temp.path().join("broken.json").exists());
        assert!(
            fs::read_dir(temp.path())
                .expect("read dir")
                .filter_map(Result::ok)
                .any(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with("broken.json.corrupt-")
                })
        );
    }

    #[test]
    fn create_does_not_mutate_memory_when_persist_fails() {
        let temp = tempdir().expect("tempdir");
        let store = SessionStore::new(temp.path()).expect("store");
        fs::remove_dir_all(temp.path()).expect("remove backing directory");

        let error = store
            .create(
                "odyssey-cowork@latest".to_string(),
                "odyssey-cowork".to_string(),
                "openai".to_string(),
                "gpt-4.1-mini".to_string(),
                None,
            )
            .expect_err("persist failure");

        assert!(error.to_string().contains("io error"));
        assert!(store.list().is_empty());
    }

    #[test]
    fn append_turn_does_not_mutate_memory_when_persist_fails() {
        let temp = tempdir().expect("tempdir");
        let store = SessionStore::new(temp.path()).expect("store");
        let session = store
            .create(
                "odyssey-cowork@latest".to_string(),
                "odyssey-cowork".to_string(),
                "openai".to_string(),
                "gpt-4.1-mini".to_string(),
                None,
            )
            .expect("session");
        fs::remove_dir_all(temp.path()).expect("remove backing directory");

        let error = store
            .append_turn(
                session.id,
                TurnRecord {
                    turn_id: Uuid::new_v4(),
                    prompt: "hello".to_string(),
                    response: "world".to_string(),
                    chat_history: Vec::new(),
                    created_at: Utc::now(),
                },
            )
            .expect_err("persist failure");

        assert!(error.to_string().contains("io error"));
        assert!(store.get(session.id).expect("session").turns.is_empty());
    }
}
