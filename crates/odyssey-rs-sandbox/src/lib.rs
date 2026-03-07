//! Sandbox provider interfaces and implementations.

pub mod error;
pub mod provider;
pub mod runner;
pub mod runtime;
pub mod types;

/// Sandbox error type.
pub use error::SandboxError;
/// Provider traits and helpers.
pub use provider::{
    CommandOutputSink, DependencyReport, SandboxProvider, local::HostExecProvider,
    local::LocalSandboxProvider, resolve_internal_landlock_helper_path,
};
/// Standalone runner API.
pub use runner::SandboxRunner;
/// Persistent runtime and reusable cell APIs.
pub use runtime::{
    SandboxCellKey, SandboxCellKind, SandboxCellLease, SandboxCellRoot, SandboxCellSpec,
    SandboxExecutionLayout, SandboxRuntime,
};
/// Core sandbox types and policies.
pub use types::{
    AccessDecision, AccessMode, CommandLandlockPolicy, CommandResult, CommandSpec, SandboxContext,
    SandboxEnvPolicy, SandboxFilesystemPolicy, SandboxHandle, SandboxLimits, SandboxNetworkMode,
    SandboxNetworkPolicy, SandboxPolicy, SandboxRunRequest, SandboxRunResult, SandboxSupport,
};

/// Default sandbox provider name for a given mode and platform.
pub fn default_provider_name(mode: odyssey_rs_protocol::SandboxMode) -> &'static str {
    if mode == odyssey_rs_protocol::SandboxMode::DangerFullAccess {
        return "host";
    }
    #[cfg(target_os = "linux")]
    {
        "bubblewrap"
    }
    #[cfg(not(target_os = "linux"))]
    {
        "unsupported"
    }
}

#[cfg(target_os = "linux")]
/// Bubblewrap provider for Linux.
pub use provider::linux::BubblewrapProvider;
