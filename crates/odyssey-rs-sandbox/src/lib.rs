//! Sandbox provider interfaces and implementations.

pub mod error;
pub mod provider;
pub mod types;

/// Sandbox error type.
pub use error::SandboxError;
/// Provider traits and helpers.
pub use provider::{
    CommandOutputSink, DependencyReport, SandboxProvider, local::LocalSandboxProvider,
};
/// Core sandbox types and policies.
pub use types::{
    AccessDecision, AccessMode, CommandResult, CommandSpec, SandboxContext, SandboxEnvPolicy,
    SandboxFilesystemPolicy, SandboxHandle, SandboxLimits, SandboxNetworkMode,
    SandboxNetworkPolicy, SandboxPolicy,
};

/// Default sandbox provider name for a given mode and platform.
pub fn default_provider_name(mode: odyssey_rs_protocol::SandboxMode) -> &'static str {
    if mode == odyssey_rs_protocol::SandboxMode::DangerFullAccess {
        return "local";
    }
    #[cfg(target_os = "linux")]
    {
        "bubblewrap"
    }
    #[cfg(not(target_os = "linux"))]
    {
        "none"
    }
}

#[cfg(target_os = "linux")]
/// Bubblewrap provider for Linux.
pub use provider::linux::BubblewrapProvider;
