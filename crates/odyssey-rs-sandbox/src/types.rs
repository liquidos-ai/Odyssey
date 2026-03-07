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
    /// Canonical or canonicalizable workspace root path.
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

/// Filesystem allow roots.
#[derive(Debug, Clone, Default)]
pub struct SandboxFilesystemPolicy {
    /// Additional read-only roots.
    pub read_roots: Vec<String>,
    /// Additional writable roots.
    pub write_roots: Vec<String>,
    /// Additional executable roots.
    pub exec_roots: Vec<String>,
}

/// Environment variable policy settings.
#[derive(Debug, Clone, Default)]
pub struct SandboxEnvPolicy {
    /// Host variables allowed to be inherited.
    pub inherit: Vec<String>,
    /// Environment variables to set explicitly.
    pub set: BTreeMap<String, String>,
}

/// Network access policy settings.
#[derive(Debug, Clone)]
pub struct SandboxNetworkPolicy {
    /// Network mode for the sandbox.
    pub mode: SandboxNetworkMode,
}

impl Default for SandboxNetworkPolicy {
    fn default() -> Self {
        Self {
            mode: SandboxNetworkMode::AllowAll,
        }
    }
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
    /// Wall clock timeout in seconds.
    pub wall_clock_seconds: Option<u64>,
    /// Maximum captured stdout bytes.
    pub stdout_bytes: Option<usize>,
    /// Maximum captured stderr bytes.
    pub stderr_bytes: Option<usize>,
}

/// Network access mode for sandbox providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxNetworkMode {
    /// Allow unrestricted outbound networking.
    AllowAll,
    /// Disable networking entirely.
    Disabled,
}

/// Additional Landlock filesystem restrictions for a sandbox command.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandLandlockPolicy {
    /// Read-only roots visible to the command.
    pub read_roots: Vec<PathBuf>,
    /// Writable roots visible to the command.
    pub write_roots: Vec<PathBuf>,
    /// Executable roots visible to the command.
    pub exec_roots: Vec<PathBuf>,
}

/// Command specification for sandbox execution.
#[derive(Debug, Clone)]
pub struct CommandSpec {
    /// Command path or executable name.
    pub command: PathBuf,
    /// Command arguments.
    pub args: Vec<String>,
    /// Optional working directory.
    pub cwd: Option<PathBuf>,
    /// Environment variables for the command.
    pub env: BTreeMap<String, String>,
    /// Optional Landlock policy applied immediately before `exec` via the internal helper binary.
    pub landlock: Option<CommandLandlockPolicy>,
}

impl CommandSpec {
    /// Create a new command spec with defaults.
    pub fn new(command: impl Into<PathBuf>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
            landlock: None,
        }
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
    /// Whether stdout had to be truncated.
    pub stdout_truncated: bool,
    /// Whether stderr had to be truncated.
    pub stderr_truncated: bool,
}

/// Standalone execution request.
#[derive(Debug, Clone)]
pub struct SandboxRunRequest {
    /// Sandbox context for the run.
    pub context: SandboxContext,
    /// Command to execute.
    pub command: CommandSpec,
}

/// Standalone execution result.
pub type SandboxRunResult = CommandResult;

/// Provider support report suitable for standalone tooling.
#[derive(Debug, Clone, Default)]
pub struct SandboxSupport {
    /// Provider display name.
    pub provider: String,
    /// Whether the provider is usable in the current environment.
    pub available: bool,
    /// Hard errors that prevent provider use.
    pub errors: Vec<String>,
    /// Non-fatal warnings.
    pub warnings: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::{CommandLandlockPolicy, CommandSpec, SandboxNetworkMode, SandboxNetworkPolicy};
    use pretty_assertions::assert_eq;
    use std::path::PathBuf;

    #[test]
    fn command_spec_defaults_are_empty() {
        let spec = CommandSpec::new("echo");
        assert_eq!(spec.command, PathBuf::from("echo"));
        assert_eq!(spec.args.len(), 0);
        assert_eq!(spec.cwd, None);
        assert_eq!(spec.env.len(), 0);
        assert_eq!(spec.landlock, None);
    }

    #[test]
    fn landlock_policy_defaults_are_empty() {
        let policy = CommandLandlockPolicy::default();
        assert_eq!(policy.read_roots.len(), 0);
        assert_eq!(policy.write_roots.len(), 0);
        assert_eq!(policy.exec_roots.len(), 0);
    }

    #[test]
    fn network_policy_defaults_to_allow_all() {
        let policy = SandboxNetworkPolicy::default();
        assert_eq!(policy.mode, SandboxNetworkMode::AllowAll);
    }
}
