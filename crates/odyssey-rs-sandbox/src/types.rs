//! Sandbox policy and command execution types.

use odyssey_rs_protocol::SandboxMode;
use std::collections::BTreeMap;
use std::path::PathBuf;
use uuid::Uuid;

/// Access mode for sandbox checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessMode {
    /// Read access.
    Read,
    /// Write access.
    Write,
    /// Execute access.
    Execute,
}

/// Result of a sandbox access check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessDecision {
    /// Allow access.
    Allow,
    /// Deny access with reason.
    Deny(String),
}

/// Context used when preparing a sandbox.
#[derive(Debug, Clone)]
pub struct SandboxContext {
    /// Workspace root path.
    pub workspace_root: PathBuf,
    /// Sandbox mode for this execution.
    pub mode: SandboxMode,
    /// Policy applied to the sandbox.
    pub policy: SandboxPolicy,
}

/// Handle returned by sandbox providers.
#[derive(Debug, Clone)]
pub struct SandboxHandle {
    /// Unique handle id.
    pub id: Uuid,
}

/// Aggregated sandbox policy settings.
#[derive(Debug, Clone, Default)]
pub struct SandboxPolicy {
    /// Filesystem access policy.
    pub filesystem: SandboxFilesystemPolicy,
    /// Environment variable policy.
    pub env: SandboxEnvPolicy,
    /// Network access policy.
    pub network: SandboxNetworkPolicy,
    /// Resource limits.
    pub limits: SandboxLimits,
}

/// Filesystem allow/deny lists.
#[derive(Debug, Clone, Default)]
pub struct SandboxFilesystemPolicy {
    /// Allowed read paths.
    pub allow_read: Vec<String>,
    /// Denied read paths.
    pub deny_read: Vec<String>,
    /// Allowed write paths.
    pub allow_write: Vec<String>,
    /// Denied write paths.
    pub deny_write: Vec<String>,
    /// Allowed executable paths.
    pub allow_exec: Vec<String>,
    /// Denied executable paths.
    pub deny_exec: Vec<String>,
}

/// Environment variable policy settings.
#[derive(Debug, Clone, Default)]
pub struct SandboxEnvPolicy {
    /// Allowed environment variables.
    pub allow: Vec<String>,
    /// Denied environment variables.
    pub deny: Vec<String>,
    /// Environment variables to set.
    pub set: BTreeMap<String, String>,
}

/// Network access policy settings.
#[derive(Debug, Clone, Default)]
pub struct SandboxNetworkPolicy {
    /// Allowed domains.
    pub allow_domains: Vec<String>,
    /// Denied domains.
    pub deny_domains: Vec<String>,
}

/// Resource limits for sandboxed commands.
#[derive(Debug, Clone, Default)]
pub struct SandboxLimits {
    /// CPU seconds limit.
    pub cpu_seconds: Option<u64>,
    /// Memory limit in bytes.
    pub memory_bytes: Option<u64>,
    /// File descriptor limit.
    pub nofile: Option<u64>,
    /// Process count limit.
    pub pids: Option<u64>,
}

/// Network access mode for sandbox providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxNetworkMode {
    /// Allow network access.
    Allow,
    /// Deny network access.
    Deny,
}

/// Command specification for sandbox execution.
#[derive(Debug, Clone)]
pub struct CommandSpec {
    /// Command path.
    pub command: PathBuf,
    /// Command arguments.
    pub args: Vec<String>,
    /// Optional working directory.
    pub cwd: Option<PathBuf>,
    /// Environment variables for the command.
    pub env: BTreeMap<String, String>,
}

impl CommandSpec {
    /// Create a new command spec with defaults.
    pub fn new(command: impl Into<PathBuf>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CommandSpec;
    use pretty_assertions::assert_eq;
    use std::path::PathBuf;

    #[test]
    fn command_spec_defaults_are_empty() {
        let spec = CommandSpec::new("echo");
        assert_eq!(spec.command, PathBuf::from("echo"));
        assert_eq!(spec.args.len(), 0);
        assert_eq!(spec.cwd, None);
        assert_eq!(spec.env.len(), 0);
    }
}

/// Result of a sandboxed command execution.
#[derive(Debug, Clone, Default)]
pub struct CommandResult {
    /// Exit status code if available.
    pub status_code: Option<i32>,
    /// Captured stdout content.
    pub stdout: String,
    /// Captured stderr content.
    pub stderr: String,
}
