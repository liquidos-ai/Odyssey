//! Turn execution flow for orchestrator and subagents.

use super::agent_factory::AgentInput;
use super::memory::{
    capture_policy_from_config, compaction_policy_from_config, recall_options_from_config,
};
use super::registry::AgentEntry;
use super::sessions::SessionStore;
use super::tool_context::ToolContextFactory;
use crate::agent::memory::OdysseyMemoryAdapter;
use crate::error::OdysseyCoreError;
use crate::tools::ToolRouter;
use crate::types::{Message, Role, SessionId};
use autoagents_core::agent::memory::{MemoryProvider, SlidingWindowMemory};
use autoagents_llm::LLMProvider;
use futures_util::StreamExt;
use log::{debug, error, info};
use odyssey_rs_config::MemoryConfig;
use odyssey_rs_protocol::EventSink;
use odyssey_rs_protocol::ToolError;
use odyssey_rs_protocol::{EventMsg, EventPayload, ModelSpec, TurnContext, TurnId};
use odyssey_rs_tools::{ToolContext, ToolResultHandler};
use parking_lot::RwLock;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// Selects how tool results are captured during a turn.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ToolResultMode {
    /// Store tool results in session history (memory handled by the agent).
    SessionAndMemory,
    /// Do not store tool results in session history.
    #[allow(dead_code)]
    MemoryOnly,
}

/// Selects the memory strategy for a turn.
#[derive(Debug, Clone, Copy)]
pub(crate) enum MemoryMode {
    /// Use the agent-configured memory provider.
    AgentProvider,
    /// Use an ephemeral sliding window for subagents.
    #[allow(dead_code)]
    SubagentWindow { window_size: usize },
}

/// Parameters for a single turn execution.
pub(crate) struct TurnParams {
    pub(crate) session_id: SessionId,
    pub(crate) agent_id: String,
    pub(crate) llm: Arc<dyn LLMProvider>,
    ///Input Message from user
    pub(crate) input: String,
    pub(crate) entry: AgentEntry,
    pub(crate) include_subagent_spawner: bool,
    pub(crate) tool_result_mode: ToolResultMode,
    pub(crate) memory_mode: MemoryMode,
    pub(crate) turn_id: Option<TurnId>,
    pub(crate) event_sink: Option<Arc<dyn EventSink>>,
    pub(crate) stream: bool,
}

/// Executes a single turn with prompt assembly and tool wiring.
pub(crate) struct TurnExecutor {
    /// Shared configuration snapshot.
    config: Arc<odyssey_rs_config::OdysseyConfig>,
    /// Session persistence store.
    session_store: SessionStore,
    /// Tool context factory for per-turn tool wiring.
    tool_context_factory: ToolContextFactory,
    /// Tool router for policy-based tool selection.
    tool_router: ToolRouter,
    /// Optional event sink for turn lifecycle events.
    event_sink: Option<Arc<dyn EventSink>>,
}

impl TurnExecutor {
    /// Create a new executor for orchestrator and subagent turns.
    pub(crate) fn new(
        config: Arc<odyssey_rs_config::OdysseyConfig>,
        session_store: SessionStore,
        tool_context_factory: ToolContextFactory,
        tool_router: ToolRouter,
        event_sink: Option<Arc<dyn EventSink>>,
    ) -> Self {
        Self {
            config,
            session_store,
            tool_context_factory,
            tool_router,
            event_sink,
        }
    }

