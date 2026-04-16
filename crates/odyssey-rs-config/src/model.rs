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
    pub orchestrator: OrchestratorConfig,
    #[serde(default)]
    pub tools: ToolsConfig,
    #[serde(default)]
    pub permissions: PermissionsConfig,
    #[serde(default)]
    pub memory: MemoryConfig,
    #[serde(default)]
    pub skills: SkillsConfig,
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

    /// Replace the global permissions configuration.
    pub fn permissions(mut self, permissions: PermissionsConfig) -> Self {
        self.config.permissions = permissions;
        self
    }

    /// Replace the orchestrator configuration.
    pub fn orchestrator(mut self, orchestrator: OrchestratorConfig) -> Self {
        self.config.orchestrator = orchestrator;
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

/// Configuration for the built-in Odyssey orchestrator agent.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OrchestratorConfig {
    #[serde(default)]
    pub additional_instruction_prompt: Option<String>,
    #[serde(default = "default_subagent_window_size")]
    pub subagent_window_size: usize,
}

fn default_subagent_window_size() -> usize {
    20
}

/// Model provider configuration for an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub provider: String,
    pub name: String,
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
    #[serde(default = "default_memory_provider")]
    pub provider: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default = "default_recall_k")]
    pub recall_k: usize,
    #[serde(default)]
    pub capture: MemoryCapturePolicy,
    #[serde(default)]
    pub recall: MemoryRecallConfig,
    #[serde(default)]
    pub compaction: MemoryCompactionPolicy,
    #[serde(default)]
    pub instruction_roots: Vec<String>,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            provider: default_memory_provider(),
            path: None,
            recall_k: default_recall_k(),
            capture: MemoryCapturePolicy::default(),
            recall: MemoryRecallConfig::default(),
            compaction: MemoryCompactionPolicy::default(),
            instruction_roots: Vec::new(),
        }
    }
}

/// Default memory provider identifier.
fn default_memory_provider() -> String {
    "file".to_string()
}

/// Default number of memory items to recall.
fn default_recall_k() -> usize {
    6
}

/// Capture policy used by memory providers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryCapturePolicy {
    #[serde(default = "default_capture_messages")]
    pub capture_messages: bool,
    #[serde(default)]
    pub capture_tool_output: bool,
    #[serde(default)]
    pub deny_patterns: Vec<String>,
    #[serde(default)]
    pub redact_patterns: Vec<String>,
    #[serde(default)]
    pub max_message_chars: Option<usize>,
    #[serde(default = "default_detect_secrets")]
    pub detect_secrets: bool,
    #[serde(default = "default_secret_entropy_threshold")]
    pub secret_entropy_threshold: f32,
    #[serde(default)]
    pub max_tool_output_chars: Option<usize>,
}

impl Default for MemoryCapturePolicy {
    fn default() -> Self {
        Self {
            capture_messages: default_capture_messages(),
            capture_tool_output: false,
            deny_patterns: Vec::new(),
            redact_patterns: Vec::new(),
            max_message_chars: None,
            detect_secrets: default_detect_secrets(),
            secret_entropy_threshold: default_secret_entropy_threshold(),
            max_tool_output_chars: None,
        }
    }
}

/// Default toggle for capturing user/assistant messages.
fn default_capture_messages() -> bool {
    true
}

/// Default toggle for secret detection in memory capture.
fn default_detect_secrets() -> bool {
    true
}

/// Default entropy threshold for identifying secrets.
fn default_secret_entropy_threshold() -> f32 {
    3.7
}

/// Recall scoring configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryRecallConfig {
    #[serde(default)]
    pub mode: MemoryRecallMode,
    #[serde(default = "default_text_weight")]
    pub text_weight: f32,
    #[serde(default = "default_vector_weight")]
    pub vector_weight: f32,
    #[serde(default)]
    pub min_score: Option<f32>,
}

impl Default for MemoryRecallConfig {
    fn default() -> Self {
        Self {
            mode: MemoryRecallMode::default(),
            text_weight: default_text_weight(),
            vector_weight: default_vector_weight(),
            min_score: None,
        }
    }
}

/// Default text similarity weight for recall scoring.
fn default_text_weight() -> f32 {
    0.3
}

/// Default vector similarity weight for recall scoring.
fn default_vector_weight() -> f32 {
    0.7
}

/// Recall mode selection for memory search.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum MemoryRecallMode {
    #[default]
    Text,
    Vector,
    Hybrid,
}

/// Compaction policy for long sessions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryCompactionPolicy {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_compaction_max_messages")]
    pub max_messages: usize,
    #[serde(default = "default_compaction_summary_chars")]
    pub summary_max_chars: usize,
    #[serde(default)]
    pub max_total_chars: Option<usize>,
}

impl Default for MemoryCompactionPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            max_messages: default_compaction_max_messages(),
            summary_max_chars: default_compaction_summary_chars(),
            max_total_chars: None,
        }
    }
}

/// Default maximum message count before compaction.
fn default_compaction_max_messages() -> usize {
    40
}

/// Default maximum summary length during compaction.
fn default_compaction_summary_chars() -> usize {
    1500
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
    pub allow_read: Vec<String>,
    #[serde(default)]
    pub deny_read: Vec<String>,
    #[serde(default)]
    pub allow_write: Vec<String>,
    #[serde(default)]
    pub deny_write: Vec<String>,
    #[serde(default)]
    pub allow_exec: Vec<String>,
    #[serde(default)]
    pub deny_exec: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SandboxNetwork {
    #[serde(default)]
    pub allow_domains: Vec<String>,
    #[serde(default)]
    pub deny_domains: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SandboxEnv {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
    #[serde(default)]
    pub set: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SandboxLimits {
    #[serde(default)]
    pub cpu_seconds: Option<u64>,
    #[serde(default)]
    pub memory_bytes: Option<u64>,
    #[serde(default)]
    pub nofile: Option<u64>,
    #[serde(default)]
    pub pids: Option<u64>,
}

/// Session persistence settings.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionsConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub path: Option<String>,
}
