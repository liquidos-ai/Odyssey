//! Orchestrator Core

mod agent_factory;
mod memory;
pub mod prompt;
mod registry;
mod runtime;
mod sessions;
mod tool_context;
pub use registry::LLMEntry;

use crate::AgentBuilder;
use crate::agent::AgentInstance;
use crate::error::OdysseyCoreError;
use crate::orchestrator::registry::LLMRegistry;
use crate::permissions::{ApprovalHandler, ApprovalRequest, PermissionEngine, PermissionHook};
use crate::skills::SkillStore;
use crate::state::{JsonlStateStore, StateStore};
use crate::tools::ToolRouter;
use crate::types::{AgentInfo, OdysseyAgentRuntime, Session, SessionId, SessionSummary};
use autoagents_core::agent::{AgentDeriveT, AgentExecutor};
use autoagents_llm::LLMProvider;
use directories::BaseDirs;
use log::{debug, info, warn};
use odyssey_rs_config::{OdysseyConfig, SessionsConfig};
use odyssey_rs_protocol::{EventMsg, EventSink, SkillProvider, SkillSummary, TurnId};
use odyssey_rs_sandbox::{
    BubblewrapProvider, LocalSandboxProvider, SandboxProvider, default_provider_name,
};
use odyssey_rs_tools::{QuestionHandler, ToolRegistry};
use parking_lot::RwLock;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio_stream::wrappers::BroadcastStream;
use uuid::Uuid;

use agent_factory::AutoAgentsExecutor;
use registry::{AgentEntry, AgentRegistry};
use runtime::{ToolResultMode, TurnExecutor};
use sessions::SessionStore;
use tool_context::ToolContextFactory;

pub const DEFAULT_AGENT_ID: &str = "odyssey-orchestrator";
pub const DEFAULT_LLM_ID: &str = "odyssey-default-llm";
const RUN_STREAM_BUFFER: usize = 512;

/// Result payload for a single run invocation.
pub struct RunResult {
    /// Session id that produced the response.
    pub session_id: SessionId,
    /// Assistant response content.
    pub response: String,
}

/// Streaming handle for a single run invocation.
pub struct RunStream {
    /// Session id that produced the response.
    pub session_id: SessionId,
    /// Turn id associated with the streaming response.
    pub turn_id: TurnId,
    /// Stream of events emitted during the run.
    pub events: BroadcastStream<EventMsg>,
    handle: JoinHandle<Result<RunResult, OdysseyCoreError>>,
}

impl RunStream {
    /// Await completion of the run and return the final result.
    pub async fn finish(self) -> Result<RunResult, OdysseyCoreError> {
        self.handle
            .await
            .map_err(|err| OdysseyCoreError::Executor(err.to_string()))?
    }
}

/// Control how the base system prompt is resolved for an agent.
#[derive(Debug, Clone)]
pub enum SystemPromptMode {
    /// Use the orchestrator default prompt from config.
    OrchestratorDefault,
    /// Override the orchestrator prompt with a custom prompt.
    Override(String),
    /// Append additional content to the orchestrator prompt.
    Append(String),
}

#[derive(Clone)]
struct RunEventBus {
    sender: broadcast::Sender<EventMsg>,
}

impl RunEventBus {
    fn new(buffer: usize) -> (Self, broadcast::Receiver<EventMsg>) {
        let (sender, receiver) = broadcast::channel(buffer);
        (Self { sender }, receiver)
    }
}

impl EventSink for RunEventBus {
    fn emit(&self, event: EventMsg) {
        let _ = self.sender.send(event);
    }
}

struct FanoutEventSink {
    primary: Option<Arc<dyn EventSink>>,
    secondary: Arc<dyn EventSink>,
}

impl EventSink for FanoutEventSink {
    fn emit(&self, event: EventMsg) {
        if let Some(primary) = &self.primary {
            primary.emit(event.clone());
        }
        self.secondary.emit(event);
    }
}

/// Main orchestration façade: registers agents, manages sessions, and runs turns.
pub struct Orchestrator {
    config: Arc<OdysseyConfig>,
    tool_router: ToolRouter,
    permission_engine: Arc<PermissionEngine>,
    question_handler: Arc<RwLock<Option<Arc<dyn QuestionHandler>>>>, //TODO: Might not be needed, handle from PROTOCOL
    llm_registry: LLMRegistry,
    agent_registry: AgentRegistry,
    session_store: SessionStore,
    executor: Arc<TurnExecutor>,
    skill_store: Option<Arc<dyn SkillProvider>>,
    event_sink: Option<Arc<dyn EventSink>>,
}