    /// Execute a single agent turn end-to-end.
    pub(crate) async fn run_turn(
        &self,
        params: TurnParams,
    ) -> Result<crate::orchestrator::RunResult, OdysseyCoreError> {
        let TurnParams {
            session_id,
            agent_id,
            llm,
            input,
            entry,
            include_subagent_spawner,
            tool_result_mode,
            memory_mode,
            turn_id,
            event_sink,
            stream,
        } = params;

        let event_sink = event_sink.or_else(|| self.event_sink.clone());
        let turn_id = turn_id.unwrap_or_else(Uuid::new_v4);
        info!(
            "starting turn (session_id={}, agent_id={}, prompt_len={}, subagents={})",
            session_id,
            agent_id,
            input.len(),
            include_subagent_spawner,
        );
        let memory_config = self.resolve_memory_config(&entry);
        let capture_policy = capture_policy_from_config(&memory_config.capture);
        let compaction_policy = compaction_policy_from_config(&memory_config.compaction);
        let recall_options = recall_options_from_config(&memory_config.recall);
        let system_prompt = entry.prompt.clone();
        let turn_context = self.build_turn_context(&entry)?;

        let tool_result_handler = self.build_tool_result_handler(tool_result_mode);
        let (sandbox_enabled, sandbox_mode) = self.resolve_sandbox(&entry);
        let tool_context = self
            .tool_context_factory
            .build_turn_context(
                session_id,
                &agent_id,
                turn_id,
                sandbox_enabled,
                sandbox_mode,
                tool_result_handler,
                event_sink.clone(),
            )
            .await?;
        let tool_context = Arc::new(RwLock::new(tool_context));
        let tools = self
            .tool_router
            .tools_for_agent(&entry.tool_policy, tool_context.clone());
        let executor = entry.executor.clone();
        let memory: Option<Box<dyn MemoryProvider>> = match memory_mode {
            MemoryMode::AgentProvider => Some(Box::new(OdysseyMemoryAdapter::new(
                session_id,
                agent_id.clone(),
                entry.memory_provider.clone(),
                capture_policy.clone(),
                compaction_policy.clone(),
                recall_options,
                Some(memory_config.recall_k),
            ))),
            MemoryMode::SubagentWindow { window_size } => {
                Some(Box::new(SlidingWindowMemory::new(window_size)))
            }
        };

        let agent_input = AgentInput {
            session_id,
            prompt: input.clone(),
            system_prompt: Some(system_prompt),
        };

        let event_sink_clone = event_sink.clone();
        let response = if stream {
            let stream_sink = event_sink.clone().ok_or_else(|| {
                OdysseyCoreError::Executor("streaming requires event sink".into())
            })?;
            let mut stream = executor
                .run_stream(
                    agent_input,
                    turn_id,
                    turn_context.clone(),
                    tools,
                    llm,
                    memory,
                    stream_sink.clone(),
                )
                .await?;
            stream_sink.emit(EventMsg {
                id: Uuid::new_v4(),
                session_id,
                created_at: chrono::Utc::now(),
                payload: EventPayload::TurnStarted {
                    turn_id,
                    context: turn_context,
                },
            });
            let mut response = String::new();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk?;
                if chunk.is_empty() {
                    continue;
                }

                let (delta, next_response) = if chunk.starts_with(&response) {
                    let delta = chunk[response.len()..].to_string();
                    (delta, chunk)
                } else {
                    let mut next_response = response.clone();
                    next_response.push_str(&chunk);
                    (chunk, next_response)
                };

                response = next_response;
                if !delta.is_empty() {
                    stream_sink.emit(EventMsg {
                        id: Uuid::new_v4(),
                        session_id,
                        created_at: chrono::Utc::now(),
                        payload: EventPayload::AgentMessageDelta { turn_id, delta },
                    });
                }
            }
            stream_sink.emit(EventMsg {
                id: Uuid::new_v4(),
                session_id,
                created_at: chrono::Utc::now(),
                payload: EventPayload::TurnCompleted {
                    turn_id,
                    message: response.clone(),
                },
            });
            Ok(response)
        } else {
            executor
                .run(
                    agent_input,
                    turn_id,
                    turn_context,
                    tools,
                    llm,
                    memory,
                    event_sink.clone(),
                )
                .await
        };
        let response = match response {
            Ok(response) => response,
            Err(err) => {
                error!(
                    "turn execution failed (session_id={}, agent_id={}, turn_id={})",
                    session_id, agent_id, turn_id
                );
                self.emit_event(
                    event_sink_clone,
                    session_id,
                    EventPayload::Error {
                        turn_id: Some(turn_id),
                        message: err.to_string(),
                    },
                );
                return Err(err);
            }
        };

        let user_message = Message {
            role: Role::User,
            content: input,
            created_at: chrono::Utc::now(),
        };
        let assistant_message = Message {
            role: Role::Assistant,
            content: response.clone(),
            created_at: chrono::Utc::now(),
        };

        self.session_store
            .append_message(session_id, &user_message)?;
        self.session_store
            .append_message(session_id, &assistant_message)?;

