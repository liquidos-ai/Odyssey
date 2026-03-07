//! Configuration schema for Odyssey.

use odyssey_rs_protocol::SandboxMode;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Root config for the Odyssey SDK.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OdysseyConfig {
    #[serde(default, rename = "$schema")]
    pub schema: Option<String>,
    #[serde(default)]
    pub agents: AgentsConfig,
    #[serde(default)]
    pub tools: ToolsConfig,
    #[serde(default)]
    pub permissions: PermissionsConfig,
    #[serde(default)]
    pub memory: MemoryConfig,
    #[serde(default)]
    pub skills: SkillsConfig,
    #[serde(default)]
    pub mcp: McpConfig,
    #[serde(default)]
    pub sandbox: SandboxConfig,
    #[serde(default)]
    pub sessions: SessionsConfig,
}

impl OdysseyConfig {
    /// Start building a config programmatically with defaults applied.
    pub fn builder() -> OdysseyConfigBuilder {
        OdysseyConfigBuilder::new()
    }
}

/// Builder for assembling an `OdysseyConfig` in code.
#[derive(Debug, Default, Clone)]
pub struct OdysseyConfigBuilder {
    config: OdysseyConfig,
}

impl OdysseyConfigBuilder {
    /// Create a new builder seeded with default config values.
    pub fn new() -> Self {
        Self {
            config: OdysseyConfig::default(),
        }
    }

    /// Replace the global tool configuration.
    pub fn tools(mut self, tools: ToolsConfig) -> Self {
        self.config.tools = tools;
        self
    }

    /// Replace the managed agent configuration.
    pub fn agents(mut self, agents: AgentsConfig) -> Self {
        self.config.agents = agents;
        self
    }

    /// Replace the global permissions configuration.
    pub fn permissions(mut self, permissions: PermissionsConfig) -> Self {
        self.config.permissions = permissions;
        self
    }

    /// Replace the global memory configuration.
    pub fn memory(mut self, memory: MemoryConfig) -> Self {
        self.config.memory = memory;
        self
    }

    /// Replace the global skills configuration.
    pub fn skills(mut self, skills: SkillsConfig) -> Self {
        self.config.skills = skills;
        self
    }

    /// Replace the global MCP configuration.
    pub fn mcp(mut self, mcp: McpConfig) -> Self {
        self.config.mcp = mcp;
        self
    }

    /// Replace the global sandbox configuration.
    pub fn sandbox(mut self, sandbox: SandboxConfig) -> Self {
        self.config.sandbox = sandbox;
        self
    }

    /// Replace the session persistence configuration.
    pub fn sessions(mut self, sessions: SessionsConfig) -> Self {
        self.config.sessions = sessions;
        self
    }

    /// Finalize and return the built `OdysseyConfig`.
    pub fn build(self) -> OdysseyConfig {
        self.config
    }
}

/// Model provider configuration for an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub provider: String,
    pub name: String,
}

/// Managed config-defined agent registrations.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentsConfig {
    #[serde(default)]
    pub list: Vec<ManagedAgentConfig>,
}

/// A config-defined ReAct agent managed by Odyssey.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedAgentConfig {
    pub id: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub model: Option<ModelConfig>,
    #[serde(default)]
    pub tools: ToolPolicy,
    #[serde(default)]
    pub memory: Option<MemoryConfig>,
    #[serde(default)]
    pub sandbox: Option<AgentSandboxConfig>,
    #[serde(default)]
    pub permissions: Option<AgentPermissionsConfig>,
}

/// Tool allow/deny policy for a single agent.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ToolPolicy {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

impl ToolPolicy {
    /// Build a policy that allows all tools.
    pub fn allow_all() -> Self {
        Self {
            allow: vec!["*".to_string()],
            deny: Vec::new(),
        }
    }
}

/// Global tool configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolsConfig {
    #[serde(default)]
    pub output_policy: ToolOutputPolicyConfig,
}

/// Output policy for tool results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutputPolicyConfig {
    #[serde(default = "default_max_string_bytes")]
    pub max_string_bytes: usize,
    #[serde(default = "default_max_array_len")]
    pub max_array_len: usize,
    #[serde(default = "default_max_object_entries")]
    pub max_object_entries: usize,
    #[serde(default)]
    pub redact_keys: Vec<String>,
    #[serde(default)]
    pub redact_values: Vec<String>,
    #[serde(default = "default_redaction_replacement")]
    pub replacement: String,
}

impl Default for ToolOutputPolicyConfig {
    fn default() -> Self {
        Self {
            max_string_bytes: default_max_string_bytes(),
            max_array_len: default_max_array_len(),
            max_object_entries: default_max_object_entries(),
            redact_keys: Vec::new(),
            redact_values: Vec::new(),
            replacement: default_redaction_replacement(),
        }
    }
}

/// Default maximum string size for tool output in bytes.
fn default_max_string_bytes() -> usize {
    32 * 1024
}

/// Default maximum array length for tool output.
fn default_max_array_len() -> usize {
    256
}

/// Default maximum object entry count for tool output.
fn default_max_object_entries() -> usize {
    256
}

/// Default replacement marker for redacted fields.
fn default_redaction_replacement() -> String {
    "[REDACTED]".to_string()
}

