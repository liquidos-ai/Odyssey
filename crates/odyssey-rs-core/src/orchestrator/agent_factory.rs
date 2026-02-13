//! AutoAgents executor wiring for orchestrator runs.

use crate::error::OdysseyCoreError;
use crate::types::{OdysseyAgentRuntime, SessionId};
use async_trait::async_trait;
use autoagents_core::agent::{
    AgentBuilder, AgentDeriveT, AgentExecutor, AgentHooks, DirectAgent, memory::MemoryProvider,
    task::Task,
};
use autoagents_core::agent::{Context, ExecutorConfig, HookOutcome};
use autoagents_core::tool::{ToolCallResult, ToolT, shared_tools_to_boxes};
use autoagents_llm::{LLMProvider, ToolCall};
use futures_util::{Stream, StreamExt};
use log::{debug, info};
use odyssey_rs_protocol::EventSink;
use odyssey_rs_protocol::{EventMsg, EventPayload, TurnContext, TurnId};
use std::sync::Arc;
use uuid::Uuid;

/// Input passed to AutoAgents executors for a single turn.
pub(crate) struct AgentInput {
    /// Target session identifier.
    pub(crate) session_id: SessionId,
    /// User prompt for the turn.
    pub(crate) prompt: String,
    /// System prompt to prepend for this run.
    pub(crate) system_prompt: Option<String>,
}

/// Execution shim for AutoAgents-backed agents.
#[async_trait]
pub(crate) trait AgentExecutorRunner: Send + Sync {
    #[allow(clippy::too_many_arguments)]
    async fn run(
        &self,
        input: AgentInput,
        turn_id: TurnId,
        turn_context: TurnContext,
        tools: Vec<Arc<dyn ToolT>>,
        llm: Arc<dyn LLMProvider>,
        memory: Option<Box<dyn MemoryProvider>>,
        event_sink: Option<Arc<dyn EventSink>>,
    ) -> Result<String, OdysseyCoreError>;

    #[allow(clippy::too_many_arguments)]
    async fn run_stream(
        &self,
        input: AgentInput,
        turn_id: TurnId,
        _turn_context: TurnContext,
        tools: Vec<Arc<dyn ToolT>>,
        llm: Arc<dyn LLMProvider>,
        memory: Option<Box<dyn MemoryProvider>>,
        _event_sink: Arc<dyn EventSink>,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<String, OdysseyCoreError>> + Send>>,
        OdysseyCoreError,
    >;
}

/// AutoAgents executor that runs a concrete agent type.
pub(crate) struct AutoAgentsExecutor<T>
where
    T: OdysseyAgentRuntime,
    String: From<<T as AgentExecutor>::Output>, //TODO: THis means only output of agent string types are allowed, Need to fix
{
    name: String,
    agent: T,
}

impl<T> AutoAgentsExecutor<T>
where
    T: OdysseyAgentRuntime,
    String: From<<T as AgentExecutor>::Output>,
{
    pub(crate) fn new(agent: T) -> Self {
        AutoAgentsExecutor {
            name: agent.name().into(),
            agent,
        }
    }
}

#[derive(Clone)]
struct ToolInjectedAgent<T>
where
    T: OdysseyAgentRuntime,
    String: From<<T as AgentExecutor>::Output>,
{
    inner: T,
    tools: Vec<Arc<dyn ToolT>>,
}

impl<T> std::fmt::Debug for ToolInjectedAgent<T>
where
    T: OdysseyAgentRuntime + std::fmt::Debug,
    String: From<<T as AgentExecutor>::Output>,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolInjectedAgent")
            .field("inner", &self.inner)
            .field("tools", &self.tools.len())
            .finish()
    }
}

impl<T> ToolInjectedAgent<T>
where
    T: OdysseyAgentRuntime,
    String: From<<T as AgentExecutor>::Output>,
{
    fn new(inner: T, tools: Vec<Arc<dyn ToolT>>) -> Self {
        Self { inner, tools }
    }
}

