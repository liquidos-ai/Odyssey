pub mod error;
pub mod provider;
pub mod runner;
pub mod runtime;
pub mod types;

pub use error::SandboxError;
pub use odyssey_rs_protocol::SandboxMode;
pub use provider::{
    CommandOutputSink, DependencyReport, SandboxProvider, local::HostExecProvider,
    local::LocalSandboxProvider, standard_system_exec_roots,
};
pub use runner::SandboxRunner;
pub use runtime::{
    SandboxCellKey, SandboxCellKind, SandboxCellLease, SandboxCellRoot, SandboxCellSpec,
    SandboxExecutionLayout, SandboxRuntime,
};
pub use types::{
    AccessDecision, AccessMode, CommandResult, CommandSpec, SandboxContext, SandboxEnvPolicy,
    SandboxFilesystemPolicy, SandboxHandle, SandboxLimits, SandboxNetworkMode,
    SandboxNetworkPolicy, SandboxPolicy, SandboxRunRequest, SandboxRunResult, SandboxSupport,
};

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
        "host"
    }
}

#[cfg(target_os = "linux")]
pub use provider::linux::BubblewrapProvider;

#[cfg(test)]
mod tests {
    use super::default_provider_name;
    use odyssey_rs_protocol::SandboxMode;
    use pretty_assertions::assert_eq;

    #[test]
    fn danger_full_access_defaults_to_host() {
        assert_eq!(default_provider_name(SandboxMode::DangerFullAccess), "host");
    }

    #[test]
    fn workspace_write_uses_platform_default_provider() {
        #[cfg(target_os = "linux")]
        assert_eq!(
            default_provider_name(SandboxMode::WorkspaceWrite),
            "bubblewrap"
        );

        #[cfg(not(target_os = "linux"))]
        assert_eq!(default_provider_name(SandboxMode::WorkspaceWrite), "host");
    }
}