impl Orchestrator {
    /// Construct a new orchestrator with optional overrides.
    pub fn new(
        config: OdysseyConfig,
        tools: ToolRegistry,
        sandbox_provider: Option<Arc<dyn SandboxProvider>>,
        state_store: Option<Arc<dyn StateStore>>,
        skill_store: Option<Arc<dyn SkillProvider>>,
        event_sink: Option<Arc<dyn EventSink>>,
    ) -> Result<Self, OdysseyCoreError> {
        info!("initializing orchestrator");
        debug!(
            "orchestrator config flags (skills={:?}, sessions={}, sandbox={})",
            config.skills, config.sessions.enabled, config.sandbox.enabled
        );
        let skill_store: Option<Arc<dyn SkillProvider>> = if skill_store.is_some() {
            skill_store
        } else {
            let cwd = std::env::current_dir().map_err(OdysseyCoreError::Io)?;
            debug!("loading skills (cwd={})", cwd.display());

            Some(Arc::new(
                SkillStore::load(&config.skills, &cwd)
                    .map_err(|err| OdysseyCoreError::Parse(err.to_string()))?,
            ))
        };

        let state_store = if config.sessions.enabled {
            match state_store {
                Some(store) => Some(store),
                None => Some(build_default_state_store(&config.sessions)?),
            }
        } else {
            None
        };
        let event_sink = event_sink.clone();
        let permission_engine = Arc::new(PermissionEngine::new(config.permissions.clone())?);
        permission_engine.set_event_sink(event_sink.clone());
        let sandbox_provider = if sandbox_provider.is_none() && sandbox_required(&config) {
            Some(build_default_sandbox_provider(&config.sandbox)?)
        } else {
            sandbox_provider
        };
        let config = Arc::new(config);
        let question_handler = Arc::new(RwLock::new(None));
        let agent_registry = AgentRegistry::new(DEFAULT_AGENT_ID.into());
        let session_store = SessionStore::new(state_store.clone());
        let tool_context_factory = ToolContextFactory::new(
            config.clone(),
            sandbox_provider.clone(),
            permission_engine.clone(),
            question_handler.clone(),
            skill_store.clone(),
            event_sink.clone(),
        );
        let tool_router = ToolRouter::new(tools);
        debug!("tool registry wired (tools={})", tool_router.list().len());

        let executor = Arc::new(TurnExecutor::new(
            config.clone(),
            session_store.clone(),
            tool_context_factory.clone(),
            tool_router.clone(),
            event_sink.clone(),
        ));

        let llm_registry = LLMRegistry::new("default_LLM".into());

        let orchestrator = Self {
            config,
            tool_router,
            permission_engine,
            question_handler,
            agent_registry,
            session_store,
            executor,
            skill_store,
            llm_registry,
            event_sink,
        };

        if orchestrator.config.sandbox.enabled && sandbox_provider.is_none() {
            warn!("sandbox enabled without provider configured");
            return Err(OdysseyCoreError::Sandbox(
                "sandbox enabled but no provider configured".to_string(),
            ));
        }

        info!("orchestrator initialized");
        Ok(orchestrator)
    }

    /// Return the shared configuration for this orchestrator.
    pub fn config(&self) -> &OdysseyConfig {
        &self.config
    }

    /// Set an approval handler to resolve permission requests.
    pub fn set_approval_handler(&self, handler: Arc<dyn ApprovalHandler>) {
        self.permission_engine.set_approval_handler(Some(handler));
    }

    /// Add a permission hook for side-effectful approvals.
    pub fn add_permission_hook(&self, hook: Arc<dyn PermissionHook>) {
        self.permission_engine.add_hook(hook);
    }

    /// Resolve a pending permission request by id.
    pub fn resolve_approval(
        &self,
        request_id: Uuid,
        decision: odyssey_rs_protocol::ApprovalDecision,
    ) -> bool {
        self.permission_engine
            .resolve_approval(request_id, decision)
    }

    /// List pending approval requests.
    pub fn list_pending_approvals(&self) -> Vec<ApprovalRequest> {
        self.permission_engine.list_pending_approvals()
    }

    /// List summaries for all registered agents.
    pub fn list_agent_info(&self) -> Vec<AgentInfo> {
        let default_id = self.agent_registry.default_agent_id();
        let mut summaries = Vec::new();
        let mut agent_ids = self.agent_registry.list_agents();
        agent_ids.sort();
        for agent_id in agent_ids {
            if let Ok(entry) = self.agent_registry.get_entry(&agent_id) {
                summaries.push(AgentInfo {
                    id: entry.id,
                    description: entry.description,
                    model: entry.model,
                    tool_policy: entry.tool_policy,
                    permission_mode: entry.permission_mode,
                    is_default: agent_id == default_id,
                });
            }
        }
        summaries
    }

