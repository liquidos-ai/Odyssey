use super::scheduler::ExecutionScheduler;
use super::templates::initialize_bundle;
use crate::resolver::agent::resolve_agent;
use crate::sandbox::build_sandbox_runtime;
use crate::session::{ApprovalStore, SessionRecord, SessionStore, TurnChatMessageKind};
use crate::skill::BundleSkillStore;
use crate::{RuntimeConfig, RuntimeError};
use log::info;
use odyssey_rs_bundle::{
    BundleArtifact, BundleBuilder, BundleInstall, BundleMetadata, BundleProject, BundleStore,
};
use odyssey_rs_protocol::SandboxMode;
use odyssey_rs_protocol::{
    AgentRef, EventMsg, ExecutionHandle, ExecutionRequest, ExecutionStatus, Message, Role, Session,
    SessionFilter, SessionSpec, SessionSummary, SkillSummary,
};
use odyssey_rs_sandbox::SandboxRuntime;
use odyssey_rs_tools::{SkillProvider, ToolRegistry, builtin_registry};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::broadcast;
use uuid::Uuid;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RunOutput {
    pub session_id: Uuid,
    pub turn_id: Uuid,
    pub response: String,
}

pub(crate) struct OdysseyRuntimeInner {
    pub(crate) config: RuntimeConfig,
    pub(crate) store: BundleStore,
    pub(crate) sessions: SessionStore,
    pub(crate) host_sandbox: Arc<SandboxRuntime>,
    pub(crate) tools: ToolRegistry,
    pub(crate) approvals: ApprovalStore,
}

#[derive(Clone)]
pub struct OdysseyRuntime {
    inner: Arc<OdysseyRuntimeInner>,
    scheduler: ExecutionScheduler,
}

impl OdysseyRuntime {
    pub fn new(config: RuntimeConfig) -> Result<Self, RuntimeError> {
        let store = BundleStore::new(config.cache_root.clone());
        let sessions = SessionStore::new(config.session_root.clone())?;

        //Host Sandbox can be shared for all agent executions instead of per agent execution
        let host_sandbox = Arc::new(build_sandbox_runtime(
            &config,
            SandboxMode::DangerFullAccess,
        )?);
        let worker_count = config.worker_count;
        let queue_capacity = config.queue_capacity;
        let inner = Arc::new(OdysseyRuntimeInner {
            config,
            store,
            sessions,
            host_sandbox,
            tools: builtin_registry(),
            approvals: ApprovalStore::new(),
        });
        let scheduler = ExecutionScheduler::new(inner.clone(), worker_count, queue_capacity);
        info!("OdysseyRuntime Initiated");
        Ok(Self { inner, scheduler })
    }

    pub fn config(&self) -> &RuntimeConfig {
        &self.inner.config
    }

    pub fn bundle_store(&self) -> BundleStore {
        self.inner.store.clone()
    }

    pub fn init(&self, root: impl AsRef<Path>) -> Result<(), RuntimeError> {
        initialize_bundle(root.as_ref())
    }

    pub fn build_and_install(
        &self,
        project_root: impl AsRef<Path>,
    ) -> Result<BundleInstall, RuntimeError> {
        self.inner
            .store
            .build_and_install(project_root)
            .map_err(RuntimeError::from)
    }

    pub fn build_to(
        &self,
        project_root: impl AsRef<Path>,
        output_root: impl AsRef<Path>,
    ) -> Result<BundleArtifact, RuntimeError> {
        let project = BundleProject::load(project_root.as_ref().to_path_buf())?;
        BundleBuilder::new(project)
            .build(output_root)
            .map_err(RuntimeError::from)
    }

    pub fn inspect_bundle(&self, reference: &str) -> Result<BundleMetadata, RuntimeError> {
        Ok(self.inner.store.resolve(reference)?.metadata)
    }

    pub fn export_bundle(
        &self,
        reference: &str,
        output: impl AsRef<Path>,
    ) -> Result<std::path::PathBuf, RuntimeError> {
        self.inner
            .store
            .export(reference, output)
            .map_err(RuntimeError::from)
    }

    pub fn import_bundle(
        &self,
        archive_path: impl AsRef<Path>,
    ) -> Result<BundleInstall, RuntimeError> {
        self.inner
            .store
            .import(archive_path)
            .map_err(RuntimeError::from)
    }

    pub fn list_agents(&self, agent_ref: impl Into<AgentRef>) -> Result<Vec<String>, RuntimeError> {
        let resolved = resolve_agent(&self.inner.store, &agent_ref.into())?;
        Ok(vec![resolved.agent.id])
    }

