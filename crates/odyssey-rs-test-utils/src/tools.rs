use async_trait::async_trait;
use autoagents_core::tool::{ToolCallError, ToolRuntime, ToolT};
use odyssey_rs_protocol::ToolError;
use odyssey_rs_tools::ToolContext;
use serde_json::{Value, json};

#[derive(Debug, Clone)]
pub struct DummyTool {
    name: String,
    description: String,
    args_schema: Value,
    result: Value,
}

impl DummyTool {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: "dummy".to_string(),
            args_schema: json!({}),
            result: json!({}),
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    pub fn with_result(mut self, result: Value) -> Self {
        self.result = result;
        self
    }

    pub fn with_args_schema(mut self, schema: Value) -> Self {
        self.args_schema = schema;
        self
    }
}

#[async_trait]
impl odyssey_rs_tools::Tool for DummyTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn args_schema(&self) -> Value {
        self.args_schema.clone()
    }

    async fn call(&self, _ctx: &ToolContext, _args: Value) -> Result<Value, ToolError> {
        Ok(self.result.clone())
    }
}

#[derive(Debug, Clone)]
pub struct DummyToolRuntime {
    name: String,
    description: String,
    args_schema: Value,
    result: Value,
}

impl DummyToolRuntime {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: "dummy tool".to_string(),
            args_schema: json!({}),
            result: json!({}),
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    pub fn with_result(mut self, result: Value) -> Self {
        self.result = result;
        self
    }

    pub fn with_args_schema(mut self, schema: Value) -> Self {
        self.args_schema = schema;
        self
    }
}

#[async_trait]
impl ToolRuntime for DummyToolRuntime {
    async fn execute(&self, _args: Value) -> Result<Value, ToolCallError> {
        Ok(self.result.clone())
    }
}

impl ToolT for DummyToolRuntime {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn args_schema(&self) -> Value {
        self.args_schema.clone()
    }
}