    /// Fetch a single agent summary by id.
    pub fn get_agent_info(&self, agent_id: &str) -> Result<AgentInfo, OdysseyCoreError> {
        let entry = self.agent_registry.get_entry(agent_id)?;
        let default_id = self.agent_registry.default_agent_id();
        Ok(AgentInfo {
            id: entry.id,
            description: entry.description,
            model: entry.model,
            tool_policy: entry.tool_policy,
            permission_mode: entry.permission_mode,
            is_default: agent_id == default_id,
        })
    }

    /// Register a question handler for interactive tool queries.
    pub fn set_question_handler(&self, handler: Arc<dyn QuestionHandler>) {
        *self.question_handler.write() = Some(handler);
    }

    pub fn register_llm_provider(&self, entry: LLMEntry) -> Result<(), OdysseyCoreError> {
        let id = entry.id.clone();
        // self.ensure_non_default_agent_id(&id)?;
        info!("registering LLM (llm_id={})", id);

        self.llm_registry.insert_entry(entry);
        Ok(())
    }

    pub fn register_agent<T>(&self, agent: AgentBuilder<T>) -> Result<(), OdysseyCoreError>
    where
        T: OdysseyAgentRuntime,
        String: From<<T as AgentExecutor>::Output>, //TODO: Instead of String directly Add AgentOutput for orchestrator agent
    {
        let id = agent.id().to_string();
        self.ensure_non_default_agent_id(&id)?;
        if self.agent_registry.get_entry(&id).is_ok() {
            return Err(OdysseyCoreError::Executor(format!(
                "agent already registered: {id}"
            )));
        }
        info!("registering agent (agent_id={})", id);
        let entry = self.build_entry_from_agent(agent)?;
        let set_default = self.agent_registry.list_agents().is_empty();
        self.permission_engine
            .register_agent_mode(id.clone(), entry.permission_mode);
        self.agent_registry.insert_entry(entry);
        if set_default {
            self.agent_registry.set_default_agent_id(id)?;
        }
        Ok(())
    }

    fn build_entry_from_agent<T>(
        &self,
        agent: AgentBuilder<T>,
    ) -> Result<AgentEntry, OdysseyCoreError>
    where
        T: OdysseyAgentRuntime,
        String: From<<T as AgentExecutor>::Output>,
    {
        let id = agent.id().to_string();
        let description = agent.description().trim();
        let description = if description.is_empty() {
            None
        } else {
            Some(description.to_string())
        };
        let prompt = agent.description().to_string();
        let tool_policy = agent.tool_policy();
        let memory_provider = agent.memory_provider();
        let executor: Arc<dyn agent_factory::AgentExecutorRunner> =
            Arc::new(AutoAgentsExecutor::new(agent));

        Ok(AgentEntry::new(
            id,
            description,
            prompt,
            None,
            tool_policy,
            None,
            None,
            None,
            memory_provider,
            executor,
        ))
    }

    /// Override the default agent id used for new sessions.
    pub fn set_default_agent_id(
        &self,
        agent_id: impl Into<String>,
    ) -> Result<(), OdysseyCoreError> {
        self.agent_registry.set_default_agent_id(agent_id)
    }

    /// Return the current default agent id.
    pub fn default_agent_id(&self) -> String {
        self.agent_registry.default_agent_id()
    }

    /// List registered agent ids.
    pub fn list_agents(&self) -> Vec<String> {
        self.agent_registry.list_agents()
    }

    /// List registered LLM provider ids.
    pub fn list_llm_ids(&self) -> Vec<String> {
        self.llm_registry.list_llm_ids()
    }

    /// List registered tool names.
    pub fn list_tools(&self) -> Vec<String> {
        self.tool_router.list()
    }

    /// Return summaries of loaded skills.
    pub fn list_skill_summaries(&self) -> Vec<SkillSummary> {
        self.skill_store
            .as_ref()
            .map(|store| store.list())
            .unwrap_or_default()
    }

    /// Create a new session for the specified agent (or default).
    pub fn create_session(&self, agent_id: Option<String>) -> Result<SessionId, OdysseyCoreError> {
        let agent_id = self.agent_registry.resolve_agent_id(agent_id.as_deref())?;
        info!("creating session (agent_id={})", agent_id);
        self.session_store.create_session(agent_id)
    }

    /// Resume a session and return its state.
    pub fn resume_session(&self, session_id: SessionId) -> Result<Session, OdysseyCoreError> {
        self.session_store.resume_session(session_id)
    }

