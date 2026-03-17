use super::bundle_loader::{LoadedBundle, load_bundle};
use super::prompt::build_system_prompt;
use super::templates::initialize_bundle;
use super::tool_event::{RuntimeApprovalHandler, RuntimeToolEventSink};
use crate::agent::{ExecutorRun, emit, run_executor};
use crate::memory::build_memory;
use crate::sandbox::{build_permission_rules, prepare_cell};
use crate::session::{
    ApprovalStore, SessionRecord, SessionStore, TurnChatMessageKind, TurnChatMessageRecord,
    TurnRecord,
};
use crate::skill::BundleSkillStore;
use crate::tool::select_tools;
use crate::{RuntimeConfig, RuntimeError};
use autoagents_llm::chat::ChatRole;
use autoagents_llm::{FunctionCall, ToolCall};
use chrono::Utc;
use odyssey_rs_bundle::{
    BundleBuilder, BundleInstallSummary, BundleMetadata, BundleProject, BundleStore,
};
use odyssey_rs_protocol::SandboxMode;
use odyssey_rs_protocol::{
    EventMsg, EventPayload, Message, ModelSpec, Role, Session, SessionSummary, SkillSummary,
};
use odyssey_rs_sandbox::{SandboxRuntime, default_provider_name};
use odyssey_rs_tools::{
    SkillProvider, ToolContext, ToolRegistry, builtin_registry, tools_to_adaptors,
};
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

#[derive(Clone)]
pub struct RuntimeEngine {
    pub config: RuntimeConfig,
    store: BundleStore,
    sessions: SessionStore,
    sandbox: Arc<SandboxRuntime>,
    host_sandbox: Arc<SandboxRuntime>,
    tools: ToolRegistry,
    approvals: ApprovalStore,
}

impl RuntimeEngine {
    pub fn new(config: RuntimeConfig) -> Result<Self, RuntimeError> {
        let store = BundleStore::new(config.cache_root.clone());
        let sessions = SessionStore::new(config.session_root.clone())?;
        let sandbox = Arc::new(build_sandbox_runtime(&config, SandboxMode::WorkspaceWrite)?);
        let host_sandbox = Arc::new(build_sandbox_runtime(
            &config,
            SandboxMode::DangerFullAccess,
        )?);
        Ok(Self {
            config,
            store,
            sessions,
            sandbox,
            host_sandbox,
            tools: builtin_registry(),
            approvals: ApprovalStore::new(),
        })
    }

    pub fn init(&self, root: impl AsRef<Path>) -> Result<(), RuntimeError> {
        initialize_bundle(root.as_ref())
    }

    pub fn build_and_install(
        &self,
        project_root: impl AsRef<Path>,
    ) -> Result<odyssey_rs_bundle::BundleInstall, RuntimeError> {
        self.store
            .build_and_install(project_root)
            .map_err(RuntimeError::from)
    }

    pub fn build_to(
        &self,
        project_root: impl AsRef<Path>,
        output_root: impl AsRef<Path>,
    ) -> Result<odyssey_rs_bundle::BundleArtifact, RuntimeError> {
        let project = BundleProject::load(project_root.as_ref().to_path_buf())?;
        BundleBuilder::new(project)
            .build(output_root)
            .map_err(RuntimeError::from)
    }

    pub fn inspect_bundle(&self, reference: &str) -> Result<BundleMetadata, RuntimeError> {
        Ok(self.store.resolve(reference)?.metadata)
    }

    pub fn export_bundle(
        &self,
        reference: &str,
        output: impl AsRef<Path>,
    ) -> Result<std::path::PathBuf, RuntimeError> {
        self.store
            .export(reference, output)
            .map_err(RuntimeError::from)
    }

    pub fn import_bundle(
        &self,
        archive_path: impl AsRef<Path>,
    ) -> Result<odyssey_rs_bundle::BundleInstall, RuntimeError> {
        self.store.import(archive_path).map_err(RuntimeError::from)
    }

    pub fn list_bundles(&self) -> Result<Vec<BundleInstallSummary>, RuntimeError> {
        self.store.list_installed().map_err(RuntimeError::from)
    }

