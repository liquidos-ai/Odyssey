// Builder for custom agents registered with the orchestrator.

use std::sync::Arc;

use async_trait::async_trait;
use autoagents_core::agent::task::Task;
use autoagents_core::agent::{
    AgentDeriveT, AgentExecutor, AgentHooks, Context, ExecutorConfig, HookOutcome,
};
use autoagents_core::tool::{ToolCallResult, ToolT};
use autoagents_llm::ToolCall;
use futures_util::Stream;
use odyssey_rs_config::{
    AgentPermissionsConfig, AgentSandboxConfig, MemoryConfig, ModelConfig, ToolPolicy,
};
use serde_json::Value;

use crate::agent::AgentInstance;
use crate::types::{AgentID, OdysseyAgentRuntime};

#[derive(Clone)]
pub struct AgentBuilder<T> {
    id: String,
    inner: Arc<T>,
    tool_policy: ToolPolicy,
    description_override: Option<String>,
    model: Option<ModelConfig>,
    permission_mode: Option<odyssey_rs_config::PermissionMode>,
    sandbox: Option<AgentSandboxConfig>,
    memory: Option<MemoryConfig>,
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
            .field("description_override", &self.description_override)
            .field("model", &self.model)
            .field("permission_mode", &self.permission_mode)
            .field("sandbox", &self.sandbox)
            .field("memory", &self.memory)
            .finish()
    }
}

impl<T> AgentBuilder<T>
where
    T: OdysseyAgentRuntime,
    String: From<<T as AgentExecutor>::Output>,
{
    pub fn new(id: AgentID, agent: T) -> Self {
        Self {
            id,
            inner: Arc::new(agent),
            tool_policy: ToolPolicy::allow_all(),
            description_override: None,
            model: None,
            permission_mode: None,
            sandbox: None,
            memory: None,
        }
    }

    pub fn with_tool_policy(mut self, tool_policy: ToolPolicy) -> Self {
        self.tool_policy = tool_policy;
        self
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description_override = Some(description.into());
        self
    }

    pub fn with_model(mut self, model: ModelConfig) -> Self {
        self.model = Some(model);
        self
    }

    pub fn with_permissions(mut self, permissions: AgentPermissionsConfig) -> Self {
        self.permission_mode = permissions.mode;
        self
    }

    pub fn with_sandbox(mut self, sandbox: AgentSandboxConfig) -> Self {
        self.sandbox = Some(sandbox);
        self
    }

    pub fn with_memory(mut self, memory: MemoryConfig) -> Self {
        self.memory = Some(memory);
        self
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub(crate) fn tool_policy(&self) -> &ToolPolicy {
        &self.tool_policy
    }

    pub(crate) fn description_override(&self) -> Option<&str> {
        self.description_override.as_deref()
    }

    pub(crate) fn model(&self) -> Option<ModelConfig> {
        self.model.clone()
    }

    pub(crate) fn permission_mode(&self) -> Option<odyssey_rs_config::PermissionMode> {
        self.permission_mode
    }

    pub(crate) fn sandbox(&self) -> Option<AgentSandboxConfig> {
        self.sandbox.clone()
    }

    pub(crate) fn memory(&self) -> Option<MemoryConfig> {
        self.memory.clone()
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

    fn model(&self) -> Option<ModelConfig> {
        self.model()
    }

    fn permission_mode(&self) -> Option<odyssey_rs_config::PermissionMode> {
        self.permission_mode()
    }

    fn sandbox(&self) -> Option<AgentSandboxConfig> {
        self.sandbox()
    }

    fn memory(&self) -> Option<MemoryConfig> {
        self.memory()
    }
}

#[cfg(test)]
mod tests {
    use super::AgentBuilder;
    use crate::agent::AgentInstance;
    use autoagents_core::agent::task::Task;
    use autoagents_core::agent::{AgentDeriveT, AgentExecutor, AgentHooks, Context};
    use futures_util::StreamExt;
    use odyssey_rs_test_utils::{DummyAgent, FailingLLM};
    use pretty_assertions::assert_eq;
    use std::sync::Arc;

    #[tokio::test]
    async fn agent_builder_delegates_calls() {
        let agent = DummyAgent::new();
        let builder = AgentBuilder::new("agent".to_string(), agent.clone());

        assert_eq!(builder.id(), "agent");
        assert_eq!(builder.name(), "agent");
        assert_eq!(builder.description(), agent.description());
        assert_eq!(builder.output_schema(), agent.output_schema());

        let tools = builder.tools();
        assert_eq!(tools.len(), 0);

        let task = Task::new("hello");
        let ctx = Arc::new(Context::new(Arc::new(FailingLLM::new("unused")), None));
        let _ = builder.on_run_start(&task, &ctx).await;
        builder.on_turn_start(0, &ctx).await;
        let result = builder.execute(&task, ctx.clone()).await.expect("execute");
        assert_eq!(result, "ok".to_string());
        builder.on_run_complete(&task, &result, &ctx).await;
        builder.on_turn_complete(0, &ctx).await;

        let mut stream = builder.execute_stream(&task, ctx).await.expect("stream");
        let chunk = stream.next().await.expect("chunk").expect("value");
        assert_eq!(chunk, "ok".to_string());

        let provider = AgentInstance::tool_policy(&builder);
        assert_eq!(provider.allow, vec!["*".to_string()]);
    }
}
