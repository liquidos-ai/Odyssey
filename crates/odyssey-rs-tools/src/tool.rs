//! Tool trait definition and metadata spec.

use crate::context::ToolContext;
use async_trait::async_trait;
use odyssey_rs_protocol::ToolError;
use serde_json::Value;
use std::fmt::Debug;

/// Tool metadata spec for discovery and schema presentation.
#[derive(Debug, Clone)]
pub struct ToolSpec {
    /// Tool name.
    pub name: String,
    /// Tool description.
    pub description: String,
    /// JSON schema for tool arguments.
    pub args_schema: Value,
}

/// Interface for executable tools.
#[async_trait]
pub trait Tool: Send + Sync + Debug {
    /// Return the tool name.
    fn name(&self) -> &str;
    /// Return the tool description.
    fn description(&self) -> &str;
    /// Return the JSON schema for tool arguments.
    fn args_schema(&self) -> Value;

    /// Whether the tool supports parallel execution.
    fn supports_parallel(&self) -> bool {
        false
    }

    /// Invoke the tool with a context and arguments.
    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError>;

    /// Build a `ToolSpec` describing this tool.
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().to_string(),
            description: self.description().to_string(),
            args_schema: self.args_schema(),
        }
    }
}
