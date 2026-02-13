//! Adaptor for autoagents tool trait.

use crate::{Tool, ToolContext};
use async_trait::async_trait;
use autoagents_core::tool::{ToolCallError, ToolRuntime, ToolT};
use parking_lot::RwLock;
use serde_json::Value;
use std::fmt;
use std::sync::Arc;

/// Adapter that bridges Odyssey tools into AutoAgents runtime.
#[derive(Clone)]
pub struct ToolAdaptor {
    /// Wrapped tool implementation.
    tool: Arc<dyn Tool>,
    /// Shared tool context.
    ctx: Arc<RwLock<ToolContext>>,
}

impl ToolAdaptor {
    /// Create a new tool adaptor.
    pub fn new(tool: Arc<dyn Tool>, ctx: Arc<RwLock<ToolContext>>) -> Self {
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
    /// Execute a tool call, delegating the full pipeline to ToolContext.
    async fn execute(&self, args: Value) -> Result<Value, ToolCallError> {
        let mut ctx = self.ctx.read().clone();
        ctx.execute_tool(self.tool.as_ref(), args)
            .await
            .map_err(|err| ToolCallError::RuntimeError(Box::new(err)))
    }
}

impl ToolT for ToolAdaptor {
    /// Return the tool name.
    fn name(&self) -> &str {
        self.tool.name()
    }

    /// Return the tool description.
    fn description(&self) -> &str {
        self.tool.description()
    }

    /// Return the argument schema for the tool.
    fn args_schema(&self) -> Value {
        self.tool.args_schema()
    }
}

/// Wrap a tool with an adaptor for AutoAgents.
pub fn tool_to_adaptor(tool: Arc<dyn Tool>, ctx: Arc<RwLock<ToolContext>>) -> Arc<dyn ToolT> {
    Arc::new(ToolAdaptor::new(tool, ctx))
}

/// Wrap multiple tools with adaptors for AutoAgents.
pub fn tools_to_adaptors(
    tools: Vec<Arc<dyn Tool>>,
    ctx: Arc<RwLock<ToolContext>>,
) -> Vec<Arc<dyn ToolT>> {
    tools
        .into_iter()
        .map(|tool| tool_to_adaptor(tool, ctx.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{ToolAdaptor, tool_to_adaptor, tools_to_adaptors};
    use crate::{Tool, ToolContext, TurnServices};
    use async_trait::async_trait;
    use autoagents_core::tool::ToolRuntime;
    use odyssey_rs_protocol::ToolError;
    use parking_lot::RwLock;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::sync::Arc;
    use uuid::Uuid;

    #[derive(Debug)]
    struct DummyTool;

    #[async_trait]
    impl Tool for DummyTool {
        fn name(&self) -> &str {
            "Dummy"
        }

        fn description(&self) -> &str {
            "dummy tool"
        }

        fn args_schema(&self) -> serde_json::Value {
            json!({})
        }

        async fn call(
            &self,
            _ctx: &ToolContext,
            _args: serde_json::Value,
        ) -> Result<serde_json::Value, ToolError> {
            Ok(json!({ "ok": true }))
        }
    }

    fn base_context() -> ToolContext {
        ToolContext {
            session_id: Uuid::nil(),
            agent_id: "agent".to_string(),
            turn_id: None,
            tool_call_id: None,
            tool_name: None,
            services: Arc::new(TurnServices {
                cwd: ".".into(),
                workspace_root: ".".into(),
                output_policy: None,
                sandbox: None,
                web: None,
                event_sink: None,
                skill_provider: None,
                question_handler: None,
                permission_checker: None,
                tool_result_handler: None,
            }),
        }
    }

    #[tokio::test]
    async fn adaptor_executes_tool_calls() {
        let ctx = Arc::new(RwLock::new(base_context()));
        let adaptor = ToolAdaptor::new(Arc::new(DummyTool), ctx);
        let result = adaptor.execute(json!({})).await.expect("execute");
        assert_eq!(result, json!({ "ok": true }));
    }

    #[test]
    fn adaptor_helpers_wrap_tools() {
        let ctx = Arc::new(RwLock::new(base_context()));
        let tool = Arc::new(DummyTool);
        let adaptor = tool_to_adaptor(tool.clone(), ctx.clone());
        assert_eq!(adaptor.name(), "Dummy");

        let adaptors = tools_to_adaptors(vec![tool], ctx);
        assert_eq!(adaptors.len(), 1);
        assert_eq!(adaptors[0].name(), "Dummy");
    }
}
