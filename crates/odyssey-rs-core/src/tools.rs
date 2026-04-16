//! Tool routing and policy filtering for orchestrator usage.

use autoagents_core::tool::ToolT;
use log::debug;
use odyssey_rs_config::ToolPolicy;
use odyssey_rs_tools::{ToolContext, ToolRegistry, ToolSpec, tools_to_adaptors};
use parking_lot::RwLock;
use std::sync::Arc;

/// Router that filters and adapts tools based on policy.
#[derive(Clone)]
pub struct ToolRouter {
    /// Registry of available tools.
    registry: ToolRegistry,
}

impl ToolRouter {
    /// Create a new router around the given registry.
    pub fn new(registry: ToolRegistry) -> Self {
        Self { registry }
    }

    #[allow(dead_code)]
    /// List the names of all registered tools.
    pub fn list(&self) -> Vec<String> {
        self.registry.list()
    }

    #[allow(dead_code)]
    /// Build tool specs for an agent policy without adaptation.
    pub fn specs_for_agent(&self, policy: &ToolPolicy) -> Vec<ToolSpec> {
        let allow = &policy.allow;
        let deny = &policy.deny;
        self.registry
            .all()
            .into_iter()
            .filter(|tool| {
                let name = tool.name();
                if deny.iter().any(|entry| entry == name) {
                    return false;
                }
                if allow.is_empty() || allow.iter().any(|entry| entry == "*") {
                    return true;
                }
                allow.iter().any(|entry| entry == name)
            })
            .map(|tool| tool.spec())
            .collect()
    }

    /// Build adapted tool instances filtered by policy.
    pub fn tools_for_agent(
        &self,
        policy: &ToolPolicy,
        ctx: Arc<RwLock<ToolContext>>,
    ) -> Vec<Arc<dyn ToolT>> {
        let allow = &policy.allow;
        let deny = &policy.deny;
        let tools = self
            .registry
            .all()
            .into_iter()
            .filter(|tool| {
                let name = tool.name();
                if deny.iter().any(|entry| entry == name) {
                    return false;
                }
                if allow.is_empty() || allow.iter().any(|entry| entry == "*") {
                    return true;
                }
                allow.iter().any(|entry| entry == name)
            })
            .collect::<Vec<_>>();
        debug!(
            "tool selection resolved (allowed={}, denied={}, selected={})",
            allow.len(),
            deny.len(),
            tools.len()
        );
        tools_to_adaptors(tools, ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::ToolRouter;
    use odyssey_rs_config::ToolPolicy;
    use odyssey_rs_test_utils::{DummyTool, base_tool_context};
    use odyssey_rs_tools::ToolRegistry;
    use pretty_assertions::assert_eq;
    use std::sync::Arc;

    #[test]
    fn tool_router_filters_allowlist() {
        let registry = ToolRegistry::new();
        registry.register(Arc::new(DummyTool::new("Read")));
        registry.register(Arc::new(DummyTool::new("Write")));
        let router = ToolRouter::new(registry);

        let policy = ToolPolicy {
            allow: vec!["Read".to_string()],
            deny: Vec::new(),
        };
        let ctx = Arc::new(parking_lot::RwLock::new(base_tool_context()));
        let tools = router.tools_for_agent(&policy, ctx);
        let names = tools.iter().map(|tool| tool.name()).collect::<Vec<_>>();

        assert_eq!(names, vec!["Read"]);
    }

    #[test]
    fn tool_router_allows_star() {
        let registry = ToolRegistry::new();
        registry.register(Arc::new(DummyTool::new("Read")));
        registry.register(Arc::new(DummyTool::new("Write")));
        let router = ToolRouter::new(registry);

        let policy = ToolPolicy::allow_all();
        let ctx = Arc::new(parking_lot::RwLock::new(base_tool_context()));
        let tools = router.tools_for_agent(&policy, ctx);
        let mut names = tools.iter().map(|tool| tool.name()).collect::<Vec<_>>();
        names.sort();

        assert_eq!(names, vec!["Read", "Write"]);
    }

    #[test]
    fn tool_router_lists_and_builds_specs() {
        let registry = ToolRegistry::new();
        registry.register(Arc::new(DummyTool::new("Read")));
        registry.register(Arc::new(DummyTool::new("Write")));
        let router = ToolRouter::new(registry);

        let mut names = router.list();
        names.sort();
        assert_eq!(names, vec!["Read".to_string(), "Write".to_string()]);

        let policy = ToolPolicy {
            allow: vec!["Read".to_string()],
            deny: vec!["Write".to_string()],
        };
        let specs = router.specs_for_agent(&policy);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "Read".to_string());
    }
}
