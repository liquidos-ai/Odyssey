//! Registry for tool implementations.

use crate::tool::{Tool, ToolSpec};
use log::debug;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

/// In-memory registry for tool implementations.
#[derive(Default, Clone)]
pub struct ToolRegistry {
    /// Map of tool name to implementation.
    tools: Arc<RwLock<HashMap<String, Arc<dyn Tool>>>>,
}

impl ToolRegistry {
    /// Create an empty tool registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a tool by name.
    pub fn register(&self, tool: Arc<dyn Tool>) {
        debug!("registering tool (name={})", tool.name());
        self.tools.write().insert(tool.name().to_string(), tool);
    }

    /// Fetch a tool by name.
    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.read().get(name).cloned()
    }

    /// List all registered tool names.
    pub fn list(&self) -> Vec<String> {
        self.tools.read().keys().cloned().collect()
    }

    /// Return all registered tool instances.
    pub fn all(&self) -> Vec<Arc<dyn Tool>> {
        self.tools.read().values().cloned().collect()
    }

    /// Return tool specs for all registered tools.
    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools.read().values().map(|tool| tool.spec()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::ToolRegistry;
    use crate::{Tool, ToolContext};
    use async_trait::async_trait;
    use odyssey_rs_protocol::ToolError;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::fmt;
    use std::sync::Arc;

    #[derive(Clone)]
    struct DummyTool {
        name: &'static str,
    }

    impl fmt::Debug for DummyTool {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "DummyTool({})", self.name)
        }
    }

    #[async_trait]
    impl Tool for DummyTool {
        fn name(&self) -> &str {
            self.name
        }

        fn description(&self) -> &str {
            "dummy"
        }

        fn args_schema(&self) -> serde_json::Value {
            json!({})
        }

        async fn call(
            &self,
            _ctx: &ToolContext,
            _args: serde_json::Value,
        ) -> Result<serde_json::Value, ToolError> {
            Ok(json!({}))
        }
    }

    #[test]
    fn registry_tracks_tools_and_specs() {
        let registry = ToolRegistry::new();
        registry.register(Arc::new(DummyTool { name: "Read" }));
        registry.register(Arc::new(DummyTool { name: "Write" }));

        let mut names = registry.list();
        names.sort();
        assert_eq!(names, vec!["Read", "Write"]);

        let specs = registry.specs();
        let mut spec_names = specs.into_iter().map(|spec| spec.name).collect::<Vec<_>>();
        spec_names.sort();
        assert_eq!(spec_names, vec!["Read", "Write"]);
    }
}