    pub fn list_agents(&self, bundle_ref: &str) -> Result<Vec<String>, RuntimeError> {
        let loaded = load_bundle(&self.store, bundle_ref)?;
        Ok(vec![loaded.agent.id])
    }

    pub fn list_models(&self, bundle_ref: &str) -> Result<Vec<String>, RuntimeError> {
        let loaded = load_bundle(&self.store, bundle_ref)?;
        Ok(vec![loaded.agent.model.name])
    }

    pub fn list_skills(&self, bundle_ref: &str) -> Result<Vec<SkillSummary>, RuntimeError> {
        let loaded = load_bundle(&self.store, bundle_ref)?;
        let store = BundleSkillStore::load(&loaded.install.path)?;
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

    pub fn list_sessions(&self) -> Vec<SessionSummary> {
        self.sessions
            .list()
            .into_iter()
            .map(|record| summary_from_record(&record))
            .collect()
    }

    pub fn create_session(&self, bundle_ref: &str) -> Result<SessionSummary, RuntimeError> {
        let loaded = load_bundle(&self.store, bundle_ref)?;
        let record = self.sessions.create(
            bundle_ref.to_string(),
            loaded.agent.id,
            loaded.agent.model.name,
        )?;
        Ok(summary_from_record(&record))
    }

    pub fn get_session(&self, session_id: Uuid) -> Result<Session, RuntimeError> {
        let record = self.sessions.get(session_id)?;
        Ok(session_from_record(record))
    }

    pub fn delete_session(&self, session_id: Uuid) -> Result<(), RuntimeError> {
        self.sessions.delete(session_id)
    }

    pub fn resolve_approval(
        &self,
        request_id: Uuid,
        decision: odyssey_rs_protocol::ApprovalDecision,
    ) -> Result<bool, RuntimeError> {
        let Some(session_id) = self.approvals.session_id_for_request(request_id) else {
            return Ok(false);
        };
        let sender = self.session_sender(session_id)?;
        Ok(self.approvals.resolve(request_id, decision, sender))
    }

    pub fn subscribe(
        &self,
        session_id: Uuid,
    ) -> Result<broadcast::Receiver<EventMsg>, RuntimeError> {
        self.sessions.subscribe(session_id)
    }

    pub async fn run(&self, session_id: Uuid, prompt: String) -> Result<RunOutput, RuntimeError> {
        let session = self.sessions.get(session_id)?;
        let loaded = load_bundle(&self.store, &session.bundle_ref)?;
        let turn_id = Uuid::new_v4();
        let sender = self.session_sender(session_id)?;
        let receiver = self.sessions.subscribe(session_id)?;
        let response = self
            .execute_loaded_bundle(
                loaded,
                session.clone(),
                turn_id,
                prompt.clone(),
                sender.clone(),
            )
            .await?;
        let chat_history = collect_turn_chat_history(turn_id, &prompt, &response, receiver);
        self.sessions.append_turn(
            session_id,
            TurnRecord::from_history(turn_id, prompt, response.clone(), chat_history, Utc::now()),
        )?;
        Ok(RunOutput {
            session_id,
            turn_id,
            response,
        })
    }

    pub async fn submit_run(&self, session_id: Uuid, prompt: String) -> Result<Uuid, RuntimeError> {
        let session = self.sessions.get(session_id)?;
        let loaded = load_bundle(&self.store, &session.bundle_ref)?;
        let turn_id = Uuid::new_v4();
        let engine = self.clone();
        tokio::spawn(async move {
            let sender = match engine.session_sender(session_id) {
                Ok(sender) => sender,
                Err(_) => return,
            };
            let receiver = match engine.sessions.subscribe(session_id) {
                Ok(receiver) => receiver,
                Err(_) => return,
            };
            let response = engine
                .execute_loaded_bundle(
                    loaded,
                    session.clone(),
                    turn_id,
                    prompt.clone(),
                    sender.clone(),
                )
                .await;
            match response {
                Ok(response) => {
                    let chat_history =
                        collect_turn_chat_history(turn_id, &prompt, &response, receiver);
                    let _ = engine.sessions.append_turn(
                        session_id,
                        TurnRecord::from_history(
                            turn_id,
                            prompt,
                            response.clone(),
                            chat_history,
                            Utc::now(),
                        ),
                    );
                }
                Err(err) => {
                    emit(
                        &sender,
                        session_id,
                        EventPayload::Error {
                            turn_id: Some(turn_id),
                            message: err.to_string(),
                        },
                    );
                }
            }
        });
        Ok(turn_id)
    }

    pub async fn publish(
        &self,
        source: &str,
        target: &str,
    ) -> Result<BundleMetadata, RuntimeError> {
        self.store
            .publish(source, target, &self.config.hub_url)
            .await
            .map_err(RuntimeError::from)
    }

    pub async fn pull(
        &self,
        reference: &str,
    ) -> Result<odyssey_rs_bundle::BundleInstall, RuntimeError> {
        self.store
            .pull(reference, &self.config.hub_url)
            .await
            .map_err(RuntimeError::from)
    }

    async fn execute_loaded_bundle(
        &self,
        loaded: LoadedBundle,
        session: SessionRecord,
        turn_id: Uuid,
        prompt: String,
        sender: broadcast::Sender<EventMsg>,
    ) -> Result<String, RuntimeError> {
        let session_id = session.id;
        let mode = effective_sandbox_mode(&loaded.manifest, self.config.sandbox_mode_override);
        let sandbox_runtime = if mode == SandboxMode::DangerFullAccess {
            &self.host_sandbox
        } else {
            &self.sandbox
        };
        let cell = prepare_cell(
            sandbox_runtime,
            session_id,
            &loaded.agent.id,
            &loaded.install.path,
            &loaded.manifest,
            self.config.sandbox_mode_override,
        )
        .await?;
        let permissions = build_permission_rules(&loaded.manifest);
        let event_sink = Arc::new(RuntimeToolEventSink {
            session_id,
            turn_id,
            sender: sender.clone(),
            working_dir: cell.work_dir.display().to_string(),
        });
        let approval_handler = Arc::new(RuntimeApprovalHandler {
            session_id,
            turn_id,
            sender: sender.clone(),
            approvals: self.approvals.clone(),
        });
        let skills = Arc::new(BundleSkillStore::load(&cell.root)?);
        let system_prompt = build_system_prompt(
            &loaded.agent.prompt,
            &skills,
            !loaded.manifest.skills.is_empty(),
        );
        let ctx = ToolContext {
            session_id,
            turn_id,
            bundle_root: cell.root.clone(),
            working_dir: cell.work_dir.clone(),
            sandbox: cell.sandbox,
            permission_rules: permissions,
            event_sink: Some(event_sink),
            approval_handler: Some(approval_handler),
            skills: Some(skills),
        };
        let selected = select_tools(&self.tools, &loaded.manifest, &loaded.agent);
        let adapted = tools_to_adaptors(selected, ctx.clone());
        let llm = self.build_llm(&loaded)?;
        let memory = build_memory(&loaded.manifest, &session.turns)?;
        run_executor(ExecutorRun {
            executor_id: loaded.manifest.executor.id.clone(),
            llm,
            system_prompt,
            prompt,
            memory,
            tools: adapted,
            session_id,
            turn_id,
            sender,
            working_dir: Some(ctx.working_dir.display().to_string()),
            model: ModelSpec {
                provider: loaded.agent.model.provider.clone(),
                name: loaded.agent.model.name.clone(),
            },
        })
        .await
    }

    fn build_llm(
        &self,
        loaded: &LoadedBundle,
    ) -> Result<Arc<dyn autoagents_llm::LLMProvider>, RuntimeError> {
        match loaded.agent.model.provider.as_str() {
            "openai" => {
                let key = std::env::var("OPENAI_API_KEY").map_err(|_| {
                    RuntimeError::Unsupported(
                        "OPENAI_API_KEY is required for provider openai".to_string(),
                    )
                })?;
                let llm: Arc<dyn autoagents_llm::LLMProvider> = autoagents_llm::builder::LLMBuilder::<autoagents_llm::backends::openai::OpenAI>::new()
                    .api_key(key)
                    .model(&loaded.agent.model.name)
                    .build()
                    .map_err(|err| RuntimeError::Executor(err.to_string()))?;
                Ok(llm)
            }
            other => Err(RuntimeError::Unsupported(format!(
                "unsupported model provider: {other}"
            ))),
        }
    }

    fn session_sender(
        &self,
        session_id: Uuid,
    ) -> Result<broadcast::Sender<EventMsg>, RuntimeError> {
        self.sessions.sender(session_id)
    }
}

fn build_sandbox_runtime(
    config: &RuntimeConfig,
    mode: SandboxMode,
) -> Result<SandboxRuntime, RuntimeError> {
    SandboxRuntime::from_provider_name(
        Some(default_provider_name(mode)),
        mode,
        config.sandbox_root.clone(),
    )
    .map_err(RuntimeError::from)
}

fn effective_sandbox_mode(
    manifest: &odyssey_rs_manifest::BundleManifest,
    override_mode: Option<SandboxMode>,
) -> SandboxMode {
    override_mode.unwrap_or(manifest.sandbox.mode)
}

fn collect_turn_chat_history(
    turn_id: Uuid,
    prompt: &str,
    response: &str,
    mut receiver: broadcast::Receiver<EventMsg>,
) -> Vec<TurnChatMessageRecord> {
    let mut collector = TurnHistoryCollector::new(turn_id, prompt);
    while let Ok(event) = receiver.try_recv() {
        collector.observe(event);
    }
    collector.finish(response)
}

struct TurnHistoryCollector {
    turn_id: Uuid,
    messages: Vec<TurnChatMessageRecord>,
    assistant_text: String,
    pending_calls: std::collections::HashMap<Uuid, ToolCall>,
}

impl TurnHistoryCollector {
    fn new(turn_id: Uuid, prompt: &str) -> Self {
        Self {
            turn_id,
            messages: vec![TurnChatMessageRecord::from_text(ChatRole::User, prompt)],
            assistant_text: String::new(),
            pending_calls: std::collections::HashMap::new(),
        }
    }