#[async_trait]
impl<T> AgentDeriveT for ToolInjectedAgent<T>
where
    T: OdysseyAgentRuntime,
    String: From<<T as AgentExecutor>::Output>,
{
    type Output = <T as AgentDeriveT>::Output;

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn output_schema(&self) -> Option<serde_json::Value> {
        self.inner.output_schema()
    }

    fn name(&self) -> &str {
        self.inner.name()
    }

    fn tools(&self) -> Vec<Box<dyn ToolT>> {
        shared_tools_to_boxes(&self.tools)
    }
}

#[async_trait]
impl<T> AgentHooks for ToolInjectedAgent<T>
where
    T: OdysseyAgentRuntime,
    String: From<<T as AgentExecutor>::Output>,
{
    async fn on_agent_create(&self) {
        self.inner.on_agent_create().await
    }

    async fn on_run_start(&self, task: &Task, ctx: &Context) -> HookOutcome {
        self.inner.on_run_start(task, ctx).await
    }

    async fn on_run_complete(&self, task: &Task, result: &Self::Output, ctx: &Context) {
        self.inner.on_run_complete(task, result, ctx).await
    }

    async fn on_turn_start(&self, turn_index: usize, ctx: &Context) {
        self.inner.on_turn_start(turn_index, ctx).await
    }

    async fn on_turn_complete(&self, turn_index: usize, ctx: &Context) {
        self.inner.on_turn_complete(turn_index, ctx).await
    }

    async fn on_tool_call(&self, tool_call: &ToolCall, ctx: &Context) -> HookOutcome {
        self.inner.on_tool_call(tool_call, ctx).await
    }

    async fn on_tool_start(&self, tool_call: &ToolCall, ctx: &Context) {
        self.inner.on_tool_start(tool_call, ctx).await
    }

    async fn on_tool_result(&self, tool_call: &ToolCall, result: &ToolCallResult, ctx: &Context) {
        self.inner.on_tool_result(tool_call, result, ctx).await
    }

    async fn on_tool_error(&self, tool_call: &ToolCall, err: serde_json::Value, ctx: &Context) {
        self.inner.on_tool_error(tool_call, err, ctx).await
    }

    async fn on_agent_shutdown(&self) {
        self.inner.on_agent_shutdown().await
    }
}

