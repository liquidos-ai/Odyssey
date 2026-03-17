use crate::{ToolContext, ToolError};
use async_trait::async_trait;
use serde_json::Value;
use std::fmt::Debug;

#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub args_schema: Value,
}

#[async_trait]
pub trait Tool: Send + Sync + Debug {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn args_schema(&self) -> Value;
    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError>;

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().to_string(),
            description: self.description().to_string(),
            args_schema: self.args_schema(),
        }
    }
}