    fn observe(&mut self, event: EventMsg) {
        match event.payload {
            EventPayload::AgentMessageDelta { turn_id, delta } if turn_id == self.turn_id => {
                self.assistant_text.push_str(&delta);
            }
            EventPayload::ToolCallStarted {
                turn_id,
                tool_call_id,
                tool_name,
                arguments,
            } if turn_id == self.turn_id => {
                self.flush_assistant_text();
                let call = ToolCall {
                    id: tool_call_id.to_string(),
                    call_type: "function".to_string(),
                    function: FunctionCall {
                        name: tool_name,
                        arguments: arguments.to_string(),
                    },
                };
                self.pending_calls.insert(tool_call_id, call.clone());
                self.messages.push(TurnChatMessageRecord::from_tool_calls(
                    ChatRole::Assistant,
                    TurnChatMessageKind::ToolUse,
                    vec![call],
                ));
            }
            EventPayload::ToolCallFinished {
                turn_id,
                tool_call_id,
                result,
                ..
            } if turn_id == self.turn_id => {
                self.flush_assistant_text();
                let started = self.pending_calls.remove(&tool_call_id);
                let call = ToolCall {
                    id: tool_call_id.to_string(),
                    call_type: "function".to_string(),
                    function: FunctionCall {
                        name: started
                            .as_ref()
                            .map(|call| call.function.name.clone())
                            .unwrap_or_default(),
                        arguments: result.to_string(),
                    },
                };
                self.messages.push(TurnChatMessageRecord::from_tool_calls(
                    ChatRole::Tool,
                    TurnChatMessageKind::ToolResult,
                    vec![call],
                ));
            }
            EventPayload::TurnCompleted { turn_id, message } if turn_id == self.turn_id => {
                self.assistant_text = message;
            }
            _ => {}
        }
    }

