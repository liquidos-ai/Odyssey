use odyssey_rs_protocol::SandboxMode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Prebuilt,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ManifestVersion {
    #[default]
    #[serde(rename = "odyssey.bundle/v1")]
    V1,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleManifest {
    pub id: String,
    pub version: String,
    pub manifest_version: ManifestVersion,
    pub readme: String,
    pub agent_spec: String,
    pub executor: BundleExecutor,
    #[serde(default)]
    pub memory: BundleMemory,
    #[serde(default)]
    pub skills: Vec<BundleSkill>,
    #[serde(default)]
    pub tools: Vec<BundleTool>,
    #[serde(default)]
    pub sandbox: BundleSandbox,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleExecutor {
    #[serde(rename = "type")]
    pub kind: ProviderKind,
    pub id: String,
    #[serde(default)]
    pub config: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleMemory {
    #[serde(rename = "type")]
    pub kind: ProviderKind,
    pub id: String,
    #[serde(default)]
    pub config: Value,
}

impl Default for BundleMemory {
    fn default() -> Self {
        Self {
            kind: ProviderKind::Prebuilt,
            id: "sliding_window".to_string(),
            config: Value::Null,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleSkill {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleTool {
    pub name: String,
    #[serde(default = "builtin_tool_source")]
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleSandbox {
    #[serde(default = "default_sandbox_mode")]
    pub mode: SandboxMode,
    #[serde(default)]
    pub permissions: BundleSandboxPermissions,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub system_tools_mode: BundleSystemToolsMode,
    #[serde(default)]
    pub system_tools: Vec<String>,
    #[serde(default)]
    pub resources: BundleSandboxLimits,
}

impl Default for BundleSandbox {
    fn default() -> Self {
        Self {
            mode: default_sandbox_mode(),
            permissions: BundleSandboxPermissions::default(),
            env: BTreeMap::new(),
            system_tools_mode: BundleSystemToolsMode::default(),
            system_tools: Vec::new(),
            resources: BundleSandboxLimits::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BundleSystemToolsMode {
    #[default]
    Explicit,
    Standard,
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct BundleSandboxPermissions {
    #[serde(default)]
    pub filesystem: BundleSandboxFilesystem,
    #[serde(default)]
    pub network: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct BundleSandboxFilesystem {
    pub exec: Vec<String>,
    pub mounts: BundleSandboxMounts,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct BundleSandboxMounts {
    pub read: Vec<String>,
    pub write: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleSandboxLimits {
    pub cpu: Option<u64>,
    pub memory_mb: Option<u64>,
}

impl Default for BundleSandboxLimits {
    fn default() -> Self {
        Self {
            cpu: Some(1),
            memory_mb: Some(512),
        }
    }
}

fn builtin_tool_source() -> String {
    "builtin".to_string()
}

fn default_sandbox_mode() -> SandboxMode {
    SandboxMode::WorkspaceWrite
}

#[cfg(test)]
mod tests {
    use crate::bundle_manifest::ProviderKind;

    use super::{BundleManifest, BundleMemory, BundleSandbox, BundleSystemToolsMode};
    use odyssey_rs_protocol::SandboxMode;
    use pretty_assertions::assert_eq;
    use serde_json::{Value, json};
    use std::collections::BTreeMap;

    #[test]
    fn defaults_match_v1_contract() {
        let memory = BundleMemory::default();
        assert_eq!(memory.kind, ProviderKind::Prebuilt);
        assert_eq!(memory.id, "sliding_window");
        assert_eq!(memory.config, Value::Null);

        let sandbox = BundleSandbox::default();
        assert_eq!(sandbox.mode, SandboxMode::WorkspaceWrite);
        assert_eq!(sandbox.permissions.network, Vec::<String>::new());
        assert_eq!(sandbox.env, BTreeMap::new());
        assert_eq!(sandbox.system_tools_mode, BundleSystemToolsMode::Explicit);
        assert_eq!(sandbox.system_tools, Vec::<String>::new());
    }

    #[test]
    fn manifest_deserialization_applies_defaults() {
        let manifest: BundleManifest = serde_json::from_value(json!({
            "id": "demo",
            "version": "0.1.0",
            "manifest_version": "odyssey.bundle/v1",
            "readme": "README.md",
            "agent_spec": "agent.yaml",
            "executor": {
                "type": "prebuilt",
                "id": "react"
            }
        }))
        .expect("deserialize bundle manifest");

        assert_eq!(manifest.memory.kind, ProviderKind::Prebuilt);
        assert_eq!(manifest.memory.id, "sliding_window");
        assert_eq!(manifest.skills.len(), 0);
        assert_eq!(manifest.tools.len(), 0);
        assert_eq!(manifest.sandbox.mode, SandboxMode::WorkspaceWrite);
        assert_eq!(
            manifest.sandbox.permissions.filesystem.exec,
            Vec::<String>::new()
        );
        assert_eq!(
            manifest.sandbox.permissions.filesystem.mounts.read,
            Vec::<String>::new()
        );
        assert_eq!(
            manifest.sandbox.permissions.filesystem.mounts.write,
            Vec::<String>::new()
        );
        assert_eq!(
            manifest.sandbox.system_tools_mode,
            BundleSystemToolsMode::Explicit
        );
        assert_eq!(manifest.sandbox.env, BTreeMap::new());
    }

    #[test]
    fn manifest_rejects_legacy_sandbox_tool_permissions() {
        let error = serde_json::from_value::<BundleManifest>(json!({
            "id": "demo",
            "version": "0.1.0",
            "manifest_version": "odyssey.bundle/v1",
            "readme": "README.md",
            "agent_spec": "agent.yaml",
            "executor": {
                "type": "prebuilt",
                "id": "react"
            },
            "sandbox": {
                "permissions": {
                    "tools": {
                        "allow": ["Read"]
                    }
                }
            }
        }))
        .expect_err("deserialize manifest with legacy sandbox tool permissions");

        assert!(error.to_string().contains("unknown field `tools`"));
    }

    #[test]
    fn manifest_deserialization_accepts_builtin_tools() {
        let manifest: BundleManifest = serde_json::from_value(json!({
            "id": "demo",
            "version": "0.1.0",
            "manifest_version": "odyssey.bundle/v1",
            "readme": "README.md",
            "agent_spec": "agent.yaml",
            "executor": {
                "type": "prebuilt",
                "id": "react"
            },
            "tools": [
                { "name": "Read" },
                { "name": "Write", "source": "builtin" }
            ]
        }))
        .expect("deserialize manifest with builtin tools");

        assert_eq!(manifest.tools[0].source, "builtin");
        assert_eq!(manifest.tools[1].source, "builtin");
    }

    #[test]
    fn manifest_deserialization_accepts_system_tools_modes() {
        let manifest: BundleManifest = serde_json::from_value(json!({
            "id": "demo",
            "version": "0.1.0",
            "manifest_version": "odyssey.bundle/v1",
            "readme": "README.md",
            "agent_spec": "agent.yaml",
            "executor": {
                "type": "prebuilt",
                "id": "react"
            },
            "sandbox": {
                "system_tools_mode": "standard"
            }
        }))
        .expect("deserialize bundle manifest");

        assert_eq!(
            manifest.sandbox.system_tools_mode,
            BundleSystemToolsMode::Standard
        );
    }

    #[test]
    fn manifest_deserialization_accepts_sandbox_env() {
        let manifest: BundleManifest = serde_json::from_value(json!({
            "id": "demo",
            "version": "0.1.0",
            "manifest_version": "odyssey.bundle/v1",
            "readme": "README.md",
            "agent_spec": "agent.yaml",
            "executor": {
                "type": "prebuilt",
                "id": "react"
            },
            "sandbox": {
                "env": {
                    "OPENAI_API_KEY": "OPENAI_API_KEY",
                    "GITHUB_TOKEN": "GH_TOKEN"
                }
            }
        }))
        .expect("deserialize bundle manifest");

        assert_eq!(
            manifest.sandbox.env,
            BTreeMap::from([
                ("GITHUB_TOKEN".to_string(), "GH_TOKEN".to_string()),
                ("OPENAI_API_KEY".to_string(), "OPENAI_API_KEY".to_string()),
            ])
        );
    }
}
