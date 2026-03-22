use odyssey_rs_protocol::SandboxMode;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    #[serde(default = "default_builtin_source")]
    pub source: String,
}

fn default_builtin_source() -> String {
    "builtin".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleSandbox {
    #[serde(default = "default_sandbox_mode")]
    pub mode: SandboxMode,
    #[serde(default)]
    pub permissions: BundleSandboxPermissions,
    #[serde(default)]
    pub system_tools: Vec<String>,
    #[serde(default)]
    pub resources: BundleSandboxLimits,
}

fn default_sandbox_mode() -> SandboxMode {
    SandboxMode::WorkspaceWrite
}

impl Default for BundleSandbox {
    fn default() -> Self {
        Self {
            mode: default_sandbox_mode(),
            permissions: BundleSandboxPermissions::default(),
            system_tools: Vec::new(),
            resources: BundleSandboxLimits::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct BundleSandboxPermissions {
    #[serde(default)]
    pub filesystem: BundleSandboxFilesystem,
    #[serde(default)]
    pub network: Vec<String>,
    #[serde(default)]
    pub tools: BundleSandboxTools,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct BundleSandboxFilesystem {
    #[serde(default)]
    pub exec: Vec<String>,
    #[serde(default)]
    pub mounts: BundleSandboxMounts,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct BundleSandboxMounts {
    #[serde(default)]
    pub read: Vec<String>,
    #[serde(default)]
    pub write: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleSandboxTools {
    #[serde(default = "default_mode")]
    pub mode: String,
    #[serde(default)]
    pub rules: Vec<BundlePermissionRule>,
}

impl Default for BundleSandboxTools {
    fn default() -> Self {
        Self {
            mode: default_mode(),
            rules: Vec::new(),
        }
    }
}

fn default_mode() -> String {
    "default".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundlePermissionRule {
    pub action: BundlePermissionAction,
    pub tool: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BundlePermissionAction {
    Allow,
    Deny,
    Ask,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct BundleSandboxLimits {
    #[serde(default)]
    pub cpu: Option<u64>,
    #[serde(default)]
    pub memory_mb: Option<u64>,
    #[serde(default)]
    pub gpu: Option<u64>,
}

#[cfg(test)]
mod tests {
    use crate::bundle_manifest::ProviderKind;

    use super::{
        BundleManifest, BundleMemory, BundlePermissionAction, BundleSandbox, BundleSandboxTools,
        default_builtin_source,
    };
    use odyssey_rs_protocol::SandboxMode;
    use pretty_assertions::assert_eq;
    use serde_json::{Value, json};

    #[test]
    fn defaults_match_v1_contract() {
        let memory = BundleMemory::default();
        assert_eq!(memory.kind, ProviderKind::Prebuilt);
        assert_eq!(memory.id, "sliding_window");
        assert_eq!(memory.config, Value::Null);

        let sandbox = BundleSandbox::default();
        assert_eq!(sandbox.mode, SandboxMode::WorkspaceWrite);
        assert_eq!(sandbox.permissions.network, Vec::<String>::new());
        assert_eq!(sandbox.system_tools, Vec::<String>::new());

        let tools = BundleSandboxTools::default();
        assert_eq!(tools.mode, "default");
        assert_eq!(tools.rules.len(), 0);

        assert_eq!(default_builtin_source(), "builtin");
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
    }

    #[test]
    fn tool_and_permission_action_deserialization_uses_expected_values() {
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
            ],
            "sandbox": {
                "permissions": {
                    "tools": {
                        "mode": "strict",
                        "rules": [
                            { "action": "allow", "tool": "Read" },
                            { "action": "ask", "tool": "Write" },
                            { "action": "deny", "tool": "Bash" }
                        ]
                    }
                }
            }
        }))
        .expect("deserialize manifest with tool rules");

        assert_eq!(manifest.tools[0].source, "builtin");
        assert_eq!(manifest.tools[1].source, "builtin");
        assert_eq!(manifest.sandbox.permissions.tools.mode, "strict");
        assert_eq!(
            manifest.sandbox.permissions.tools.rules[0].action,
            BundlePermissionAction::Allow
        );
        assert_eq!(
            manifest.sandbox.permissions.tools.rules[1].action,
            BundlePermissionAction::Ask
        );
        assert_eq!(
            manifest.sandbox.permissions.tools.rules[2].action,
            BundlePermissionAction::Deny
        );
    }
}