    pub fn list_models(&self, agent_ref: impl Into<AgentRef>) -> Result<Vec<String>, RuntimeError> {
        let resolved = resolve_agent(&self.inner.store, &agent_ref.into())?;
        Ok(vec![resolved.agent.model.name])
    }

    pub fn list_skills(
        &self,
        agent_ref: impl Into<AgentRef>,
    ) -> Result<Vec<SkillSummary>, RuntimeError> {
        let resolved = resolve_agent(&self.inner.store, &agent_ref.into())?;
        let store = BundleSkillStore::load(&resolved.install_path)?;
        Ok(store
            .list()
            .into_iter()
            .map(|skill| SkillSummary {
                name: skill.name,
                description: skill.description,
                path: skill.path,
            })
            .collect())
    }

    pub fn create_session(
        &self,
        spec: impl Into<SessionSpec>,
    ) -> Result<SessionSummary, RuntimeError> {
        let spec = spec.into();
        let resolved = resolve_agent(&self.inner.store, &spec.agent_ref)?;
        let model = spec.model.unwrap_or_else(|| resolved.default_model.clone());
        let record = self.inner.sessions.create(
            spec.agent_ref.to_string(),
            resolved.agent.id,
            model.provider,
            model.name,
        )?;
        Ok(summary_from_record(&record))
    }

    pub fn list_sessions(&self, filter: Option<&SessionFilter>) -> Vec<SessionSummary> {
        self.inner
            .sessions
            .list()
            .into_iter()
            .filter(
                |record| match filter.and_then(|value| value.agent_ref.as_ref()) {
                    Some(agent_ref) => record.agent_ref == agent_ref.as_str(),
                    None => true,
                },
            )
            .map(|record| summary_from_record(&record))
            .collect()
    }

    pub fn get_session(&self, session_id: Uuid) -> Result<Session, RuntimeError> {
        let record = self.inner.sessions.get(session_id)?;
        Ok(session_from_record(record))
    }

    pub fn delete_session(&self, session_id: Uuid) -> Result<(), RuntimeError> {
        self.inner.sessions.delete(session_id)
    }

    pub fn resolve_approval(
        &self,
        request_id: Uuid,
        decision: odyssey_rs_protocol::ApprovalDecision,
    ) -> Result<bool, RuntimeError> {
        let Some(session_id) = self.inner.approvals.session_id_for_request(request_id) else {
            return Ok(false);
        };
        let sender = self.session_sender(session_id)?;
        Ok(self.inner.approvals.resolve(request_id, decision, sender))
    }

    pub fn subscribe_session(
        &self,
        session_id: Uuid,
    ) -> Result<broadcast::Receiver<EventMsg>, RuntimeError> {
        self.inner.sessions.subscribe(session_id)
    }

    pub fn execution_status(&self, turn_id: Uuid) -> Option<ExecutionStatus> {
        self.scheduler.status(turn_id)
    }

    // Wait for run to complete
    pub async fn run(&self, request: ExecutionRequest) -> Result<RunOutput, RuntimeError> {
        let request_id = request.request_id;
        let (_, completion) = self.scheduler.submit(request).await?;
        info!("Submitted Execution Request : {}", request_id);
        completion
            .await
            .map_err(|_| RuntimeError::Executor("execution completion dropped".to_string()))?
    }

    //Submit the run and return
    pub async fn submit(&self, request: ExecutionRequest) -> Result<ExecutionHandle, RuntimeError> {
        let request_id = request.request_id;
        let (handle, completion) = self.scheduler.submit(request).await?;
        info!("Submitted Execution Request : {}", request_id);
        drop(completion);
        Ok(handle)
    }

    fn session_sender(
        &self,
        session_id: Uuid,
    ) -> Result<broadcast::Sender<EventMsg>, RuntimeError> {
        self.inner.sessions.sender(session_id)
    }
}

fn summary_from_record(record: &SessionRecord) -> SessionSummary {
    SessionSummary {
        id: record.id,
        agent_id: record.agent_id.clone(),
        message_count: record
            .turns
            .iter()
            .map(|turn| {
                if turn.chat_history.is_empty() {
                    2
                } else {
                    turn.chat_history.len()
                }
            })
            .sum(),
        created_at: record.created_at,
    }
}

