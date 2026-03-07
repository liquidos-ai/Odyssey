//! Host execution provider with policy enforcement but no kernel isolation.

use crate::{
    SandboxError,
    provider::{
        BufferingSink, PreparedSandbox, build_host_child_command, build_prepared_sandbox,
        run_host_process,
    },
};
use log::{debug, info, warn};
use std::{collections::HashMap, path::Path};

use async_trait::async_trait;

use crate::{
    AccessDecision, AccessMode, CommandOutputSink, CommandResult, CommandSpec, SandboxContext,
    SandboxHandle, SandboxProvider,
};

/// Provider that executes on the host with path/env/limit enforcement.
#[derive(Debug, Default)]
pub struct HostExecProvider {
    /// Prepared sandbox state keyed by handle id.
    state: parking_lot::RwLock<HashMap<uuid::Uuid, PreparedSandbox>>,
}

/// Backwards-compatible alias used across the existing runtime.
pub type LocalSandboxProvider = HostExecProvider;

impl HostExecProvider {
    /// Create a new host execution provider.
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl SandboxProvider for HostExecProvider {
    async fn prepare(&self, ctx: &SandboxContext) -> Result<SandboxHandle, SandboxError> {
        let prepared = build_prepared_sandbox(ctx)?;
        let handle = SandboxHandle {
            id: uuid::Uuid::new_v4(),
        };
        self.state.write().insert(handle.id, prepared);
        warn!(
            "host execution provider prepared (handle_id={}); this backend does not provide kernel isolation",
            handle.id
        );
        Ok(handle)
    }

    async fn run_command(
        &self,
        handle: &SandboxHandle,
        spec: CommandSpec,
    ) -> Result<CommandResult, SandboxError> {
        let mut sink = BufferingSink::default();
        let result = self.run_command_streaming(handle, spec, &mut sink).await?;
        Ok(CommandResult {
            status_code: result.status_code,
            stdout: sink.stdout,
            stderr: sink.stderr,
            stdout_truncated: result.stdout_truncated,
            stderr_truncated: result.stderr_truncated,
        })
    }

    async fn run_command_streaming(
        &self,
        handle: &SandboxHandle,
        spec: CommandSpec,
        sink: &mut dyn CommandOutputSink,
    ) -> Result<CommandResult, SandboxError> {
        debug!("host execution run (handle_id={})", handle.id);
        let prepared = self
            .state
            .read()
            .get(&handle.id)
            .cloned()
            .ok_or_else(|| SandboxError::InvalidConfig("unknown sandbox handle".to_string()))?;
        run_host_process(spec, &prepared, sink).await
    }

    fn check_access(
        &self,
        handle: &SandboxHandle,
        path: &Path,
        mode: AccessMode,
    ) -> AccessDecision {
        let state = self.state.read();
        let Some(prepared) = state.get(&handle.id) else {
            return AccessDecision::Deny("unknown sandbox handle".to_string());
        };
        prepared.access.check(path, mode)
    }

    fn spawn_command(
        &self,
        handle: &SandboxHandle,
        spec: CommandSpec,
    ) -> Result<tokio::process::Command, SandboxError> {
        let prepared = self
            .state
            .read()
            .get(&handle.id)
            .cloned()
            .ok_or_else(|| SandboxError::InvalidConfig("unknown sandbox handle".to_string()))?;
        build_host_child_command(spec, &prepared)
    }

    async fn shutdown(&self, handle: SandboxHandle) {
        info!("host execution provider shutdown (handle_id={})", handle.id);
        self.state.write().remove(&handle.id);
    }
}

#[cfg(test)]
mod tests {
    use super::HostExecProvider;
    use crate::provider::SandboxProvider;
    use crate::{AccessDecision, AccessMode, CommandSpec, SandboxContext, SandboxPolicy};
    use odyssey_rs_protocol::SandboxMode;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    #[tokio::test]
    async fn host_provider_runs_commands() {
        let workspace = tempdir().expect("workspace");
        let provider = HostExecProvider::new();
        let ctx = SandboxContext {
            workspace_root: workspace.path().to_path_buf(),
            mode: SandboxMode::WorkspaceWrite,
            policy: SandboxPolicy::default(),
        };
        let handle = provider.prepare(&ctx).await.expect("prepare");

        let mut spec = CommandSpec::new("sh");
        spec.args
            .extend(["-c".to_string(), "printf 'hello'".to_string()]);

        let result = provider.run_command(&handle, spec).await.expect("run");
        assert_eq!(result.stdout, "hello");
        assert_eq!(result.status_code, Some(0));
    }

    #[tokio::test]
    async fn host_provider_check_access_and_shutdown() {
        let workspace = tempdir().expect("workspace");
        let provider = HostExecProvider::new();
        let ctx = SandboxContext {
            workspace_root: workspace.path().to_path_buf(),
            mode: SandboxMode::WorkspaceWrite,
            policy: SandboxPolicy::default(),
        };
        let handle = provider.prepare(&ctx).await.expect("prepare");
        let handle_clone = handle.clone();

        let inside = workspace.path().join("file.txt");
        assert_eq!(
            provider.check_access(&handle, &inside, AccessMode::Read),
            AccessDecision::Allow
        );

        let outside = tempdir().expect("outside");
        match provider.check_access(&handle, outside.path(), AccessMode::Write) {
            AccessDecision::Deny(message) => assert_eq!(message.is_empty(), false),
            other => panic!("unexpected decision: {other:?}"),
        }

        provider.shutdown(handle).await;
        match provider.check_access(&handle_clone, &inside, AccessMode::Read) {
            AccessDecision::Deny(message) => assert_eq!(message.contains("unknown"), true),
            other => panic!("unexpected decision: {other:?}"),
        }
    }
}