    fn finish(mut self, response: &str) -> Vec<TurnChatMessageRecord> {
        if self.assistant_text.is_empty() && !response.is_empty() {
            self.assistant_text = response.to_string();
        }
        self.flush_assistant_text();
        self.messages
    }

    fn flush_assistant_text(&mut self) {
        if self.assistant_text.is_empty() {
            return;
        }
        self.messages.push(TurnChatMessageRecord::from_text(
            ChatRole::Assistant,
            std::mem::take(&mut self.assistant_text),
        ));
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
        bundle_ref: record.bundle_ref,
        model_id: record.model_id,
        created_at: record.created_at,
        messages,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_sandbox_runtime, collect_turn_chat_history, effective_sandbox_mode,
        session_from_record, summary_from_record,
    };
    use crate::RuntimeConfig;
    use crate::session::{SessionRecord, TurnChatMessageKind, TurnChatMessageRecord, TurnRecord};
    use autoagents_llm::chat::ChatRole;
    use autoagents_llm::{FunctionCall, ToolCall};
    use chrono::Utc;
    use odyssey_rs_manifest::{
        BundleExecutor, BundleManifest, BundleMemory, BundleSandbox, BundleServer, BundleTool,
    };
    use odyssey_rs_protocol::{EventMsg, EventPayload};
    use odyssey_rs_protocol::{Role, SandboxMode};
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use tempfile::tempdir;
    use tokio::sync::broadcast;
    use uuid::Uuid;