fn session_from_record(record: SessionRecord) -> Session {
    let mut messages = Vec::new();
    for turn in record.turns {
        if turn.chat_history.is_empty() {
            messages.push(Message {
                role: Role::User,
                content: turn.prompt,
            });
            messages.push(Message {
                role: Role::Assistant,
                content: turn.response,
            });
            continue;
        }

        for message in turn.chat_history {
            let role = match message.role.as_str() {
                "assistant" => Role::Assistant,
                "system" => Role::System,
                _ => Role::User,
            };
            let content = if !message.content.is_empty() {
                message.content
            } else if matches!(message.kind, TurnChatMessageKind::ToolUse) {
                format!(
                    "tool_use: {}",
                    message
                        .tool_calls
                        .iter()
                        .map(|call| call.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            } else if matches!(message.kind, TurnChatMessageKind::ToolResult) {
                message
                    .tool_calls
                    .iter()
                    .map(|call| format!("tool_result {}: {}", call.name, call.arguments))
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                String::new()
            };
            messages.push(Message { role, content });
        }
    }
    Session {
        id: record.id,
        agent_id: record.agent_id,
        agent_ref: record.agent_ref,
        model_id: record.model_id,
        created_at: record.created_at,
        messages,
    }
}

#[cfg(test)]
mod tests {
    use super::{build_sandbox_runtime, session_from_record, summary_from_record};
    use crate::RuntimeConfig;
    use crate::session::{SessionRecord, TurnChatMessageKind, TurnChatMessageRecord, TurnRecord};
    use autoagents_llm::chat::ChatRole;
    use autoagents_llm::{FunctionCall, ToolCall};
    use chrono::Utc;
    use odyssey_rs_protocol::Task;
    use odyssey_rs_protocol::{Role, SandboxMode};
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;
    use uuid::Uuid;

    #[test]
    fn summary_and_session_conversion_preserve_message_semantics() {
        let session_id = Uuid::new_v4();
        let tool_call = ToolCall {
            id: "call-1".to_string(),
            call_type: "function".to_string(),
            function: FunctionCall {
                name: "Read".to_string(),
                arguments: "{\"path\":\"notes.txt\"}".to_string(),
            },
        };
        let record = SessionRecord {
            id: session_id,
            agent_ref: "local/demo@0.1.0".to_string(),
            agent_id: "demo".to_string(),
            model_provider: "openai".to_string(),
            model_id: "gpt-4.1-mini".to_string(),
            created_at: Utc::now(),
            turns: vec![
                TurnRecord {
                    turn_id: Uuid::new_v4(),
                    prompt: "hello".to_string(),
                    response: "world".to_string(),
                    chat_history: Vec::new(),
                    created_at: Utc::now(),
                },
                TurnRecord::from_history(
                    Uuid::new_v4(),
                    &Task::new(""),
                    "",
                    vec![
                        TurnChatMessageRecord::from_text(ChatRole::User, "check file"),
                        TurnChatMessageRecord::from_tool_calls(
                            ChatRole::Assistant,
                            TurnChatMessageKind::ToolUse,
                            vec![tool_call.clone()],
                        ),
                        TurnChatMessageRecord::from_tool_calls(
                            ChatRole::Tool,
                            TurnChatMessageKind::ToolResult,
                            vec![tool_call],
                        ),
                    ],
                    Utc::now(),
                ),
            ],
        };

        let summary = summary_from_record(&record);
        assert_eq!(summary.id, session_id);
        assert_eq!(summary.agent_id, "demo");
        assert_eq!(summary.message_count, 5);

        let session = session_from_record(record);
        assert_eq!(session.id, session_id);
        assert_eq!(session.agent_ref, "local/demo@0.1.0");
        assert_eq!(session.messages[0].role, Role::User);
        assert_eq!(session.messages[0].content, "hello");
        assert_eq!(session.messages[1].role, Role::Assistant);
        assert_eq!(session.messages[1].content, "world");
        assert_eq!(session.messages[2].content, "check file");
        assert_eq!(session.messages[3].content, "tool_use: Read");
        assert_eq!(
            session.messages[4].content,
            "tool_result Read: {\"path\":\"notes.txt\"}"
        );
    }

    #[test]
    fn build_sandbox_runtime_uses_host_backend_for_danger_mode() {
        let temp = tempdir().expect("tempdir");
        let config = RuntimeConfig {
            cache_root: temp.path().join("cache"),
            session_root: temp.path().join("sessions"),
            sandbox_root: temp.path().join("sandbox"),
            bind_addr: "127.0.0.1:0".to_string(),
            sandbox_mode_override: None,
            hub_url: "http://127.0.0.1:8473".to_string(),
            worker_count: 2,
            queue_capacity: 32,
        };

        let runtime =
            build_sandbox_runtime(&config, SandboxMode::DangerFullAccess).expect("runtime");

        assert_eq!(runtime.provider_name(), "host");
        assert_eq!(runtime.storage_root(), config.sandbox_root.as_path());
    }
}