        info!(
            "completed turn (session_id={}, agent_id={}, turn_id={}, response_len={})",
            session_id,
            agent_id,
            turn_id,
            response.len()
        );
        Ok(crate::orchestrator::RunResult {
            session_id,
            response,
        })
    }

    /// Build a turn context populated from config and agent entry.
    pub(crate) fn build_turn_context(
        &self,
        entry: &AgentEntry,
    ) -> Result<TurnContext, OdysseyCoreError> {
        let cwd = std::env::current_dir()
            .map_err(OdysseyCoreError::Io)?
            .display()
            .to_string();
        let model = entry.model.as_ref().map(model_spec_from_config);
        let (sandbox_enabled, sandbox_mode) = self.resolve_sandbox(entry);
        let sandbox_mode = if sandbox_enabled {
            Some(sandbox_mode)
        } else {
            None
        };

        Ok(TurnContext {
            cwd: Some(cwd),
            model,
            sandbox_mode,
            approval_policy: None,
            metadata: json!({}),
        })
    }

    /// Resolve memory configuration for an agent entry.
    pub(crate) fn resolve_memory_config(&self, entry: &AgentEntry) -> MemoryConfig {
        entry
            .memory
            .clone()
            .unwrap_or_else(|| self.config.memory.clone())
    }

    /// Resolve sandbox enablement and mode for the agent entry.
    pub(crate) fn resolve_sandbox(
        &self,
        entry: &AgentEntry,
    ) -> (bool, odyssey_rs_protocol::SandboxMode) {
        let mut enabled = self.config.sandbox.enabled;
        let mut mode = self.config.sandbox.mode;
        if let Some(agent_sandbox) = entry.sandbox.as_ref() {
            if let Some(agent_enabled) = agent_sandbox.enabled {
                enabled = agent_enabled;
            }
            if let Some(agent_mode) = agent_sandbox.mode {
                mode = agent_mode;
            }
        }
        (enabled, mode)
    }

    /// Build a tool result handler chain based on capture policy and mode.
    fn build_tool_result_handler(
        &self,
        mode: ToolResultMode,
    ) -> Option<Arc<dyn ToolResultHandler>> {
        match mode {
            ToolResultMode::SessionAndMemory => Some(Arc::new(SessionToolResultHandler {
                sessions: self.session_store.sessions(),
                state_store: self.session_store.state_store(),
            })),
            ToolResultMode::MemoryOnly => None,
        }
    }

    /// Emit a turn-scoped event if an event sink is configured.
    fn emit_event(
        &self,
        event_sink: Option<Arc<dyn EventSink>>,
        session_id: SessionId,
        payload: EventPayload,
    ) {
        let Some(sink) = event_sink else {
            return;
        };
        let event = EventMsg {
            id: Uuid::new_v4(),
            session_id,
            created_at: chrono::Utc::now(),
            payload,
        };
        sink.emit(event);
    }
}

/// Convert a model config into a protocol model spec.
fn model_spec_from_config(model: &odyssey_rs_config::ModelConfig) -> ModelSpec {
    ModelSpec {
        provider: model.provider.clone(),
        name: model.name.clone(),
    }
}

/// Tool result handler that writes tool output into session history.
struct SessionToolResultHandler {
    sessions: Arc<RwLock<HashMap<SessionId, crate::types::Session>>>,
    state_store: Option<Arc<dyn crate::state::StateStore>>,
}

#[async_trait::async_trait]
impl ToolResultHandler for SessionToolResultHandler {
    /// Record a tool result into session history, applying output policy.
    async fn record_tool_result(
        &self,
        ctx: &ToolContext,
        name: &str,
        args: &serde_json::Value,
        result: &serde_json::Value,
    ) -> Result<(), ToolError> {
        const MAX_TOOL_LOG_CHARS: usize = 2000;
        debug!(
            "recording tool result into session (session_id={}, tool_name={})",
            ctx.session_id, name
        );
        let args_value = ctx.apply_output_policy(args.clone());
        let result_value = ctx.apply_output_policy(result.clone());
        let args_text = summarize_json(&args_value, MAX_TOOL_LOG_CHARS)?;
        let result_text = summarize_json(&result_value, MAX_TOOL_LOG_CHARS)?;
        let content = if result_text.is_empty() {
            format!("tool {name}\nargs: {args_text}")
        } else {
            format!("tool {name}\nargs: {args_text}\nresult: {result_text}")
        };
        let message = Message {
            role: Role::System,
            content,
            created_at: chrono::Utc::now(),
        };

        if let Some(session) = self.sessions.write().get_mut(&ctx.session_id) {
            session.messages.push(message.clone());
        }

        if let Some(store) = &self.state_store {
            let record = crate::state::MessageRecord {
                role: message.role.as_str().to_string(),
                content: message.content.clone(),
                created_at: message.created_at,
            };
            store
                .append_message(ctx.session_id, &record)
                .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
        }

        Ok(())
    }
}

/// Serialize JSON and truncate it to a max character count.
fn summarize_json(value: &serde_json::Value, max_chars: usize) -> Result<String, ToolError> {
    let text =
        serde_json::to_string(value).map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
    if text.chars().count() <= max_chars {
        Ok(text)
    } else {
        let truncated: String = text.chars().take(max_chars).collect();
        Ok(format!("{truncated}…"))
    }
}