    /// List all persisted sessions.
    pub fn list_sessions(&self) -> Result<Vec<SessionSummary>, OdysseyCoreError> {
        self.session_store.list_sessions()
    }

    /// Delete a session and any associated overrides.
    pub fn delete_session(&self, session_id: SessionId) -> Result<bool, OdysseyCoreError> {
        info!("deleting session (session_id={})", session_id);
        self.session_store.delete_session(session_id)
    }

    /// Run a single turn, creating a fresh session.
    pub async fn run(
        &self,
        agent_id: Option<&str>,
        llm_id: Option<&str>,
        input: impl Into<String>, //TODO: Accept Images as well, Look at AutoAgents Task which has that
    ) -> Result<RunResult, OdysseyCoreError> {
        let agent_id = self.agent_registry.resolve_agent_id(agent_id)?;
        let llm_id = self.llm_registry.resolve_llm_id(llm_id)?;
        let session_id = self.create_session(Some(agent_id.clone()))?;
        let input_prompt = input.into();
        info!(
            "running turn in new session (session_id={}, agent_id={}, prompt_len={})",
            session_id,
            agent_id,
            input_prompt.len()
        );
        self.run_in_session(session_id, &agent_id, &llm_id, input_prompt)
            .await
    }

    /// Run a single turn in an existing session.
    pub async fn run_in_session(
        &self,
        session_id: SessionId,
        agent_id: &str,
        llm_id: &str,
        input: String,
    ) -> Result<RunResult, OdysseyCoreError> {
        debug!(
            "running session turn (session_id={}, agent_id={}, prompt_len={})",
            session_id,
            agent_id,
            input.len()
        );
        let entry = self.agent_registry.get_entry(agent_id)?;
        let llm = self.resovle_llm(llm_id)?;
        self.executor
            .run_turn(runtime::TurnParams {
                session_id,
                agent_id: agent_id.to_string(),
                llm,
                input,
                entry,
                include_subagent_spawner: true,
                tool_result_mode: ToolResultMode::SessionAndMemory,
                memory_mode: runtime::MemoryMode::AgentProvider,
                turn_id: None,
                event_sink: None,
                stream: false,
            })
            .await
    }

    /// Run a single turn and stream events, creating a fresh session.
    pub async fn run_stream(
        &self,
        agent_id: Option<&str>,
        llm_id: Option<&str>,
        input: impl Into<String>,
    ) -> Result<RunStream, OdysseyCoreError> {
        let agent_id = self.agent_registry.resolve_agent_id(agent_id)?;
        let llm_id = self.llm_registry.resolve_llm_id(llm_id)?;
        let session_id = self.create_session(Some(agent_id.clone()))?;
        let input_prompt = input.into();
        info!(
            "streaming turn in new session (session_id={}, agent_id={}, prompt_len={})",
            session_id,
            agent_id,
            input_prompt.len()
        );
        self.run_stream_in_session(session_id, &agent_id, &llm_id, input_prompt)
            .await
    }

    /// Run a single turn in an existing session and stream events.
    pub async fn run_stream_in_session(
        &self,
        session_id: SessionId,
        agent_id: &str,
        llm_id: &str,
        input: String,
    ) -> Result<RunStream, OdysseyCoreError> {
        debug!(
            "streaming session turn (session_id={}, agent_id={}, prompt_len={})",
            session_id,
            agent_id,
            input.len()
        );
        let entry = self.agent_registry.get_entry(agent_id)?;
        let llm = self.resovle_llm(llm_id)?;
        let turn_id = Uuid::new_v4();
        let (run_bus, receiver) = RunEventBus::new(RUN_STREAM_BUFFER);
        let run_bus = Arc::new(run_bus);
        let fanout: Arc<dyn EventSink> = Arc::new(FanoutEventSink {
            primary: self.event_sink.clone(),
            secondary: run_bus,
        });
        let executor = self.executor.clone();
        let agent_id = agent_id.to_string();
        let handle = tokio::spawn(async move {
            executor
                .run_turn(runtime::TurnParams {
                    session_id,
                    agent_id,
                    llm,
                    input,
                    entry,
                    include_subagent_spawner: true,
                    tool_result_mode: ToolResultMode::SessionAndMemory,
                    memory_mode: runtime::MemoryMode::AgentProvider,
                    turn_id: Some(turn_id),
                    event_sink: Some(fanout),
                    stream: true,
                })
                .await
        });

        Ok(RunStream {
            session_id,
            turn_id,
            events: BroadcastStream::new(receiver),
            handle,
        })
    }

