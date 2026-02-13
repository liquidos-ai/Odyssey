// Builder for the orchestrator's default agent runtime.

use std::sync::Arc;

use async_trait::async_trait;
use autoagents_core::agent::task::Task;
use autoagents_core::agent::{
    AgentDeriveT, AgentExecutor, AgentHooks, Context, ExecutorConfig, HookOutcome,
};
use autoagents_core::tool::{ToolCallResult, ToolT};
use autoagents_llm::ToolCall;
use futures_util::Stream;
use odyssey_rs_config::ToolPolicy;
use odyssey_rs_memory::MemoryProvider;
use serde_json::Value;

use crate::agent::AgentInstance;
use crate::types::{AgentID, OdysseyAgentRuntime};

#[derive(Clone)]
pub struct AgentBuilder<T> {
    id: String,
    inner: Arc<T>,
    tool_policy: ToolPolicy,
    memory_provider: Arc<dyn MemoryProvider>,
}

impl<T> std::fmt::Debug for AgentBuilder<T>
where
    T: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentBuilder")
            .field("id", &self.id)
            .field("inner", &self.inner)
            .field("tool_policy", &self.tool_policy)
            .finish()
    }
}

impl<T> AgentBuilder<T>
where
    T: OdysseyAgentRuntime,
    String: From<<T as AgentExecutor>::Output>,
{
    /// Build a default agent from a fully constructed AutoAgents agent.
    /// Use `from_factory` or `odyssey` if you need Odyssey-managed tool injection.
    /// A memory provider is required for prompt recall.
    pub fn new(id: AgentID, agent: T, memory_provider: Arc<dyn MemoryProvider>) -> Self {
        let agent = Arc::new(agent);
        Self {
            id,
            inner: agent,
            tool_policy: ToolPolicy::allow_all(),
            memory_provider,
        }
    }

    /// Return the configured agent id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Return the tool policy assigned to this default agent.
    fn tool_policy(&self) -> &ToolPolicy {
        &self.tool_policy
    }

    /// Return the memory provider for this default agent.
    fn memory_provider(&self) -> Arc<dyn MemoryProvider> {
        self.memory_provider.clone()
    }
}

#[async_trait]
impl<T> AgentDeriveT for AgentBuilder<T>
where
    T: OdysseyAgentRuntime,
    String: From<<T as AgentExecutor>::Output>,
{
    type Output = <T as AgentDeriveT>::Output;

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn output_schema(&self) -> Option<Value> {
        self.inner.output_schema()
    }

    fn name(&self) -> &str {
        &self.id
    }

    fn tools(&self) -> Vec<Box<dyn ToolT>> {
        self.inner.tools()
    }
}

#[async_trait]
impl<T> AgentHooks for AgentBuilder<T>
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

    async fn on_tool_error(&self, tool_call: &ToolCall, err: Value, ctx: &Context) {
        self.inner.on_tool_error(tool_call, err, ctx).await
    }

    async fn on_agent_shutdown(&self) {
        self.inner.on_agent_shutdown().await
    }
}

#[async_trait]
impl<T> AgentExecutor for AgentBuilder<T>
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

impl<T> AgentInstance for AgentBuilder<T>
where
    T: OdysseyAgentRuntime,
    String: From<<T as AgentExecutor>::Output>,
{
    fn tool_policy(&self) -> ToolPolicy {
        self.tool_policy().clone()
    }

    fn memory_provider(&self) -> Arc<dyn MemoryProvider> {
        self.memory_provider()
    }
}

#[cfg(test)]
mod tests {
    use super::AgentBuilder;
    use crate::agent::AgentInstance;
    use autoagents_core::agent::task::Task;
    use autoagents_core::agent::{AgentDeriveT, AgentExecutor, AgentHooks, Context};
    use futures_util::StreamExt;
    use odyssey_rs_memory::MemoryProvider;
    use odyssey_rs_test_utils::{DummyAgent, FailingLLM, StubMemory};
    use pretty_assertions::assert_eq;
    use std::sync::Arc;

    #[tokio::test]
    async fn agent_builder_delegates_calls() {
        let agent = DummyAgent::new();
        let memory = Arc::new(StubMemory::default());
        let builder = AgentBuilder::new("agent".to_string(), agent.clone(), memory.clone());

        assert_eq!(builder.id(), "agent");
        assert_eq!(builder.description(), "dummy");
        assert_eq!(builder.name(), "agent");
        assert_eq!(builder.output_schema(), None);
        assert_eq!(builder.config().max_turns, 1);
        assert_eq!(builder.tools().len(), 0);

        let provider = AgentInstance::memory_provider(&builder);
        let memory_dyn: Arc<dyn MemoryProvider> = memory.clone();
        assert_eq!(Arc::ptr_eq(&provider, &memory_dyn), true);
        assert_eq!(builder.tool_policy().allow, vec!["*".to_string()]);

        builder.on_agent_create().await;
        let task = Task::new("hello");
        let ctx = Arc::new(Context::new(Arc::new(FailingLLM::new("dummy")), None));
        builder.on_run_start(&task, &ctx).await;
        builder.on_turn_start(0, &ctx).await;
        builder.on_turn_complete(0, &ctx).await;
        let tool_call = autoagents_llm::ToolCall {
            id: "call_1".to_string(),
            call_type: "function".to_string(),
            function: autoagents_llm::FunctionCall {
                name: "noop".to_string(),
                arguments: "{}".to_string(),
            },
        };
        builder.on_tool_call(&tool_call, &ctx).await;
        builder.on_tool_start(&tool_call, &ctx).await;
        builder
            .on_tool_result(
                &tool_call,
                &autoagents_core::tool::ToolCallResult {
                    tool_name: "noop".to_string(),
                    success: true,
                    arguments: serde_json::json!({}),
                    result: serde_json::json!({}),
                },
                &ctx,
            )
            .await;
        builder
            .on_tool_error(&tool_call, serde_json::json!({ "err": "boom" }), &ctx)
            .await;

        let result = builder.execute(&task, ctx.clone()).await.expect("execute");
        assert_eq!(result, "ok".to_string());
        builder.on_run_complete(&task, &result, &ctx).await;

        let stream = builder
            .execute_stream(
                &task,
                Arc::new(Context::new(Arc::new(FailingLLM::new("dummy")), None)),
            )
            .await
            .expect("stream");
        let outputs = stream.collect::<Vec<_>>().await;
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].as_ref().expect("ok"), "ok");
        builder.on_agent_shutdown().await;

        let calls_handle = agent.calls();
        let calls = calls_handle.lock();
        assert_eq!(
            calls.as_slice(),
            &[
                "create",
                "run_start",
                "turn_start",
                "turn_complete",
                "tool_call",
                "tool_start",
                "tool_result",
                "tool_error",
                "run_complete",
                "shutdown"
            ]
        );
    }
}
