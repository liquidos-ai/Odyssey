use async_trait::async_trait;
use autoagents_core::agent::task::Task;
use autoagents_core::agent::{
    AgentDeriveT, AgentExecutor, AgentHooks, Context, ExecutorConfig, HookOutcome,
};
use autoagents_core::tool::ToolT;
use autoagents_llm::ToolCall;
use parking_lot::Mutex;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct DummyAgent {
    calls: Arc<Mutex<Vec<&'static str>>>,
    name: String,
    description: String,
    output: String,
}

impl DummyAgent {
    pub fn new() -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            name: "dummy-agent".to_string(),
            description: "dummy".to_string(),
            output: "ok".to_string(),
        }
    }

    pub fn with_output(mut self, output: impl Into<String>) -> Self {
        self.output = output.into();
        self
    }

    pub fn calls(&self) -> Arc<Mutex<Vec<&'static str>>> {
        self.calls.clone()
    }

    fn record(&self, name: &'static str) {
        self.calls.lock().push(name);
    }
}

impl Default for DummyAgent {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AgentDeriveT for DummyAgent {
    type Output = String;

    fn description(&self) -> &str {
        &self.description
    }

    fn output_schema(&self) -> Option<serde_json::Value> {
        None
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn tools(&self) -> Vec<Box<dyn ToolT>> {
        Vec::new()
    }
}

#[async_trait]
impl AgentExecutor for DummyAgent {
    type Output = String;
    type Error = std::convert::Infallible;

    fn config(&self) -> ExecutorConfig {
        ExecutorConfig { max_turns: 1 }
    }

    async fn execute(
        &self,
        _task: &Task,
        _context: Arc<Context>,
    ) -> Result<Self::Output, Self::Error> {
        Ok(self.output.clone())
    }
}

#[async_trait]
impl AgentHooks for DummyAgent {
    async fn on_agent_create(&self) {
        self.record("create");
    }

    async fn on_run_start(&self, _task: &Task, _ctx: &Context) -> HookOutcome {
        self.record("run_start");
        HookOutcome::Continue
    }

    async fn on_run_complete(&self, _task: &Task, _result: &Self::Output, _ctx: &Context) {
        self.record("run_complete");
    }

    async fn on_turn_start(&self, _turn_index: usize, _ctx: &Context) {
        self.record("turn_start");
    }

    async fn on_turn_complete(&self, _turn_index: usize, _ctx: &Context) {
        self.record("turn_complete");
    }

    async fn on_tool_call(&self, _tool_call: &ToolCall, _ctx: &Context) -> HookOutcome {
        self.record("tool_call");
        HookOutcome::Continue
    }

    async fn on_tool_start(&self, _tool_call: &ToolCall, _ctx: &Context) {
        self.record("tool_start");
    }

    async fn on_tool_result(
        &self,
        _tool_call: &ToolCall,
        _result: &autoagents_core::tool::ToolCallResult,
        _ctx: &Context,
    ) {
        self.record("tool_result");
    }

    async fn on_tool_error(&self, _tool_call: &ToolCall, _err: serde_json::Value, _ctx: &Context) {
        self.record("tool_error");
    }

    async fn on_agent_shutdown(&self) {
        self.record("shutdown");
    }
}
