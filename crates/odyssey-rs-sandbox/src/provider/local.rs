//! Local (non-isolated) sandbox provider implementation.

use crate::{
    SandboxError,
    provider::{BufferingSink, PreparedSandbox, build_prepared_sandbox, run_local_process},
};
use log::{debug, info};
use std::{collections::HashMap, path::Path};

use async_trait::async_trait;

use crate::{
    AccessDecision, AccessMode, CommandOutputSink, CommandResult, CommandSpec, SandboxContext,
    SandboxHandle, SandboxProvider,
};

/// Sandbox provider that runs commands on the host with policy checks.
#[derive(Debug, Default)]
pub struct LocalSandboxProvider {
    /// Prepared sandbox state keyed by handle id.
    state: parking_lot::RwLock<HashMap<uuid::Uuid, PreparedSandbox>>,
}

impl LocalSandboxProvider {
    /// Create a new local sandbox provider.
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl SandboxProvider for LocalSandboxProvider {
    /// Prepare sandbox state for a session.
    async fn prepare(&self, ctx: &SandboxContext) -> Result<SandboxHandle, SandboxError> {
        let prepared = build_prepared_sandbox(ctx)?;
        let handle = SandboxHandle {
            id: uuid::Uuid::new_v4(),
        };
        self.state.write().insert(handle.id, prepared);
        info!("local sandbox prepared (handle_id={})", handle.id);
        Ok(handle)
    }

    /// Run a command without streaming output.
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
        })
    }

    /// Run a command with streaming output callbacks.
    async fn run_command_streaming(
        &self,
        handle: &SandboxHandle,
        spec: CommandSpec,
        sink: &mut dyn CommandOutputSink,
    ) -> Result<CommandResult, SandboxError> {
        debug!("local sandbox run (handle_id={})", handle.id);
        let prepared = self
            .state
            .read()
            .get(&handle.id)
            .cloned()
            .ok_or_else(|| SandboxError::InvalidConfig("unknown sandbox handle".to_string()))?;
        run_local_process(spec, &prepared, sink).await
    }

    /// Check filesystem access in the prepared sandbox.
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

    /// Shutdown and remove sandbox state.
    async fn shutdown(&self, handle: SandboxHandle) {
        info!("local sandbox shutdown (handle_id={})", handle.id);
        self.state.write().remove(&handle.id);
    }
}

#[cfg(test)]
mod tests {
    use super::LocalSandboxProvider;
    use crate::provider::SandboxProvider;
    use crate::{AccessDecision, AccessMode, CommandSpec, SandboxContext, SandboxPolicy};
    use odyssey_rs_protocol::SandboxMode;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    #[tokio::test]
    async fn local_provider_runs_commands() {
        let workspace = tempdir().expect("workspace");
        let provider = LocalSandboxProvider::new();
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
    async fn local_provider_check_access_and_shutdown() {
        let workspace = tempdir().expect("workspace");
        let provider = LocalSandboxProvider::new();
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
        match provider.check_access(&handle, outside.path(), AccessMode::Read) {
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
