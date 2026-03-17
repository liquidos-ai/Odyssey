use crate::{Tool, ToolContext};
use async_trait::async_trait;
use autoagents_core::tool::{ToolCallError, ToolRuntime, ToolT};
use serde_json::Value;
use std::fmt;
use std::sync::Arc;

#[derive(Clone)]
pub struct ToolAdaptor {
    tool: Arc<dyn Tool>,
    ctx: ToolContext,
}

impl ToolAdaptor {
    pub fn new(tool: Arc<dyn Tool>, ctx: ToolContext) -> Self {
        Self { tool, ctx }
    }
}

impl fmt::Debug for ToolAdaptor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolAdaptor")
            .field("name", &self.tool.name())
            .finish()
    }
}

#[async_trait]
impl ToolRuntime for ToolAdaptor {
    async fn execute(&self, args: Value) -> Result<Value, ToolCallError> {
        self.tool
            .call(&self.ctx, args)
            .await
            .map_err(|err| ToolCallError::RuntimeError(Box::new(err)))
    }
}

impl ToolT for ToolAdaptor {
    fn name(&self) -> &str {
        self.tool.name()
    }

    fn description(&self) -> &str {
        self.tool.description()
    }

    fn args_schema(&self) -> Value {
        self.tool.args_schema()
    }
}

pub fn tool_to_adaptor(tool: Arc<dyn Tool>, ctx: ToolContext) -> Arc<dyn ToolT> {
    Arc::new(ToolAdaptor::new(tool, ctx))
}

pub fn tools_to_adaptors(tools: Vec<Arc<dyn Tool>>, ctx: ToolContext) -> Vec<Arc<dyn ToolT>> {
    tools
        .into_iter()
        .map(|tool| tool_to_adaptor(tool, ctx.clone()))
        .collect()
}
