use odyssey_rs_protocol::SandboxMode;
use std::collections::BTreeMap;
use std::path::PathBuf;
use uuid::Uuid;

/// Filesystem operation being authorized by a sandbox provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessMode {
    Read,
    Write,
    Execute,
}

/// Result returned by sandbox access checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessDecision {
    Allow,
    Deny(String),
}

/// Input used to prepare a provider-specific sandbox handle.
#[derive(Debug, Clone)]
pub struct SandboxContext {
    pub workspace_root: PathBuf,
    pub mode: SandboxMode,
    pub policy: SandboxPolicy,
}

/// Opaque identifier for a prepared sandbox instance.
#[derive(Debug, Clone)]
pub struct SandboxHandle {
    pub id: Uuid,
}

/// Complete sandbox policy applied to a command execution.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SandboxPolicy {
    pub filesystem: SandboxFilesystemPolicy,
    pub env: SandboxEnvPolicy,
    pub network: SandboxNetworkPolicy,
    pub limits: SandboxLimits,
}

/// Filesystem allowlists and bind mounts exposed to the sandbox.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SandboxFilesystemPolicy {
    pub read_roots: Vec<String>,
    pub write_roots: Vec<String>,
    pub exec_roots: Vec<String>,
    pub exec_allow_all: bool,
    pub mount_bindings: Vec<SandboxMountBinding>,
}

/// Explicit host-to-sandbox bind mount.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxMountBinding {
    pub source: String,
    pub target: String,
    pub writable: bool,
}

/// Environment variables inherited or injected into the sandbox.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SandboxEnvPolicy {
    pub inherit: Vec<String>,
    pub set: BTreeMap<String, String>,
}

/// Network settings enforced by the active sandbox backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxNetworkPolicy {
    pub mode: SandboxNetworkMode,
}

impl Default for SandboxNetworkPolicy {
    fn default() -> Self {
        Self {
            mode: SandboxNetworkMode::AllowAll,
        }
    }
}

/// Resource limits enforced for a single command invocation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SandboxLimits {
    pub cpu_seconds: Option<u64>,
    pub memory_bytes: Option<u64>,
    pub nofile: Option<u64>,
    pub pids: Option<u64>,
    pub wall_clock_seconds: Option<u64>,
    pub stdout_bytes: Option<usize>,
    pub stderr_bytes: Option<usize>,
}

/// Simplified network policy supported by the current runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxNetworkMode {
    AllowAll,
    Disabled,
}

/// Process specification executed inside the sandbox.
#[derive(Debug, Clone)]
pub struct CommandSpec {
    pub command: PathBuf,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: BTreeMap<String, String>,
}

impl CommandSpec {
    pub fn new(command: impl Into<PathBuf>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
        }
    }
}

/// Buffered output collected from a process execution.
#[derive(Debug, Clone, Default)]
pub struct CommandResult {
    pub status_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

/// Wrapper passed to [`crate::SandboxRunner`] and provider implementations.
#[derive(Debug, Clone)]
pub struct SandboxRunRequest {
    pub context: SandboxContext,
    pub command: CommandSpec,
}

pub type SandboxRunResult = CommandResult;

/// Availability report for a sandbox backend on the current machine.
#[derive(Debug, Clone, Default)]
pub struct SandboxSupport {
    pub provider: String,
    pub available: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::{CommandSpec, SandboxNetworkMode, SandboxNetworkPolicy};
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

    #[test]
    fn network_policy_defaults_to_allow_all() {
        let policy = SandboxNetworkPolicy::default();
        assert_eq!(policy.mode, SandboxNetworkMode::AllowAll);
    }
}
