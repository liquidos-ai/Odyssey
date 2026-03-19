use odyssey_rs_manifest::{AgentSpec, BundleManifest};
use odyssey_rs_tools::ToolRegistry;
use std::sync::Arc;

pub fn select_tools(
    registry: &ToolRegistry,
    manifest: &BundleManifest,
    agent: &AgentSpec,
) -> Vec<Arc<dyn odyssey_rs_tools::Tool>> {
    let mut names = if manifest.tools.is_empty() {
        registry.names()
    } else {
        manifest
            .tools
            .iter()
            .map(|tool| tool.name.clone())
            .collect()
    };
    if !agent.tools.allow.is_empty() && !agent.tools.allow.iter().any(|entry| entry == "*") {
        names.retain(|name| agent.tools.allow.iter().any(|entry| entry == name));
    }
    names.retain(|name| !agent.tools.deny.iter().any(|entry| entry == name));
    names.sort();
    names.dedup();
    names
        .into_iter()
        .filter_map(|name| registry.get(&name))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::select_tools;
    use async_trait::async_trait;
    use odyssey_rs_manifest::{
        AgentSpec, AgentToolPolicy, BundleExecutor, BundleManifest, BundleMemory, BundleSandbox,
        BundleServer, BundleTool,
    };
    use odyssey_rs_protocol::ModelSpec;
    use odyssey_rs_tools::{Tool, ToolContext, ToolError, ToolRegistry};
    use pretty_assertions::assert_eq;
    use serde_json::{Value, json};
    use std::sync::Arc;

    #[derive(Debug)]
    struct DummyTool(&'static str);

    #[async_trait]
    impl Tool for DummyTool {
        fn name(&self) -> &str {
            self.0
        }

        fn description(&self) -> &str {
            self.0
        }

        fn args_schema(&self) -> Value {
            json!({"type": "object"})
        }

        async fn call(&self, _ctx: &ToolContext, _args: Value) -> Result<Value, ToolError> {
            Ok(Value::Null)
        }
    }

    fn manifest(tools: Vec<&str>) -> BundleManifest {
        BundleManifest {
            id: "demo".to_string(),
            version: "0.1.0".to_string(),
            agent_spec: "agent.yaml".to_string(),
            executor: BundleExecutor {
                kind: "prebuilt".to_string(),
                id: "react".to_string(),
                config: Value::Null,
            },
            memory: BundleMemory::default(),
            resources: Vec::new(),
            skills: Vec::new(),
            tools: tools
                .into_iter()
                .map(|name| BundleTool {
                    name: name.to_string(),
                    source: "builtin".to_string(),
                })
                .collect(),
            server: BundleServer::default(),
            sandbox: BundleSandbox::default(),
        }
    }

    fn agent(allow: Vec<&str>, deny: Vec<&str>) -> AgentSpec {
        AgentSpec {
            id: "demo".to_string(),
            description: String::default(),
            prompt: "test".to_string(),
            model: ModelSpec {
                provider: "openai".to_string(),
                name: "gpt-4.1-mini".to_string(),
                config: None,
            },
            tools: AgentToolPolicy {
                allow: allow.into_iter().map(ToString::to_string).collect(),
                deny: deny.into_iter().map(ToString::to_string).collect(),
            },
        }
    }

    fn registry() -> ToolRegistry {
        let mut registry = ToolRegistry::new();
        registry.register(Arc::new(DummyTool("Read")));
        registry.register(Arc::new(DummyTool("Skill")));
        registry.register(Arc::new(DummyTool("Write")));
        registry
    }

    #[test]
    fn select_tools_uses_registry_when_manifest_tools_are_empty() {
        let selected = select_tools(
            &registry(),
            &manifest(Vec::new()),
            &agent(vec!["*"], vec![]),
        );

        assert_eq!(
            selected
                .into_iter()
                .map(|tool| tool.name().to_string())
                .collect::<Vec<_>>(),
            vec!["Read".to_string(), "Skill".to_string(), "Write".to_string()]
        );
    }

    #[test]
    fn select_tools_applies_manifest_allow_and_deny_filters() {
        let selected = select_tools(
            &registry(),
            &manifest(vec!["Write", "Read", "Write", "Missing"]),
            &agent(vec!["Read", "Write"], vec!["Read"]),
        );

        assert_eq!(
            selected
                .into_iter()
                .map(|tool| tool.name().to_string())
                .collect::<Vec<_>>(),
            vec!["Write".to_string()]
        );
    }
}
