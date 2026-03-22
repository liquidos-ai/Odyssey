use odyssey_rs_manifest::{AgentSpec, BundleManifest};
use odyssey_rs_tools::ToolRegistry;
use std::collections::BTreeSet;
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

    if !agent.tools.allow.iter().any(|entry| entry == "*") {
        let allowed = explicitly_available_tools(agent);
        if !allowed.is_empty() {
            names.retain(|name| allowed.contains(name));
        }
    }

    let denied = explicitly_denied_tools(agent);
    names.retain(|name| !denied.contains(name));
    names.sort();
    names.dedup();
    names
        .into_iter()
        .filter_map(|name| registry.get(&name))
        .collect()
}

fn explicitly_available_tools(agent: &AgentSpec) -> BTreeSet<String> {
    agent
        .tools
        .allow
        .iter()
        .chain(&agent.tools.ask)
        .filter(|entry| entry.as_str() != "*")
        .map(|entry| permission_rule_tool_name(entry))
        .collect()
}

fn explicitly_denied_tools(agent: &AgentSpec) -> BTreeSet<String> {
    agent
        .tools
        .deny
        .iter()
        .filter(|entry| entry.as_str() != "*")
        .filter(|entry| !entry.contains('('))
        .map(|entry| permission_rule_tool_name(entry))
        .collect()
}

fn permission_rule_tool_name(entry: &str) -> String {
    entry
        .split_once('(')
        .map_or(entry, |(tool, _)| tool)
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::select_tools;
    use async_trait::async_trait;
    use odyssey_rs_manifest::{
        AgentSpec, AgentToolPolicy, BundleExecutor, BundleManifest, BundleMemory, BundleSandbox,
        BundleTool, ManifestVersion, ProviderKind,
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
            manifest_version: ManifestVersion::V1,
            readme: "README.md".to_string(),
            agent_spec: "agent.yaml".to_string(),
            executor: BundleExecutor {
                kind: ProviderKind::Prebuilt,
                id: "react".to_string(),
                config: Value::Null,
            },
            memory: BundleMemory::default(),
            skills: Vec::new(),
            tools: tools
                .into_iter()
                .map(|name| BundleTool {
                    name: name.to_string(),
                    source: "builtin".to_string(),
                })
                .collect(),
            sandbox: BundleSandbox::default(),
        }
    }

    fn agent(allow: Vec<&str>, ask: Vec<&str>, deny: Vec<&str>) -> AgentSpec {
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
                ask: ask.into_iter().map(ToString::to_string).collect(),
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
            &agent(vec!["*"], Vec::new(), Vec::new()),
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
    fn select_tools_applies_manifest_and_agent_allow_filters() {
        let selected = select_tools(
            &registry(),
            &manifest(vec!["Write", "Read", "Write", "Missing"]),
            &agent(vec!["Read", "Write"], Vec::new(), Vec::new()),
        );

        assert_eq!(
            selected
                .into_iter()
                .map(|tool| tool.name().to_string())
                .collect::<Vec<_>>(),
            vec!["Read".to_string(), "Write".to_string()]
        );
    }

    #[test]
    fn select_tools_includes_prompted_and_granular_tools() {
        let selected = select_tools(
            &registry(),
            &manifest(vec!["Read", "Write", "Skill"]),
            &agent(
                vec!["Read"],
                vec!["Write", "Skill(repo-hygiene)"],
                Vec::new(),
            ),
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
    fn select_tools_keeps_granular_denies_but_removes_exact_denies() {
        let selected = select_tools(
            &registry(),
            &manifest(vec!["Read", "Write", "Skill"]),
            &agent(
                vec!["*"],
                Vec::new(),
                vec!["Write", "Skill(repo-hygiene)", "Missing"],
            ),
        );

        assert_eq!(
            selected
                .into_iter()
                .map(|tool| tool.name().to_string())
                .collect::<Vec<_>>(),
            vec!["Read".to_string(), "Skill".to_string()]
        );
    }
}