#[async_trait]
impl<T> AgentExecutor for ToolInjectedAgent<T>
where
    T: OdysseyAgentRuntime,
    String: From<<T as AgentExecutor>::Output>,
{
    type Output = <T as AgentExecutor>::Output;
    type Error = <T as AgentExecutor>::Error;

    fn config(&self) -> ExecutorConfig {
        self.inner.config()
    }

    async fn execute(
        &self,
        task: &Task,
        context: Arc<Context>,
    ) -> Result<Self::Output, Self::Error> {
        self.inner.execute(task, context).await
    }

    async fn execute_stream(
        &self,
        task: &Task,
        context: Arc<Context>,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<Self::Output, Self::Error>> + Send>>,
        Self::Error,
    > {
        self.inner.execute_stream(task, context).await
    }
}

#[async_trait]
impl<T> AgentExecutorRunner for AutoAgentsExecutor<T>
where
    T: OdysseyAgentRuntime,
    String: From<<T as AgentExecutor>::Output>,
{
    async fn run(
        &self,
        input: AgentInput,
        turn_id: TurnId,
        turn_context: TurnContext,
        tools: Vec<Arc<dyn ToolT>>,
        llm: Arc<dyn LLMProvider>,
        memory: Option<Box<dyn MemoryProvider>>,
        event_sink: Option<Arc<dyn EventSink>>,
    ) -> Result<String, OdysseyCoreError> {
        info!(
            "executor start (agent_id={}, session_id={}, turn_id={}, prompt_len={})",
            self.name,
            input.session_id,
            turn_id,
            input.prompt.len()
        );
        if let Some(sink) = event_sink.as_ref() {
            sink.emit(EventMsg {
                id: Uuid::new_v4(),
                session_id: input.session_id,
                created_at: chrono::Utc::now(),
                payload: EventPayload::TurnStarted {
                    turn_id,
                    context: turn_context,
                },
            });
        }
        let merged_tools = merge_tools(tools, self.agent.tools());
        let agent = ToolInjectedAgent::new(self.agent.clone(), merged_tools);
        let mut builder = AgentBuilder::<ToolInjectedAgent<T>, DirectAgent>::new(agent).llm(llm);
        if let Some(memory) = memory {
            builder = builder.memory(memory);
        }
        let handle = builder
            .build()
            .await
            .map_err(|err| OdysseyCoreError::Executor(err.to_string()))?;

        let task = match input.system_prompt {
            Some(system_prompt) => Task::new(input.prompt).with_system_prompt(system_prompt),
            None => Task::new(input.prompt),
        };

        let output = handle
            .agent
            .run(task)
            .await
            .map_err(|err| OdysseyCoreError::Executor(err.to_string()))?;
        let response: String = output;
        if let Some(sink) = event_sink.as_ref() {
            sink.emit(EventMsg {
                id: Uuid::new_v4(),
                session_id: input.session_id,
                created_at: chrono::Utc::now(),
                payload: EventPayload::TurnCompleted {
                    turn_id,
                    message: response.clone(),
                },
            });
        }
        info!(
            "executor complete (agent_id={}, session_id={}, turn_id={}, response_len={})",
            self.name,
            input.session_id,
            turn_id,
            response.len()
        );
        Ok(response)
    }

    async fn run_stream(
        &self,
        input: AgentInput,
        turn_id: TurnId,
        _turn_context: TurnContext,
        tools: Vec<Arc<dyn ToolT>>,
        llm: Arc<dyn LLMProvider>,
        memory: Option<Box<dyn MemoryProvider>>,
        _event_sink: Arc<dyn EventSink>,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<String, OdysseyCoreError>> + Send>>,
        OdysseyCoreError,
    > {
        info!(
            "executor start (agent_id={}, session_id={}, turn_id={}, prompt_len={})",
            self.name,
            input.session_id,
            turn_id,
            input.prompt.len()
        );
        let merged_tools = merge_tools(tools, self.agent.tools());
        let agent = ToolInjectedAgent::new(self.agent.clone(), merged_tools);
        let mut builder = AgentBuilder::<ToolInjectedAgent<T>, DirectAgent>::new(agent)
            .llm(llm)
            .stream(true);
        if let Some(memory) = memory {
            builder = builder.memory(memory);
        }
        let handle = builder
            .build()
            .await
            .map_err(|err| OdysseyCoreError::Executor(err.to_string()))?;

        let task = match input.system_prompt {
            Some(system_prompt) => Task::new(input.prompt).with_system_prompt(system_prompt),
            None => Task::new(input.prompt),
        };

        let stream = handle
            .agent
            .run_stream(task)
            .await
            .map_err(|err| OdysseyCoreError::Executor(err.to_string()))?;
        let mapped_stream =
            stream.map(|chunk| chunk.map_err(|err| OdysseyCoreError::Executor(err.to_string())));
        Ok(Box::pin(mapped_stream))
    }
}

fn merge_tools(
    registry_tools: Vec<Arc<dyn ToolT>>,
    agent_tools: Vec<Box<dyn ToolT>>,
) -> Vec<Arc<dyn ToolT>> {
    let mut tools = registry_tools;
    let mut names = tools
        .iter()
        .map(|tool| tool.name().to_string())
        .collect::<std::collections::HashSet<_>>();
    for tool in agent_tools {
        let name = tool.name().to_string();
        if names.insert(name.clone()) {
            tools.push(Arc::from(tool));
        } else {
            debug!("agent tool skipped due to registry collision (tool={name})");
        }
    }
    tools
}
