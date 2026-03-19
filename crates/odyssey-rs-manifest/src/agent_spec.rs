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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct AgentToolPolicy {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}