/// Memory backend configuration for an agent or global defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default = "default_recall_k")]
    pub recall_k: usize,
    #[serde(default)]
    pub instruction_roots: Vec<String>,
    #[serde(default)]
    pub capture_tool_output: bool,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            path: None,
            recall_k: default_recall_k(),
            instruction_roots: Vec::new(),
            capture_tool_output: false,
        }
    }
}

/// Default number of memory items to recall.
fn default_recall_k() -> usize {
    6
}

/// Per-agent sandbox overrides.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentSandboxConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub mode: Option<SandboxMode>,
}

/// Per-agent permission overrides.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentPermissionsConfig {
    #[serde(default)]
    pub mode: Option<PermissionMode>,
}

/// Skill discovery configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsConfig {
    #[serde(default, alias = "settingSources")]
    pub setting_sources: Vec<SettingSource>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default = "default_skill_allow")]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            setting_sources: vec![SettingSource::User],
            paths: Vec::new(),
            allow: default_skill_allow(),
            deny: Vec::new(),
        }
    }
}

fn default_skill_allow() -> Vec<String> {
    vec!["*".to_string()]
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SettingSource {
    User,
    Project,
    System,
}

/// Global permission rules applied before tool/sandbox checks.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PermissionsConfig {
    #[serde(default)]
    pub mode: PermissionMode,
    #[serde(default)]
    pub rules: Vec<PermissionRule>,
}

/// Permission mode applied before callbacks.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    #[default]
    Default,
    AcceptEdits,
    BypassPermissions,
    Plan,
}

/// Single permission rule (tool, path, or command matching).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRule {
    pub action: PermissionAction,
    #[serde(default)]
    pub tool: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub command: Option<Vec<String>>,
    #[serde(default)]
    pub access: Option<PathAccess>,
}

/// Re-export protocol path access (used in permission rules).
pub use odyssey_rs_protocol::PathAccess;
/// Re-export protocol permission action (used in permission rules).
pub use odyssey_rs_protocol::PermissionAction;

/// Global MCP client configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub servers: Vec<McpServerConfig>,
}

/// Configuration for a single MCP server connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    #[serde(default = "default_mcp_protocol")]
    pub protocol: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub sandbox: McpServerSandboxConfig,
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            protocol: default_mcp_protocol(),
            command: String::new(),
            args: Vec::new(),
            env: HashMap::new(),
            cwd: None,
            description: None,
            sandbox: McpServerSandboxConfig::default(),
        }
    }
}

fn default_mcp_protocol() -> String {
    "stdio".to_string()
}

/// Sandbox policy for a single MCP server process.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpServerSandboxConfig {
    #[serde(default)]
    pub filesystem: SandboxFilesystem,
    #[serde(default)]
    pub network: SandboxNetwork,
    #[serde(default)]
    pub env: SandboxEnv,
    #[serde(default)]
    pub limits: SandboxLimits,
}

/// Top-level sandbox configuration applied to all tools.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default = "default_sandbox_mode")]
    pub mode: SandboxMode,
    #[serde(default)]
    pub filesystem: SandboxFilesystem,
    #[serde(default)]
    pub network: SandboxNetwork,
    #[serde(default)]
    pub env: SandboxEnv,
    #[serde(default)]
    pub limits: SandboxLimits,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: None,
            mode: default_sandbox_mode(),
            filesystem: SandboxFilesystem::default(),
            network: SandboxNetwork::default(),
            env: SandboxEnv::default(),
            limits: SandboxLimits::default(),
        }
    }
}

/// Default sandbox mode for tool execution.
fn default_sandbox_mode() -> SandboxMode {
    SandboxMode::WorkspaceWrite
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SandboxFilesystem {
    #[serde(default)]
    pub read: Vec<String>,
    #[serde(default)]
    pub write: Vec<String>,
    #[serde(default)]
    pub exec: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SandboxNetworkMode {
    #[default]
    Disabled,
    AllowAll,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SandboxNetwork {
    #[serde(default)]
    pub mode: SandboxNetworkMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SandboxEnv {
    #[serde(default)]
    pub inherit: Vec<String>,
    #[serde(default)]
    pub set: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxLimits {
    #[serde(default)]
    pub cpu_seconds: Option<u64>,
    #[serde(default)]
    pub memory_bytes: Option<u64>,
    #[serde(default)]
    pub nofile: Option<u64>,
    #[serde(default)]
    pub pids: Option<u64>,
    #[serde(default = "default_sandbox_wall_clock_seconds")]
    pub wall_clock_seconds: Option<u64>,
    #[serde(default = "default_sandbox_stdout_bytes")]
    pub stdout_bytes: Option<usize>,
    #[serde(default = "default_sandbox_stderr_bytes")]
    pub stderr_bytes: Option<usize>,
}

impl Default for SandboxLimits {
    fn default() -> Self {
        Self {
            cpu_seconds: None,
            memory_bytes: None,
            nofile: None,
            pids: None,
            wall_clock_seconds: default_sandbox_wall_clock_seconds(),
            stdout_bytes: default_sandbox_stdout_bytes(),
            stderr_bytes: default_sandbox_stderr_bytes(),
        }
    }
}

fn default_sandbox_wall_clock_seconds() -> Option<u64> {
    Some(60)
}

fn default_sandbox_stdout_bytes() -> Option<usize> {
    Some(64 * 1024)
}

fn default_sandbox_stderr_bytes() -> Option<usize> {
    Some(64 * 1024)
}

/// Session persistence settings.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionsConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub path: Option<String>,
}