    fn resovle_llm(&self, llm_id: &str) -> Result<Arc<dyn LLMProvider>, OdysseyCoreError> {
        Ok(self.llm_registry.get_entry(llm_id)?.provider)
    }

    fn ensure_non_default_agent_id(&self, id: &str) -> Result<(), OdysseyCoreError> {
        if id == self.agent_registry.default_agent_id()
            && !self.agent_registry.list_agents().is_empty()
        {
            return Err(OdysseyCoreError::Executor(
                "agent id conflicts with default orchestrator agent".to_string(),
            ));
        }
        Ok(())
    }
}

/// Build the default state store from config.
fn build_default_state_store(
    config: &SessionsConfig,
) -> Result<Arc<dyn StateStore>, OdysseyCoreError> {
    let root = resolve_default_root(config.path.as_ref(), "sessions")?;
    info!("initializing session store (root={})", root.display());
    let store =
        JsonlStateStore::new(root).map_err(|err| OdysseyCoreError::State(err.to_string()))?;
    Ok(Arc::new(store))
}

/// Build the default sandbox provider from config and platform defaults.
fn build_default_sandbox_provider(
    config: &odyssey_rs_config::SandboxConfig,
) -> Result<Arc<dyn SandboxProvider>, OdysseyCoreError> {
    let provider = config
        .provider
        .as_deref()
        .unwrap_or_else(|| default_provider_name(config.mode))
        .to_lowercase();
    info!("initializing sandbox provider (provider={})", provider);
    match provider.as_str() {
        "bubblewrap" | "bwrap" => BubblewrapProvider::new()
            .map(|provider| Arc::new(provider) as Arc<dyn SandboxProvider>)
            .map_err(|err| OdysseyCoreError::Sandbox(err.to_string())),
        "local" | "none" | "nosandbox" => Ok(Arc::new(LocalSandboxProvider::new())),
        other => Err(OdysseyCoreError::Sandbox(format!(
            "unsupported sandbox provider: {other}"
        ))),
    }
}

/// Determine whether any sandbox provider is required by config.
fn sandbox_required(config: &OdysseyConfig) -> bool {
    if config.sandbox.enabled {
        return true;
    }
    false
}

/// Resolve an absolute storage root for config-specified paths.
fn resolve_default_root(
    path: Option<&String>,
    fallback_dir: &str,
) -> Result<PathBuf, OdysseyCoreError> {
    let cwd = std::env::current_dir().map_err(OdysseyCoreError::Io)?;
    if let Some(path) = path {
        let path = PathBuf::from(path);
        if path.is_absolute() {
            debug!("using absolute storage root: {}", path.display());
            return Ok(path);
        }
        debug!(
            "resolving storage root relative to cwd: {}",
            cwd.join(&path).display()
        );
        return Ok(cwd.join(path));
    }

    if let Some(home) = BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf()) {
        debug!(
            "resolving storage root under home: {}",
            home.join(".odyssey").join(fallback_dir).display()
        );
        return Ok(home.join(".odyssey").join(fallback_dir));
    }

    Ok(cwd.join(".odyssey").join(fallback_dir))
}

#[cfg(test)]
mod tests {
    use super::{build_default_sandbox_provider, resolve_default_root, sandbox_required};
    use odyssey_rs_config::{OdysseyConfig, SandboxConfig};
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    #[test]
    fn sandbox_required_respects_config() {
        let mut config = OdysseyConfig::default();
        config.sandbox.enabled = false;
        assert_eq!(sandbox_required(&config), false);
        config.sandbox.enabled = true;
        assert_eq!(sandbox_required(&config), true);
    }

    #[test]
    fn resolve_default_root_respects_absolute_and_relative_paths() {
        let temp = tempdir().expect("tempdir");
        let absolute = temp.path().join("sessions");
        let absolute_str = absolute.to_string_lossy().to_string();
        let resolved = resolve_default_root(Some(&absolute_str), "sessions").expect("absolute");
        assert_eq!(resolved, absolute);

        let relative = "tmp/sessions".to_string();
        let cwd = std::env::current_dir().expect("cwd");
        let resolved = resolve_default_root(Some(&relative), "sessions").expect("relative");
        assert_eq!(resolved, cwd.join(&relative));
    }

    #[test]
    fn build_default_sandbox_provider_accepts_local() {
        let config = SandboxConfig {
            enabled: true,
            provider: Some("local".to_string()),
            ..SandboxConfig::default()
        };
        let provider = build_default_sandbox_provider(&config).expect("provider");
        let report = provider.dependency_report();
        assert_eq!(report.errors.is_empty(), true);
    }
}
