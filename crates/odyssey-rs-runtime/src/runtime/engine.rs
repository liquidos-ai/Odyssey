use super::scheduler::ExecutionScheduler;
use super::templates::initialize_bundle;
use super::tool_event::RuntimeToolEventSink;
use crate::resolver::bundle::resolve_bundle_from_ref;
use crate::sandbox::{build_permission_rules, build_sandbox_runtime};
use crate::session::{ApprovalStore, SessionRecord, SessionStore, TurnChatMessageKind};
use crate::skill::BundleSkillStore;
use crate::{RuntimeConfig, RuntimeError};
use log::{debug, info};
use odyssey_rs_bundle::{
    BundleArtifact, BundleBuilder, BundleInstall, BundleMetadata, BundleProject, BundleStore,
};
use odyssey_rs_protocol::SandboxMode;
use odyssey_rs_protocol::{
    BundleRef, EventMsg, ExecutionHandle, ExecutionRequest, ExecutionStatus, Message, Role,
    Session, SessionFilter, SessionSpec, SessionSummary, SkillSummary,
};
use odyssey_rs_sandbox::SandboxRuntime;
use odyssey_rs_tools::{SkillProvider, ToolContext, ToolRegistry, ToolSandbox, builtin_registry};
use parking_lot::Mutex;
use std::collections::{HashMap, hash_map::Entry};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::time::Instant;
use uuid::Uuid;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RunOutput {
    pub session_id: Uuid,
    pub turn_id: Uuid,
    pub response: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionCommandOutput {
    pub session_id: Uuid,
    pub turn_id: Uuid,
    pub status_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

#[derive(Default)]
struct SessionExecutionGuards {
    guards: Mutex<HashMap<Uuid, std::sync::Weak<tokio::sync::Mutex<()>>>>,
}

pub(crate) struct SessionExecutionGuard {
    _lock: Arc<tokio::sync::Mutex<()>>,
    _guard: tokio::sync::OwnedMutexGuard<()>,
}

impl SessionExecutionGuards {
    async fn acquire(&self, session_id: Uuid) -> SessionExecutionGuard {
        let lock = {
            let mut guards = self.guards.lock();
            match guards.entry(session_id) {
                Entry::Occupied(mut entry) => {
                    if let Some(lock) = entry.get().upgrade() {
                        lock
                    } else {
                        let lock = Arc::new(tokio::sync::Mutex::new(()));
                        entry.insert(Arc::downgrade(&lock));
                        lock
                    }
                }
                Entry::Vacant(entry) => {
                    let lock = Arc::new(tokio::sync::Mutex::new(()));
                    entry.insert(Arc::downgrade(&lock));
                    lock
                }
            }
        };

        let guard = lock.clone().lock_owned().await;
        SessionExecutionGuard {
            _lock: lock,
            _guard: guard,
        }
    }

    fn remove(&self, session_id: Uuid) {
        self.guards.lock().remove(&session_id);
    }
}

pub(crate) struct OdysseyRuntimeInner {
    pub(crate) config: RuntimeConfig,
    pub(crate) store: BundleStore,
    pub(crate) sessions: SessionStore,
    pub(crate) host_sandbox: Arc<SandboxRuntime>,
    pub(crate) restricted_sandbox: Arc<SandboxRuntime>,
    pub(crate) tools: ToolRegistry,
    pub(crate) approvals: ApprovalStore,
    execution_guards: SessionExecutionGuards,
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
        let restricted_sandbox =
            Arc::new(build_sandbox_runtime(&config, SandboxMode::WorkspaceWrite)?);
        let worker_count = config.worker_count;
        let queue_capacity = config.queue_capacity;
        let inner = Arc::new(OdysseyRuntimeInner {
            config,
            store,
            sessions,
            host_sandbox,
            restricted_sandbox,
            tools: builtin_registry(),
            approvals: ApprovalStore::default(),
            execution_guards: SessionExecutionGuards::default(),
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

    pub fn list_agents(
        &self,
        bundle_ref: impl Into<BundleRef>,
    ) -> Result<Vec<String>, RuntimeError> {
        let resolved = resolve_bundle_from_ref(&self.inner.store, &bundle_ref.into())?;
        Ok(vec![resolved.default_agent.id])
    }

    pub fn list_models(
        &self,
        bundle_ref: impl Into<BundleRef>,
    ) -> Result<Vec<String>, RuntimeError> {
        let resolved = resolve_bundle_from_ref(&self.inner.store, &bundle_ref.into())?;
        Ok(vec![resolved.default_agent.model.name])
    }

    pub fn list_skills(
        &self,
        bundle_ref: impl Into<BundleRef>,
    ) -> Result<Vec<SkillSummary>, RuntimeError> {
        let resolved = resolve_bundle_from_ref(&self.inner.store, &bundle_ref.into())?;
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
        let resolved = resolve_bundle_from_ref(&self.inner.store, &spec.bundle_ref)?;
        let model = spec.model.unwrap_or_else(|| resolved.model.clone());
        let record = self.inner.sessions.create(
            spec.bundle_ref.to_string(),
            resolved.default_agent.id,
            model.provider,
            model.name,
            model.config,
        )?;
        Ok(summary_from_record(&record))
    }

    pub fn list_sessions(&self, filter: Option<&SessionFilter>) -> Vec<SessionSummary> {
        self.inner
            .sessions
            .list()
            .into_iter()
            .filter(
                |record| match filter.and_then(|value| value.bundle_ref.as_ref()) {
                    Some(bundle_ref) => record.bundle_ref == bundle_ref.as_str(),
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

    pub async fn delete_session(&self, session_id: Uuid) -> Result<(), RuntimeError> {
        let _session_guard = self.inner.lock_session_execution(session_id).await;
        self.inner.sessions.delete(session_id)?;
        self.inner.approvals.clear_session(session_id);
        self.inner.host_sandbox.shutdown_session(session_id)?;
        if !Arc::ptr_eq(&self.inner.host_sandbox, &self.inner.restricted_sandbox) {
            self.inner.restricted_sandbox.shutdown_session(session_id)?;
        }
        self.inner.remove_session_execution_guard(session_id);
        Ok(())
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

    /// Execute a direct process command inside the active session sandbox.
    pub async fn run_session_command(
        &self,
        session_id: Uuid,
        command_line: impl AsRef<str>,
    ) -> Result<SessionCommandOutput, RuntimeError> {
        let command_line = command_line.as_ref();
        if command_line.trim().is_empty() {
            return Err(RuntimeError::Executor(
                "command cannot be empty".to_string(),
            ));
        }

        let _session_guard = self.inner.lock_session_execution(session_id).await;
        let session = self.inner.sessions.get(session_id)?;
        let resolved =
            resolve_bundle_from_ref(&self.inner.store, &BundleRef::from(session.bundle_ref))?;
        let mode = super::executor::effective_sandbox_mode(
            &resolved.manifest,
            self.inner.config.sandbox_mode_override,
        );
        debug!("Effective Sandbox mode: {:?}", mode);
        let cell = super::executor::prepare_resolved_bundle_command_cell(
            &mode,
            &self.inner,
            &resolved,
            session_id,
        )
        .await?;
        let sender = self.inner.sessions.sender(session_id)?;
        let turn_id = Uuid::new_v4();
        let event_sink = Arc::new(RuntimeToolEventSink {
            session_id,
            turn_id,
            sender: sender.clone(),
            working_dir: cell.work_dir.display().to_string(),
        });
        let approval_handler = Arc::new(super::tool_event::RuntimeApprovalHandler {
            session_id,
            turn_id,
            sender,
            approvals: self.inner.approvals.clone(),
        });
        let ctx = ToolContext {
            session_id,
            turn_id,
            bundle_root: cell.root.clone(),
            working_dir: cell.work_dir.clone(),
            workspace_mounts: cell.workspace_mounts.clone(),
            sandbox: ToolSandbox {
                provider: cell.sandbox.provider,
                handle: cell.sandbox.handle,
                lease: cell.sandbox.lease,
            },
            permission_rules: build_permission_rules(&resolved.default_agent)?,
            event_sink: Some(event_sink),
            approval_handler: Some(approval_handler),
            skills: None,
        };
        info!("Built tool context");

        let spec = build_session_command_spec(&ctx, command_line)?;

        info!("Running command in sandbox session with spec: {:?}", spec);
        let start_time = Instant::now();
        let output = ctx.run_command("SessionCommand", spec).await?;
        info!(
            "Command execution completed in : {}",
            start_time.elapsed().as_millis()
        );

        Ok(SessionCommandOutput {
            session_id,
            turn_id,
            status_code: output.status_code,
            stdout: output.stdout,
            stderr: output.stderr,
            stdout_truncated: output.stdout_truncated,
            stderr_truncated: output.stderr_truncated,
        })
    }

    fn session_sender(
        &self,
        session_id: Uuid,
    ) -> Result<broadcast::Sender<EventMsg>, RuntimeError> {
        self.inner.sessions.sender(session_id)
    }
}

impl OdysseyRuntimeInner {
    pub(crate) async fn lock_session_execution(&self, session_id: Uuid) -> SessionExecutionGuard {
        self.execution_guards.acquire(session_id).await
    }

    fn remove_session_execution_guard(&self, session_id: Uuid) {
        self.execution_guards.remove(session_id);
    }
}

fn build_session_command_spec(
    ctx: &ToolContext,
    command_line: &str,
) -> Result<odyssey_rs_sandbox::CommandSpec, RuntimeError> {
    if command_line.trim().is_empty() {
        return Err(RuntimeError::Executor(
            "command cannot be empty".to_string(),
        ));
    }

    let tokens = shell_words::split(command_line)
        .map_err(|err| RuntimeError::Executor(format!("invalid command line: {err}")))?;
    let (program, args) = tokens
        .split_first()
        .ok_or_else(|| RuntimeError::Executor("command cannot be empty".to_string()))?;

    // Direct session commands are operator-invoked sandbox processes, not the
    // bundle's `Bash` tool. Execute the resolved program directly so the
    // sandbox policy applies to the actual binary being launched.
    let mut spec = odyssey_rs_sandbox::CommandSpec::new(std::path::PathBuf::from(program));
    spec.args = args.to_vec();
    spec.cwd = Some(ctx.working_dir.clone());
    Ok(spec)
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
                String::default()
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
        SessionExecutionGuards, build_sandbox_runtime, build_session_command_spec,
        session_from_record, summary_from_record,
    };
    use crate::OdysseyRuntime;
    use crate::RuntimeConfig;
    use crate::session::{SessionRecord, TurnChatMessageKind, TurnChatMessageRecord, TurnRecord};
    use autoagents_llm::chat::ChatRole;
    use autoagents_llm::{FunctionCall, ToolCall};
    use chrono::Utc;
    use odyssey_rs_protocol::Task;
    use odyssey_rs_protocol::{EventPayload, Role, SandboxMode};
    use odyssey_rs_sandbox::{HostExecProvider, SandboxHandle};
    use odyssey_rs_tools::{ToolContext, ToolSandbox};
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::path::Path;
    use std::sync::Arc;
    use tempfile::tempdir;
    use tokio::sync::broadcast;
    use tokio::time::{Duration, timeout};
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
            bundle_ref: "local/demo@0.1.0".to_string(),
            agent_id: "demo".to_string(),
            model_provider: "openai".to_string(),
            model_id: "gpt-4.1-mini".to_string(),
            model_config: None,
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
        assert_eq!(session.bundle_ref, "local/demo@0.1.0");
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

    fn runtime_config(root: &Path) -> RuntimeConfig {
        RuntimeConfig {
            cache_root: root.join("cache"),
            session_root: root.join("sessions"),
            sandbox_root: root.join("sandbox"),
            bind_addr: "127.0.0.1:0".to_string(),
            sandbox_mode_override: Some(SandboxMode::DangerFullAccess),
            hub_url: "http://127.0.0.1:8473".to_string(),
            worker_count: 2,
            queue_capacity: 32,
        }
    }

    fn write_bundle_project(root: &Path, bundle_id: &str, agent_id: &str) {
        fs::create_dir_all(root.join("skills").join("repo-hygiene")).expect("create skill dir");
        fs::create_dir_all(root.join("resources").join("data")).expect("create data dir");
        fs::write(
            root.join("odyssey.bundle.json5"),
            format!(
                r#"{{
                    id: "{bundle_id}",
                    version: "0.1.0",
                    manifest_version: "odyssey.bundle/v1",
                    readme: "README.md",
                    agent_spec: "agent.yaml",
                    executor: {{ type: "prebuilt", id: "react" }},
                    memory: {{ type: "prebuilt", id: "sliding_window" }},
                    skills: [{{ name: "repo-hygiene", path: "skills/repo-hygiene" }}],
                    tools: [{{ name: "Read", source: "builtin" }}],
                    sandbox: {{
                        permissions: {{
                            filesystem: {{ exec: [], mounts: {{ read: [], write: [] }} }},
                            network: ["*"]
                        }},
                        system_tools: ["sh"],
                        resources: {{}}
                    }}
                }}"#
            ),
        )
        .expect("write manifest");
        fs::write(
            root.join("agent.yaml"),
            format!(
                r#"id: {agent_id}
description: test bundle
prompt: keep responses concise
model:
  provider: openai
  name: gpt-4.1-mini
tools:
  allow: ["Read", "Skill"]
"#
            ),
        )
        .expect("write agent");
        fs::write(root.join("README.md"), format!("# {bundle_id}\n")).expect("write readme");
        fs::write(
            root.join("skills").join("repo-hygiene").join("SKILL.md"),
            "Keep commits focused.\n",
        )
        .expect("write skill");
        fs::write(
            root.join("resources").join("data").join("notes.txt"),
            "hello world\n",
        )
        .expect("write resource");
    }

    fn write_bundle_project_with_read_mount(
        root: &Path,
        bundle_id: &str,
        agent_id: &str,
        read_mount: &Path,
    ) {
        let read_mounts =
            serde_json::to_string(&vec![read_mount.display().to_string()]).expect("read mounts");
        fs::create_dir_all(root.join("skills").join("repo-hygiene")).expect("create skill dir");
        fs::create_dir_all(root.join("resources").join("data")).expect("create data dir");
        fs::write(
            root.join("odyssey.bundle.json5"),
            format!(
                r#"{{
                    id: "{bundle_id}",
                    version: "0.1.0",
                    manifest_version: "odyssey.bundle/v1",
                    readme: "README.md",
                    agent_spec: "agent.yaml",
                    executor: {{ type: "prebuilt", id: "react" }},
                    memory: {{ type: "prebuilt", id: "sliding_window" }},
                    skills: [{{ name: "repo-hygiene", path: "skills/repo-hygiene" }}],
                    tools: [{{ name: "Read", source: "builtin" }}],
                    sandbox: {{
                        permissions: {{
                            filesystem: {{ exec: [], mounts: {{ read: {read_mounts}, write: [] }} }},
                            network: ["*"]
                        }},
                        system_tools: ["sh"],
                        resources: {{}}
                    }}
                }}"#
            ),
        )
        .expect("write manifest");
        fs::write(
            root.join("agent.yaml"),
            format!(
                r#"id: {agent_id}
description: test bundle
prompt: keep responses concise
model:
  provider: openai
  name: gpt-4.1-mini
tools:
  allow: ["Read", "Skill"]
"#
            ),
        )
        .expect("write agent");
        fs::write(root.join("README.md"), format!("# {bundle_id}\n")).expect("write readme");
        fs::write(
            root.join("skills").join("repo-hygiene").join("SKILL.md"),
            "Keep commits focused.\n",
        )
        .expect("write skill");
        fs::write(
            root.join("resources").join("data").join("notes.txt"),
            "hello world\n",
        )
        .expect("write resource");
    }

    #[test]
    fn build_session_command_spec_rejects_empty_commands() {
        let temp = tempdir().expect("tempdir");
        let (sender, _) = broadcast::channel(8);
        let ctx = ToolContext {
            session_id: Uuid::new_v4(),
            turn_id: Uuid::new_v4(),
            bundle_root: temp.path().to_path_buf(),
            working_dir: temp.path().to_path_buf(),
            workspace_mounts: Vec::new(),
            sandbox: ToolSandbox {
                provider: Arc::new(HostExecProvider::default()),
                handle: SandboxHandle { id: Uuid::new_v4() },
                lease: None,
            },
            permission_rules: Vec::new(),
            event_sink: Some(Arc::new(super::super::tool_event::RuntimeToolEventSink {
                session_id: Uuid::new_v4(),
                turn_id: Uuid::new_v4(),
                sender,
                working_dir: temp.path().display().to_string(),
            })),
            approval_handler: None,
            skills: None,
        };

        let error = build_session_command_spec(&ctx, "   ").expect_err("empty command");
        assert_eq!(error.to_string(), "executor error: command cannot be empty");
    }

    #[test]
    fn build_session_command_spec_executes_direct_program() {
        let temp = tempdir().expect("tempdir");
        let (sender, _) = broadcast::channel(8);
        let ctx = ToolContext {
            session_id: Uuid::new_v4(),
            turn_id: Uuid::new_v4(),
            bundle_root: temp.path().to_path_buf(),
            working_dir: temp.path().to_path_buf(),
            workspace_mounts: Vec::new(),
            sandbox: ToolSandbox {
                provider: Arc::new(HostExecProvider::default()),
                handle: SandboxHandle { id: Uuid::new_v4() },
                lease: None,
            },
            permission_rules: Vec::new(),
            event_sink: Some(Arc::new(super::super::tool_event::RuntimeToolEventSink {
                session_id: Uuid::new_v4(),
                turn_id: Uuid::new_v4(),
                sender,
                working_dir: temp.path().display().to_string(),
            })),
            approval_handler: None,
            skills: None,
        };

        let spec = build_session_command_spec(&ctx, "ls -la").expect("build spec");

        assert_eq!(spec.command, std::path::PathBuf::from("ls"));
        assert_eq!(spec.args, vec!["-la".to_string()]);
        assert_eq!(spec.cwd, Some(temp.path().to_path_buf()));
    }

    #[tokio::test]
    async fn session_execution_guards_serialize_same_session_work() {
        let guards = Arc::new(SessionExecutionGuards::default());
        let session_id = Uuid::new_v4();
        let first = guards.acquire(session_id).await;
        let guards_clone = guards.clone();
        let (signal_tx, mut signal_rx) = tokio::sync::oneshot::channel();

        let waiter = tokio::spawn(async move {
            let _second = guards_clone.acquire(session_id).await;
            let _ = signal_tx.send(());
        });

        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(25)).await;
        assert!(matches!(
            signal_rx.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));

        drop(first);
        timeout(Duration::from_secs(1), async {
            signal_rx.await.expect("signal");
        })
        .await
        .expect("same-session waiter should resume");
        waiter.await.expect("waiter joined");
    }

    #[tokio::test]
    async fn session_execution_guards_allow_different_sessions_in_parallel() {
        let guards = Arc::new(SessionExecutionGuards::default());
        let _first = guards.acquire(Uuid::new_v4()).await;
        let guards_clone = guards.clone();

        timeout(Duration::from_secs(1), async move {
            let _second = guards_clone.acquire(Uuid::new_v4()).await;
        })
        .await
        .expect("different sessions should not block");
    }

    #[tokio::test]
    async fn run_session_command_executes_and_streams_exec_events() {
        let temp = tempdir().expect("tempdir");
        let runtime = Arc::new(OdysseyRuntime::new(runtime_config(temp.path())).expect("runtime"));
        let project = temp.path().join("alpha-project");
        fs::create_dir_all(&project).expect("create project");
        write_bundle_project(&project, "alpha", "alpha-agent");
        runtime.build_and_install(&project).expect("install bundle");

        let session_id = runtime
            .create_session("local/alpha@0.1.0")
            .expect("create session")
            .id;
        let mut receiver = runtime.subscribe_session(session_id).expect("subscribe");
        let output = runtime
            .run_session_command(session_id, "printf runtime-direct")
            .await
            .expect("run command");

        assert_eq!(output.session_id, session_id);
        assert_eq!(output.stdout, "runtime-direct");
        assert_eq!(output.stderr, "");
        assert_eq!(output.status_code, Some(0));

        assert!(matches!(
            receiver.recv().await.expect("begin").payload,
            EventPayload::ExecCommandBegin { .. }
        ));
        assert!(matches!(
            receiver.recv().await.expect("stdout").payload,
            EventPayload::ExecCommandOutputDelta { .. }
        ));
        assert!(matches!(
            receiver.recv().await.expect("end").payload,
            EventPayload::ExecCommandEnd { .. }
        ));
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn run_session_command_exposes_host_mount_aliases_in_danger_mode() {
        let temp = tempdir().expect("tempdir");
        let runtime = Arc::new(OdysseyRuntime::new(runtime_config(temp.path())).expect("runtime"));
        let host_read = temp.path().join("host-read");
        fs::create_dir_all(&host_read).expect("host read dir");
        fs::write(host_read.join("mounted.txt"), "mounted via alias").expect("mounted file");
        let project = temp.path().join("alpha-project");
        fs::create_dir_all(&project).expect("create project");
        write_bundle_project_with_read_mount(&project, "alpha", "alpha-agent", &host_read);
        runtime.build_and_install(&project).expect("install bundle");

        let session_id = runtime
            .create_session("local/alpha@0.1.0")
            .expect("create session")
            .id;
        let visible_mount = std::path::PathBuf::from("mount")
            .join("read")
            .join("abs")
            .join(
                host_read
                    .strip_prefix(std::path::Path::new("/"))
                    .expect("absolute host path"),
            );
        let output = runtime
            .run_session_command(
                session_id,
                format!("cat {}/mounted.txt", visible_mount.display()),
            )
            .await
            .expect("run command");

        assert_eq!(output.status_code, Some(0));
        assert_eq!(output.stdout, "mounted via alias");
        assert_eq!(output.stderr, "");
    }

    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn run_session_command_uses_operator_exec_policy_in_restricted_sandbox() {
        if which::which("bwrap").is_err() {
            return;
        }

        let temp = tempdir().expect("tempdir");
        let mut config = runtime_config(temp.path());
        config.sandbox_mode_override = None;
        let runtime = Arc::new(OdysseyRuntime::new(config).expect("runtime"));
        let project = temp.path().join("alpha-project");
        fs::create_dir_all(&project).expect("create project");
        write_bundle_project(&project, "alpha", "alpha-agent");
        runtime.build_and_install(&project).expect("install bundle");

        let session_id = runtime
            .create_session("local/alpha@0.1.0")
            .expect("create session")
            .id;
        let output = runtime
            .run_session_command(session_id, "ls")
            .await
            .expect("run command");

        assert_eq!(output.status_code, Some(0));
        assert!(output.stdout.contains("agent.yaml"));
    }

    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn workspace_write_session_commands_persist_staged_changes_within_session() {
        if which::which("bwrap").is_err() {
            return;
        }

        let temp = tempdir().expect("tempdir");
        let mut config = runtime_config(temp.path());
        config.sandbox_mode_override = None;
        let runtime = Arc::new(OdysseyRuntime::new(config).expect("runtime"));
        let project = temp.path().join("alpha-project");
        fs::create_dir_all(&project).expect("create project");
        write_bundle_project(&project, "alpha", "alpha-agent");
        runtime.build_and_install(&project).expect("install bundle");

        let session_id = runtime
            .create_session("local/alpha@0.1.0")
            .expect("create session")
            .id;

        let touch = runtime
            .run_session_command(session_id, "touch hellp.py")
            .await
            .expect("touch");
        assert_eq!(touch.status_code, Some(0));

        let list = runtime
            .run_session_command(session_id, "ls")
            .await
            .expect("list");
        assert_eq!(list.status_code, Some(0));
        assert!(list.stdout.contains("hellp.py"));
    }
}