    #[test]
    fn collect_turn_chat_history_preserves_tool_use_and_result_ids() {
        let session_id = Uuid::new_v4();
        let turn_id = Uuid::new_v4();
        let tool_call_id = Uuid::new_v4();
        let (sender, receiver) = broadcast::channel(32);

        let _ = sender.send(EventMsg {
            id: Uuid::new_v4(),
            session_id,
            created_at: Utc::now(),
            payload: EventPayload::ToolCallStarted {
                turn_id,
                tool_call_id,
                tool_name: "Write".to_string(),
                arguments: json!({ "path": "helloworld.py" }),
            },
        });
        let _ = sender.send(EventMsg {
            id: Uuid::new_v4(),
            session_id,
            created_at: Utc::now(),
            payload: EventPayload::ToolCallFinished {
                turn_id,
                tool_call_id,
                result: json!({ "error": "permission denied" }),
                success: false,
            },
        });
        let _ = sender.send(EventMsg {
            id: Uuid::new_v4(),
            session_id,
            created_at: Utc::now(),
            payload: EventPayload::TurnCompleted {
                turn_id,
                message: "The write failed.".to_string(),
            },
        });

        let history =
            collect_turn_chat_history(turn_id, "create file", "The write failed.", receiver);

        assert_eq!(history.len(), 4);
        assert_eq!(history[0].content, "create file");
        assert_eq!(history[1].tool_calls[0].id, tool_call_id.to_string());
        assert_eq!(history[2].tool_calls[0].id, tool_call_id.to_string());
        assert_eq!(history[3].content, "The write failed.");
    }

    fn manifest(mode: SandboxMode) -> BundleManifest {
        BundleManifest {
            id: "demo".to_string(),
            version: "0.1.0".to_string(),
            agent_spec: "agent.yaml".to_string(),
            executor: BundleExecutor {
                kind: "prebuilt".to_string(),
                id: "react".to_string(),
                config: json!({}),
            },
            memory: BundleMemory::default(),
            resources: Vec::new(),
            skills: Vec::new(),
            tools: vec![BundleTool {
                name: "Read".to_string(),
                source: "builtin".to_string(),
            }],
            server: BundleServer::default(),
            sandbox: BundleSandbox {
                mode,
                ..BundleSandbox::default()
            },
        }
    }

    #[test]
    fn effective_sandbox_mode_prefers_override() {
        assert_eq!(
            effective_sandbox_mode(
                &manifest(SandboxMode::WorkspaceWrite),
                Some(SandboxMode::DangerFullAccess)
            ),
            SandboxMode::DangerFullAccess
        );
        assert_eq!(
            effective_sandbox_mode(&manifest(SandboxMode::WorkspaceWrite), None),
            SandboxMode::WorkspaceWrite
        );
    }

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
            bundle_ref: "local/demo@0.1.0".to_string(),
            agent_id: "demo".to_string(),
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
                    "",
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
        };

        let runtime =
            build_sandbox_runtime(&config, SandboxMode::DangerFullAccess).expect("runtime");

        assert_eq!(runtime.provider_name(), "host");
        assert_eq!(runtime.storage_root(), config.sandbox_root.as_path());
    }
}
