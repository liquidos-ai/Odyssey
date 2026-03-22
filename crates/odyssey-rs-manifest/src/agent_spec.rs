use odyssey_rs_protocol::ModelSpec;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSpec {
    pub id: String,
    #[serde(default)]
    pub description: String,
    pub prompt: String,
    pub model: ModelSpec,
    #[serde(default)]
    pub tools: AgentToolPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentToolPolicy {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub ask: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::{AgentSpec, AgentToolPolicy};
    use odyssey_rs_protocol::ModelSpec;
    use pretty_assertions::assert_eq;
    use serde_yaml::from_str;

    #[test]
    fn agent_tool_policy_defaults_to_empty_permission_groups() {
        let spec: AgentSpec = from_str(
            r#"
id: demo
prompt: hello
model:
  provider: openai
  name: gpt-4.1-mini
"#,
        )
        .expect("deserialize agent spec");

        assert_eq!(spec.description, String::default());
        assert_eq!(spec.tools, AgentToolPolicy::default());
    }

    #[test]
    fn agent_tool_policy_deserializes_grouped_permissions() {
        let spec: AgentSpec = from_str(
            r#"
id: demo
prompt: hello
model:
  provider: openai
  name: gpt-4.1-mini
tools:
  allow: ['Read', 'Bash(curl:*)']
  ask: ['Write', 'Bash']
  deny: ['Skill']
"#,
        )
        .expect("deserialize grouped tool policy");

        assert_eq!(
            spec.tools,
            AgentToolPolicy {
                allow: vec!["Read".to_string(), "Bash(curl:*)".to_string()],
                ask: vec!["Write".to_string(), "Bash".to_string()],
                deny: vec!["Skill".to_string()],
            }
        );
    }

    #[test]
    fn agent_spec_model_shape_is_unchanged() {
        let spec = AgentSpec {
            id: "demo".to_string(),
            description: String::default(),
            prompt: "hello".to_string(),
            model: ModelSpec {
                provider: "openai".to_string(),
                name: "gpt-4.1-mini".to_string(),
                config: None,
            },
            tools: AgentToolPolicy::default(),
        };

        assert_eq!(spec.model.provider, "openai");
        assert_eq!(spec.model.name, "gpt-4.1-mini");
    }
}
